//! Medidores de nível: do pico cru da engine até a barra na tela.
//!
//! # Onde cada coisa é decidida
//!
//! A engine publica o **pico do buffer** por canal, em amplitude
//! linear (ver `daw_audio::meter`). Tudo o que é apresentação acontece
//! aqui, na thread da UI, e nessa ordem:
//!
//! 1. **Conversão para dB** e normalização 0..1. Um VU linear
//!    (altura = amplitude) joga quase todo o material para o fundo da
//!    escala: um sinal a -20 dB ocupa 10% da barra. Em dB, a mesma
//!    coisa ocupa dois terços.
//! 2. **Envelope de VU**: ataque instantâneo, decaimento por tick.
//!    Ataque instantâneo porque um VU que sobe devagar parece lag;
//!    decaimento porque sem ele a barra pisca no ruído de fundo.
//! 3. **Peak-hold**: a linha branca segura o pico por ~1 s antes de
//!    descer. Sem ela o usuário nunca vê o topo que o sinal alcançou.
//!
//! # Por que a curva está escrita duas vezes
//!
//! A conversão dB <-> linear aparece aqui e em
//! `ui/components/audio_scale.slint`. Não é duplicação acidental: o
//! Rust normaliza a **barra** (que vem da medição) e o Slint posiciona
//! o **fader** (que o usuário arrasta). Se o Rust publicasse a posição
//! do fader, arrastar o triangle passaria por um modelo a cada pixel de
//! mouse — latência no controle que precisa ser o mais responsivo da
//! tela. As funções são de três linhas e as constantes são as mesmas;
//! qualquer mudança tem que ser feita nas duas, e ambas dizem isso.
//!
//! # Taxa de atualização
//!
//! Um pico por buffer de áudio (~2,7 ms a 48 kHz com buffer pequeno) é
//! MUITO mais rápido que a tela. Bombear a 30 Hz ([`PUMP_INTERVAL`]) é
//! o suficiente para o olho ler como contínuo e mantém o custo por
//! canal em quatro escritas de modelo. A animação DENTRO do Slint foi
//! descartada de propósito: suja a cena a cada quadro e satura o
//! renderer custom (o playhead engasga) — ver o aviso em `playback.rs`.

use std::rc::Rc;
use std::time::Duration;

use slint::{Model, VecModel};

use crate::audio_bridge::AudioBridge;

/// Floats por canal no modelo da UI: L, R, pico L, pico R.
///
/// São 4 e não 2 porque o Slint recebe barra e peak-hold já
/// suavizados, que evoluem em taxas diferentes. O layout é o mesmo dos
/// canais do mixer (`mixer_state()`): índice do canal em
/// `canal * FLOATS_PER_CHANNEL + campo`.
pub const FLOATS_PER_CHANNEL: usize = 4;

/// Índices dentro do bloco de um canal.
pub const FIELD_LEFT: usize = 0;
pub const FIELD_RIGHT: usize = 1;
pub const FIELD_PEAK_LEFT: usize = 2;
pub const FIELD_PEAK_RIGHT: usize = 3;

/// Bools por canal no modelo de clip: L, R.
pub const CLIP_BOOLS_PER_CHANNEL: usize = 2;

/// Intervalo do bombeamento dos medidores (~30 Hz).
pub const PUMP_INTERVAL: Duration = Duration::from_millis(33);

/// Piso da escala, em dBFS. Abaixo disso a barra fica no chão.
///
/// -60 dB é o piso padrão de VU: 60 dB de faixa útil, que é o que um
/// olho distingue como "presença" contra silêncio. Abaixo disso só
/// existe ruído de fundo, e a barra passaria a tremer no áudio do
/// sistema.
pub const DB_FLOOR: f32 = -60.0;

/// Teto da escala, em dBFS.
///
/// Acima de 0 dB é clip, e o clip tem luz própria. Esticar a barra
/// para +6 dB só faria a luz de clip parecer parte do medidor.
pub const DB_CEILING: f32 = 0.0;

