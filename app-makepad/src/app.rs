//! Aplicativo RuanaStudio em Makepad — fase 1 (shell + transport + timeline).
//!
//! - Janela Makepad: barra de transport (play/pause, stop, BPM, relógio).
//! - Timeline real: `timeline-render` numa janela winit/wgpu borderless
//!   ENCAIXADA na janela Makepad (rastreia a janela mãe e cobre a área da
//!   timeline), com o `Transport` compartilhado (Arc<Mutex>) e clips
//!   espelhados via `WindowCommand::SetClips/SetBpm` (read-only nesta fase).
//! - Estado autoritativo fora da UI: `Project` + `Transport` + `AudioEngine`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use makepad_widgets::*;

use daw_core::transport::Transport;
use daw_core::Project;
use timeline_render::clips::ClipDemo;
use timeline_render::{DockSpec, WindowCommand};

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: { inner_size: vec2(1100, 640), title: "RuanaStudio" }
                pass: { clear_color: #191919 }

                body = <View> {
                    width: Fill, height: Fill,
                    flow: Down,
                    spacing: 0,

                    transport_bar = <View> {
                        // Altura do cabeçalho: 46 px lógicos — a timeline
                        // encaixada começa exatamente abaixo desta barra.
                        width: Fill, height: 46,
                        flow: Right,
                        spacing: 10,
                        align: { y: 0.5 },
                        padding: { left: 12, right: 12 },

                        title_label = <Label> {
                            text: "RuanaStudio",
                            draw_text: {
                                color: #8fc7ff,
                                text_style: { font_size: 18 },
                            },
                        }

                        <View> { width: Fill, height: Fill }

                        bpm_label = <Label> {
                            text: "120 BPM",
                            draw_text: {
                                color: #ccc,
                                text_style: { font_size: 14 },
                            },
                        }

                        time_label = <Label> {
                            text: "0:00.000",
                            draw_text: {
                                color: #fff,
                                text_style: { font_size: 18 },
                            },
                        }

                        play_btn = <Button> {
                            text: "Play",
                            width: 88,
                        }

                        stop_btn = <Button> {
                            text: "Stop",
                            width: 88,
                        }
                    }

                    timeline_area = <View> {
                        // A janela nativa da timeline cobre esta área com
                        // margens (10px laterais) e reserva o rodapé abaixo.
                        width: Fill, height: Fill,
                        show_bg: true,
                        draw_bg: { color: #101014 },
                    }

                    status_bar = <View> {
                        width: Fill, height: 22,
                        align: { x: 0.5, y: 0.5 },
                        status_label = <Label> {
                            text: "RuanaStudio · timeline wgpu encaixada",
                            draw_text: {
                                color: #555,
                                text_style: { font_size: 11 },
                            },
                        }
                    }
                }
            }
        }
    }
}

app_main!(App);

/// Estado do shell (fora do `#[live]`, vive no App e sobrevive à UI).
/// `Default` é inerte; o wiring real acontece em `ShellState::build` (Startup).
struct ShellState {
    transport: Arc<Mutex<Transport>>,
    project: Rc<RefCell<Project>>,
    _audio_engine: Option<daw_audio::AudioEngine>,
    timeline_tx: Option<mpsc::Sender<WindowCommand>>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            transport: Arc::new(Mutex::new(Transport::default())),
            project: Rc::new(RefCell::new(Project::startup())),
            _audio_engine: None,
            timeline_tx: None,
        }
    }
}

impl ShellState {
    /// Sobe o estado autoritativo + timeline nativa. Chamado no Startup.
    fn build() -> Self {
        let transport = Arc::new(Mutex::new(Transport::default()));
        {
            let mut t = transport.lock().unwrap();
            t.bpm = 120;
            t.ppq = 960;
        }
        let project = Rc::new(RefCell::new(Project::startup()));

        // Motor de áudio: avança o playhead do Transport compartilhado mesmo
        // sem clips (a timeline lê `position_seconds` a cada frame). Se o
        // dispositivo não abrir, o shell segue em silêncio.
        let _audio_engine = match daw_audio::AudioEngine::start(transport.clone()) {
            Ok(engine) => {
                let cfg = engine.output_config();
                eprintln!("[audio] {} Hz, {} canais", cfg.sample_rate, cfg.channels);
                Some(engine)
            }
            Err(err) => {
                eprintln!("[audio] dispositivo indisponível: {err} — em silêncio");
                None
            }
        };

        // Timeline encaixada na janela Makepad: janela winit/wgpu borderless
        // que rastreia a janela "RuanaStudio" e fica como painel com margens
        // abaixo da barra de transport (46) e acima do rodapé (22+12=34).
        // Render direto na surface; o Transport compartilhado faz o playhead
        // seguir o play.
        let (tx, rx) = mpsc::channel::<WindowCommand>();
        let transport_thread = transport.clone();
        let spec = DockSpec {
            parent_title: "RuanaStudio".to_string(),
            top: 46,
            left: 10,
            right: 10,
            bottom: 34,
        };
        std::thread::Builder::new()
            .name("timeline-window".into())
            .spawn(move || {
                let _ = timeline_render::run_docked_window(transport_thread, rx, spec);
            })
            .map(|_| eprintln!("[timeline] janela encaixada iniciada"))
            .unwrap_or_else(|e| eprintln!("[timeline] falha ao abrir janela: {e}"));

        // Espelho inicial: clips demo read-only + BPM (a engine regenera as
        // barras de waveform procedurais ao receber).
        let mut clips = demo_clips();
        timeline_render::waveform::build_wave_bars_and_segments(&mut clips, 8.0);
        let _ = tx.send(WindowCommand::SetClips(clips));
        let _ = tx.send(WindowCommand::SetBpm(120.0));

        Self {
            transport,
            project,
            _audio_engine,
            timeline_tx: Some(tx),
        }
    }

