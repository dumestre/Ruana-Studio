//! Geração de sinais sintéticos estéreo para o laboratório.
//!
//! Nada de arquivos: cada sinal é gerado em memória com um
//! nível de dB diferente, para validar que a waveform desenha
//! a amplitude REAL. Inclui a armadilha L=+1/R=-1 que expõe
//! o problema do downmix mono.

/// Um sinal de teste: canais L e R separados + duração.
pub struct TestSignal {
    #[allow(dead_code)]
    pub name: &'static str,
    /// Rótulo exibido no combo (WAVs usam o nome do arquivo).
    pub label_name: String,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub sample_rate: f32,
}

impl TestSignal {
    pub fn duration(&self) -> f32 {
        self.left.len() as f32 / self.sample_rate
    }
}

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Gerador pseudo-aleatório determinístico (sem dependências).
struct Rng(u64);
impl Rng {
    fn next_f32(&mut self) -> f32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545F4914F6CDD1D);
        ((v >> 40) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
    }
}

/// Seno estéreo, -6 dB (~0.5 de amplitude), L defasado de R.
pub fn sine_stereo(sample_rate: u32, seconds: f32) -> TestSignal {
    let n = (sample_rate as f32 * seconds) as usize;
    let amp = db_to_amp(-6.0);
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / sample_rate as f32;
        left.push((t * 220.0 * std::f32::consts::TAU).sin() * amp);
        right.push((t * 330.0 * std::f32::consts::TAU).sin() * amp);
    }
    TestSignal { name: "sine",
        label_name: "Seno estéreo (-6 dB)".into(), left, right, sample_rate: sample_rate as f32 }
}

/// Dentes de serra, -18 dB, com envelope de "frases" para dar forma.
pub fn saw_pairs(sample_rate: u32, seconds: f32) -> TestSignal {
    let n = (sample_rate as f32 * seconds) as usize;
    let amp = db_to_amp(-18.0);
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / sample_rate as f32;
        // Envelope tipo "notas" de 1/2 segundo.
        let note_t = (t % 0.5) / 0.5;
        let env = (note_t * std::f32::consts::PI).sin();
        let phase_l = (t * 110.0) % 1.0;
        let phase_r = (t * 165.0) % 1.0;
        left.push((phase_l * 2.0 - 1.0) * amp * env);
        right.push((phase_r * 2.0 - 1.0) * amp * env * 0.8);
    }
    TestSignal { name: "saw",
        label_name: "Saw dupla (-18 dB)".into(), left, right, sample_rate: sample_rate as f32 }
}

/// Ruído branco baixo, -40 dB — quase invisível em render ruim.
pub fn noise_quiet(sample_rate: u32, seconds: f32) -> TestSignal {
    let n = (sample_rate as f32 * seconds) as usize;
    let amp = db_to_amp(-40.0);
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for _ in 0..n {
        let a = rng.next_f32() * amp;
        let b = rng.next_f32() * amp;
        left.push(a);
        right.push(b);
    }
    TestSignal { name: "noise",
        label_name: "Ruído baixo (-40 dB)".into(), left, right, sample_rate: sample_rate as f32 }
}

/// A armadilha do downmix: L constante +1, R constante -1.
/// O mono vira silêncio; o estéreo mostra dois canais cheios.
pub fn out_of_phase(sample_rate: u32, seconds: f32) -> TestSignal {
    let n = (sample_rate as f32 * seconds) as usize;
    // Pulso intermitente pra ficar óbvio onde está cada canal.
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for i in 0..n {
        let on = (i as f32 / sample_rate as f32) % 1.0 < 0.5;
        left.push(if on { 1.0 } else { 0.0 });
        right.push(if on { -1.0 } else { 0.0 });
    }
    TestSignal { name: "antiphase",
        label_name: "Armadilha L=+1 R=-1".into(), left, right, sample_rate: sample_rate as f32 }
}

/// Todos os sinais do lab.
pub fn all_signals() -> Vec<TestSignal> {
    vec![
        sine_stereo(44100, 4.0),
        saw_pairs(44100, 4.0),
        noise_quiet(44100, 4.0),
        out_of_phase(44100, 4.0),
    ]
}