/// Amplitude -> dBFS.
///
/// `0.0` é silêncio e não tem dB; devolvemos o piso em vez de
/// `-inf`/`NaN`, que contaminaria a normalização e o Slint não tem como
/// desenhar.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 1e-6 {
        return DB_FLOOR;
    }
    20.0 * linear.log10()
}

/// dBFS -> amplitude linear.
pub fn db_to_linear(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

/// Pico linear -> altura de barra 0..1 na escala dB.
pub fn meter_norm(peak_linear: f32) -> f32 {
    let db = linear_to_db(peak_linear);
    ((db - DB_FLOOR) / (DB_CEILING - DB_FLOOR)).clamp(0.0, 1.0)
}

/// Posição do fader (0 = baixo, 1 = topo) -> ganho linear.
///
/// Mesma curva do medidor, e por isso o topo do fader e a leitura de
/// unity do VU são o MESMO lugar: um volume em 0 dB põe a barra no topo
/// e o triangle no topo. É o que faz o par fader+meter ser legível de
/// relance.
///
/// O fundo é SILÊNCIO de verdade, não -60 dB: um fader que não chega
/// a zero é um fader que obriga o usuário a mutar em outro botão para
/// calar a faixa.
pub fn fader_pos_to_linear(pos: f32) -> f32 {
    let pos = pos.clamp(0.0, 1.0);
    if pos <= 0.0 {
        return 0.0;
    }
    db_to_linear(DB_FLOOR + pos * (DB_CEILING - DB_FLOOR))
}

/// Ganho linear -> posição do fader.
pub fn linear_to_fader_pos(linear: f32) -> f32 {
    ((linear_to_db(linear) - DB_FLOOR) / (DB_CEILING - DB_FLOOR)).clamp(0.0, 1.0)
}

/// Decaimento da barra por tick a 30 Hz.
///
/// 0,82 dá meia-vida de ~3,5 ticks (~120 ms): o VU volta rápido o
/// bastante para não mentir, devagar o bastante para o olho ler o
/// silêncio chegando.
const LEVEL_DECAY: f32 = 0.82;

/// Decaimento do peak-hold por tick depois de expirar o hold.
///
/// Bem mais lento que o da barra (meia-vida de ~14 ticks, ~0,5 s): a
/// linha de peak existe para mostrar o máximo alcançado, e descer junto
/// com a barra seria redundante.
const PEAK_DECAY: f32 = 0.95;

/// Ticks que o pico é segurado antes de começar a descer (~1 s a 30 Hz).
const PEAK_HOLD_TICKS: u32 = 30;

/// Estado de suavização de um canal.
///
/// Vive só na thread da UI. Os `*_level` são o que a barra mostra; os
/// `*_peak` alimentam a linha de peak-hold.
#[derive(Clone, Copy, Default)]
struct ChannelMeter {
    level_left: f32,
    level_right: f32,
    peak_left: f32,
    peak_right: f32,
    /// Ticks que o peak-hold ainda segura antes de começar a cair.
    ///
    /// Um por lado, e decrementado UMA vez por tick. Com um contador
    /// único para os dois lados, o laço interno decrementava duas
    /// vezes por chamada e o hold durava metade do declarado.
    hold_left: u32,
    hold_right: u32,
}

impl ChannelMeter {
    /// Avança o envelope um tick com o pico medido.
    fn advance(&mut self, peak_left: f32, peak_right: f32) {
        // Ataque instantâneo, decaimento suave: o VU tem que subir
        // junto com o som, não atrás dele.
        for (level, peak) in [
            (&mut self.level_left, peak_left),
            (&mut self.level_right, peak_right),
        ] {
            if peak > *level {
                *level = peak;
            } else {
                *level *= LEVEL_DECAY;
            }
        }

        for (held, hold, level, peak) in [
            (
                &mut self.peak_left,
                &mut self.hold_left,
                self.level_left,
                peak_left,
            ),
            (
                &mut self.peak_right,
                &mut self.hold_right,
                self.level_right,
                peak_right,
            ),
        ] {
            if peak > *held {
                *held = peak;
                *hold = PEAK_HOLD_TICKS;
            } else if *hold > 0 {
                *hold -= 1;
            } else {
                // O pico decai, mas NUNCA abaixo do nível vivo.
                //
                // O sinal atual é, por definição, um pico recente: se a
                // linha descesse enquanto a barra ainda mostrava o mesmo
                // valor, o "hold" ficaria visualmente ABAIXO do que já
                // passou, e a linha leria como ruído em vez de máximo.
                *held = (*held * PEAK_DECAY).max(level);
            }
        }
    }
}

/// Modelos de medição COMPARTILHADOS entre mixer e track panel.
///
/// Um único `Rc<VecModel>` por campo, apontado pelas DUAS superfícies:
/// um `set_row_data` por canal atualiza o mixer e a timeline no mesmo
/// quadro, e as barras dos dois lados não podem divergir porque leem o
/// mesmo número. É o mesmo truque de `MixerModels`.
pub struct MeterModels {
    /// `FLOATS_PER_CHANNEL` por canal: L, R, pico L, pico R (0..1).
    pub values: Rc<VecModel<f32>>,
    /// `CLIP_BOOLS_PER_CHANNEL` por canal: L, R.
    pub clipped: Rc<VecModel<bool>>,
}

impl MeterModels {
    pub fn new(channels: usize) -> Self {
        Self {
            values: Rc::new(VecModel::from(vec![0.0; channels * FLOATS_PER_CHANNEL])),
            clipped: Rc::new(VecModel::from(vec![false; channels * CLIP_BOOLS_PER_CHANNEL])),
        }
    }

    /// Canais que o modelo comporta agora.
    pub fn channels(&self) -> usize {
        self.values.row_count() / FLOATS_PER_CHANNEL
    }

    /// Cria/expande os modelos para `channels`, preservando o que já
    /// estava aceso.
    ///
    /// Criar track no meio da sessão não pode apagar os medidores das
    /// outras, então o conteúdo antigo é copiado. Só muda de tamanho
    /// quando o número de trilhas muda; no caminho comum é um compare.
    pub fn ensure_channels(&self, channels: usize) {
        if self.channels() == channels {
            return;
        }
        self.values
            .set_vec(resized(snapshot(&self.values), channels * FLOATS_PER_CHANNEL));
        self.clipped
            .set_vec(resized(snapshot(&self.clipped), channels * CLIP_BOOLS_PER_CHANNEL));
    }

    fn publish(&self, channel: usize, state: &ChannelMeter, clipped: (bool, bool)) {
        let base = channel * FLOATS_PER_CHANNEL;
        self.values
            .set_row_data(base + FIELD_LEFT, meter_norm(state.level_left));
        self.values
            .set_row_data(base + FIELD_RIGHT, meter_norm(state.level_right));
        self.values
            .set_row_data(base + FIELD_PEAK_LEFT, meter_norm(state.peak_left));
        self.values
            .set_row_data(base + FIELD_PEAK_RIGHT, meter_norm(state.peak_right));
        let cbase = channel * CLIP_BOOLS_PER_CHANNEL;
        self.clipped.set_row_data(cbase, clipped.0);
        self.clipped.set_row_data(cbase + 1, clipped.1);
    }
}

/// Estica/encurta preservando o prefixo comum.
fn resized<T: Copy + Default>(current: Vec<T>, len: usize) -> Vec<T> {
    let mut out = vec![T::default(); len];
    for (dst, src) in out.iter_mut().zip(current.iter()) {
        *dst = *src;
    }
    out
}

/// Copia o conteúdo de um `VecModel` para um `Vec`.
///
/// `Model::row_data` recebe o índice e devolve `Option`, então não dá
/// para ler o modelo inteiro de uma vez. Custa `row_count` leituras — e
/// só roda quando o número de canais muda, não a cada tick.
fn snapshot<T: Copy + Default + 'static>(model: &VecModel<T>) -> Vec<T> {
    (0..model.row_count())
        .map(|i| model.row_data(i).unwrap_or_default())
        .collect()
}

