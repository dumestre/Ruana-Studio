//! Timeline renderer standalone — wgpu windowed (canvas 2D style).
//!
//! Replica visualmente a timeline do app: régua, grid, clips, playhead e VU.
//! Usa o MESMO shader (app/src/timeline_gpu/wgpu.wgsl) e a mesma lógica de
//! instancing + storage buffer de waveform do `TimelineGpuRenderer` do app.
//!
//! Controles:
//!   - Roda do mouse / touchpad 2 dedos: scroll vertical (e lateral)
//!   - Click na roda + arrastar:          pan na timeline
//!   - Ctrl + PixelDelta vertical:        zoom horizontal (onde suportado)
//!   - Alt + PixelDelta vertical:         zoom vertical (altura das faixas)
//!   - Espaço:                            play/pause do playhead animado
//!   - ESC:                               fecha
//!   - Resize:                            redimensiona o canvas

fn main() -> Result<(), Box<dyn std::error::Error>> {
    timeline_render::run()
}