    /// Espelha o estado do transport no painel (relógio, play/pause).
    fn refresh_status(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        let (playing, pos, bpm) = {
            let t = self.transport.lock().unwrap();
            (t.playing, t.position_seconds() as f32, t.bpm)
        };
        ui.label(id!(time_label))
            .set_text(cx, &format!("{}  {}", if playing { "▶" } else { "" }, fmt_time(pos)));
        ui.label(id!(bpm_label)).set_text(cx, &format!("{bpm} BPM"));
        ui.button(id!(play_btn))
            .set_text(cx, if playing { "Pause" } else { "Play" });
        ui.redraw(cx);
    }

    fn toggle_play(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        let mut t = self.transport.lock().unwrap();
        t.playing = !t.playing;
        drop(t);
        self.refresh_status(ui, cx);
    }

    fn stop(&mut self, ui: &WidgetRef, cx: &mut Cx) {
        let mut t = self.transport.lock().unwrap();
        t.playing = false;
        t.position_ticks = 0;
        drop(t);
        self.refresh_status(ui, cx);
    }
}

/// Converte segundos → "m:ss.mmm" (estilo DAW).
fn fmt_time(secs: f32) -> String {
    let secs = secs.max(0.0);
    let m = (secs / 60.0) as u32;
    let s = (secs as u32) % 60;
    let ms = (secs.fract() * 1000.0) as u32;
    format!("{m}:{s:02}.{ms:03}")
}

/// Clips demo read-only para a fase 1 (sem arquivo de áudio real; a engine
/// sintetiza a waveform).
fn demo_clips() -> Vec<ClipDemo> {
    const ROWS: &[(usize, f32, f32, bool, usize, &str)] = &[
        (0, 0.0, 4.0, true, 2, "Bateria A"),
        (0, 4.0, 4.0, true, 2, "Bateria B"),
        (1, 0.0, 8.0, false, 4, "Baixo"),
        (2, 0.0, 8.0, false, 3, "Piano"),
        (2, 8.0, 4.0, false, 3, "Piano BR"),
        (3, 0.0, 4.0, false, 5, "Pad"),
        (3, 4.0, 4.0, false, 1, "Guitarra"),
        (3, 8.0, 4.0, false, 5, "Pad 2"),
    ];
    ROWS.iter()
        .map(|&(track, beat, len, is_midi, color, name)| ClipDemo {
            track_index: track,
            beat,
            len: len.max(0.05),
            is_midi,
            color,
            name: Some(name.to_string()),
            seam_beat: None,
            pre_glue_len: None,
            pre_glue_color: None,
            wave: Vec::new(),
            file_path: None,
            source_start_beats: beat,
            source_length_beats: len,
            channels: if is_midi { 1 } else { 2 },
        })
        .collect()
}

/// Componente raiz do app.
#[derive(Live, LiveHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust]
    shell: ShellState,
}

impl LiveRegister for App {
    fn live_register(cx: &mut Cx) {
        makepad_widgets::live_design(cx);
        cx.link(live_id!(theme), live_id!(theme_desktop_dark));
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.shell = ShellState::build();
        // Atualiza o relógio do transport a cada 100 ms.
        let _ = cx.start_interval(0.1);
        self.shell.refresh_status(&self.ui, cx);
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        if self.ui.button(id!(play_btn)).clicked(&actions) {
            self.shell.toggle_play(&self.ui, cx);
        }
        if self.ui.button(id!(stop_btn)).clicked(&actions) {
            self.shell.stop(&self.ui, cx);
        }
    }

    fn handle_timer(&mut self, cx: &mut Cx, _event: &TimerEvent) {
        self.shell.refresh_status(&self.ui, cx);
    }

    fn handle_key_down(&mut self, cx: &mut Cx, event: &KeyEvent) {
        if event.key_code == KeyCode::Space {
            self.shell.toggle_play(&self.ui, cx);
        }
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}