/// Bombeia a medição da engine para os modelos do Slint.
pub struct MeterPump {
    meters: MeterModels,
    /// Um por canal (trilhas + master), na mesma ordem.
    channels: Vec<ChannelMeter>,
}

impl MeterPump {
    pub fn new(channels: usize) -> Self {
        Self {
            meters: MeterModels::new(channels),
            channels: vec![ChannelMeter::default(); channels],
        }
    }

    pub fn models(&self) -> &MeterModels {
        &self.meters
    }

    /// Lê a engine, suaviza e publica.
    ///
    /// `channels` é `trilhas + master`, a mesma ordem de `mixer_state()`
    /// e a mesma que a engine publica — é o que mantém o índice do
    /// medidor e o índice do canal do mixer coincidentes sem mapa.
    pub fn tick(&mut self, audio: &AudioBridge, channels: usize) {
        if audio.is_silent() {
            return;
        }
        let bank = audio.meters();
        // A engine declara quantos canais existem. Enquanto a contagem
        // não bater com o projeto, publicar seria escrever a medição no
        // canal errado.
        if bank.channel_count() != channels {
            audio.set_meter_channels(channels);
            if bank.channel_count() != channels {
                return;
            }
        }

        self.meters.ensure_channels(channels);
        if self.channels.len() != channels {
            self.channels.resize(channels, ChannelMeter::default());
        }

        for channel in 0..channels {
            let reading = bank.read(channel);
            let state = &mut self.channels[channel];
            state.advance(reading.peak_left, reading.peak_right);
            self.meters.publish(
                channel,
                state,
                (reading.clipped_left, reading.clipped_right),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_one_is_zero_db() {
        assert!(linear_to_db(1.0).abs() < 1e-5);
    }

    #[test]
    fn silence_is_the_floor_not_negative_infinity() {
        assert_eq!(linear_to_db(0.0), DB_FLOOR);
        // O piso normaliza para 0 e não para NaN — o Slint não tem como
        // desenhar NaN.
        assert_eq!(meter_norm(0.0), 0.0);
    }

    #[test]
    fn unity_sits_at_the_top_of_the_meter() {
        assert_eq!(meter_norm(1.0), 1.0);
    }

    #[test]
    fn a_quiet_signal_is_readable_in_db_but_not_linear() {
        // -20 dBFS: 10% da barra na escala linear, 2/3 na escala dB.
        let quiet = db_to_linear(-20.0);
        assert!((quiet - 0.1).abs() < 1e-3);
        let db_position = meter_norm(quiet);
        assert!(
            (db_position - 0.666).abs() < 0.01,
            "esperado ~2/3 da barra em dB, veio {}",
            db_position,
        );
    }

    #[test]
    fn above_unity_is_clamped_to_the_top() {
        assert_eq!(meter_norm(4.0), 1.0);
    }

    #[test]
    fn fader_and_meter_share_one_curve() {
        // O ponto do fader e a altura da barra concordam para o mesmo
        // ganho: é isso que faz o par ser legível.
        for linear in [0.05f32, 0.25, 0.5, 0.8, 1.0] {
            let pos = linear_to_fader_pos(linear);
            let back = fader_pos_to_linear(pos);
            assert!(
                (back - linear).abs() < 0.01,
                "ida e volta mudou o ganho: {} -> {} -> {}",
                linear,
                pos,
                back,
            );
        }
    }

    #[test]
    fn fader_bottom_is_real_silence_and_top_is_unity() {
        assert_eq!(fader_pos_to_linear(0.0), 0.0);
        assert!((fader_pos_to_linear(1.0) - 1.0).abs() < 1e-4);
        assert_eq!(linear_to_fader_pos(0.0), 0.0);
        assert_eq!(linear_to_fader_pos(1.0), 1.0);
    }

    #[test]
    fn fader_position_is_monotonic() {
        let mut last = -1.0;
        for step in 0..=100 {
            let pos = step as f32 / 100.0;
            let linear = fader_pos_to_linear(pos);
            assert!(linear > last, "ganho não é monótono em pos={}", pos);
            last = linear;
        }
    }

    #[test]
    fn envelope_attacks_instantly_and_decays() {
        let mut c = ChannelMeter::default();
        c.advance(1.0, 1.0);
        assert_eq!(c.level_left, 1.0, "ataque tem que ser instantâneo");

        c.advance(0.0, 0.0);
        assert_eq!(c.level_left, LEVEL_DECAY, "decaimento por tick");
        assert_eq!(c.peak_left, 1.0, "o pico segura no primeiro tick");
    }

    #[test]
    fn peak_hold_survives_a_short_gap() {
        let mut c = ChannelMeter::default();
        c.advance(0.8, 0.8);
        // Silêncio por menos que o hold: o pico não pode cair.
        for _ in 0..PEAK_HOLD_TICKS - 5 {
            c.advance(0.0, 0.0);
        }
        assert_eq!(c.peak_left, 0.8);
    }

    #[test]
    fn the_hold_lasts_the_declared_ticks_on_both_sides() {
        // Regressão: com UM contador para os dois lados, o laço interno
        // decrementava duas vezes por chamada e o hold acabava na
        // metade — a linha descia cedo demais justamente no lado que o
        // usuário estava ouvindo.
        let mut c = ChannelMeter::default();
        c.advance(0.8, 0.8);
        assert_eq!(c.hold_left, PEAK_HOLD_TICKS);
        assert_eq!(c.hold_right, PEAK_HOLD_TICKS);

        for _ in 1..PEAK_HOLD_TICKS {
            c.advance(0.0, 0.0);
            assert!(c.hold_left > 0, "esquerdo acabou antes do hold");
            assert!(c.hold_right > 0, "direito acabou antes do hold");
            assert_eq!(c.peak_left, 0.8, "pico caiu durante o hold");
        }

        // No último tick o contador zera; a linha só começa a descer no
        // seguinte.
        c.advance(0.0, 0.0);
        assert_eq!(c.hold_left, 0);
        assert_eq!(c.hold_right, 0);
        assert_eq!(c.peak_left, 0.8);
    }

    #[test]
    fn peak_falls_after_the_hold_expires() {
        let mut c = ChannelMeter::default();
        c.advance(0.8, 0.8);
        for _ in 0..PEAK_HOLD_TICKS + 5 {
            c.advance(0.0, 0.0);
        }
        assert!(c.peak_left < 0.8, "o pico tem que ceder depois do hold");
    }

    #[test]
    fn peak_never_sits_below_the_live_level() {
        let mut c = ChannelMeter::default();
        c.advance(0.2, 0.2);
        for _ in 0..PEAK_HOLD_TICKS + 40 {
            c.advance(0.2, 0.2);
            assert!(c.peak_left >= c.level_left, "peak abaixo do nível");
        }
    }

    #[test]
    fn growing_channels_keeps_the_ones_already_alive() {
        let models = MeterModels::new(2);
        models.values.set_row_data(0, 0.5);
        models.values.set_row_data(FLOATS_PER_CHANNEL, 0.9);
        models.clipped.set_row_data(0, true);

        models.ensure_channels(4);

        assert_eq!(models.channels(), 4);
        assert_eq!(models.values.row_data(0), Some(0.5));
        assert_eq!(models.values.row_data(FLOATS_PER_CHANNEL), Some(0.9));
        assert_eq!(models.clipped.row_data(0), Some(true));
        assert_eq!(models.values.row_count(), 4 * FLOATS_PER_CHANNEL);
    }

    #[test]
    fn shrinking_channels_does_not_leak_rows() {
        let models = MeterModels::new(4);
        models.ensure_channels(2);
        assert_eq!(models.channels(), 2);
        assert_eq!(models.values.row_count(), 2 * FLOATS_PER_CHANNEL);
    }

    #[test]
    fn ensure_channels_is_idempotent() {
        let models = MeterModels::new(3);
        models.values.set_row_data(2, 0.75);
        models.ensure_channels(3);
        assert_eq!(models.values.row_data(2), Some(0.75));
    }
}
