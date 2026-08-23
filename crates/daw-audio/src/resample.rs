//! Conversão de sample rate.
//!
//! Roda no carregamento do arquivo, nunca na thread de áudio — então
//! pode gastar qualidade em vez de economizar ciclos.
//!
//! # Por que não interpolação linear
//!
//! Interpolação linear é um filtro passa-baixa muito ruim: reamostrar
//! ponto a ponto ao reduzir a taxa (96 kHz → 48 kHz, por exemplo)
//! dobra tudo acima do novo Nyquist de volta para dentro da banda
//! audível. O resultado é um chiado áspero em cima do sample — parte
//! do som "granulado". Um sinc janelado (Lanczos) filtra antes de
//! decimar e resolve isso.

/// Meia-largura do kernel Lanczos, em períodos de sinc.
///
/// 4 dá ~ -60 dB de rejeição fora da banda, o suficiente para material
/// musical, com 8–16 taps por sample de saída.
const LANCZOS_A: f64 = 4.0;

#[inline]
fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// Kernel Lanczos: sinc(x) janelado por sinc(x/a).
#[inline]
fn lanczos(x: f64, a: f64) -> f64 {
    if x.abs() >= a {
        0.0
    } else {
        sinc(x) * sinc(x / a)
    }
}

/// Reamostra com sinc janelado (Lanczos), com anti-aliasing.
///
/// Ao reduzir a taxa, a frequência de corte do kernel acompanha o novo
/// Nyquist, então o conteúdo que não cabe na taxa de destino é
/// filtrado em vez de dobrado.
pub fn resample_sinc(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() || from_rate == 0 || to_rate == 0 {
        return input.to_vec();
    }

    // Samples de origem consumidos por sample de saída.
    let ratio = from_rate as f64 / to_rate as f64;
    // Corte normalizado ao Nyquist da origem. Ao ampliar a taxa não
    // há o que filtrar (1.0); ao reduzir, corta no Nyquist do destino.
    let cutoff = (1.0 / ratio).min(1.0);
    // Meia-janela medida em samples de ORIGEM.
    let half = LANCZOS_A / cutoff;

    let output_len = (input.len() as f64 / ratio).ceil() as usize;
    let last = input.len() as isize - 1;

    let mut out = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let center = i as f64 * ratio;
        let first = (center - half).ceil() as isize;
        let stop = (center + half).floor() as isize;

        let mut acc = 0.0f64;
        let mut weight = 0.0f64;
        let mut j = first;
        while j <= stop {
            // Espelha nas bordas em vez de assumir zero: zero fora do
            // arquivo puxaria as pontas para baixo e criaria um clique.
            let idx = j.clamp(0, last.max(0)) as usize;
            let w = lanczos((j as f64 - center) * cutoff, LANCZOS_A);
            acc += input[idx] as f64 * w;
            weight += w;
            j += 1;
        }

        // Normaliza pela soma dos pesos: mantém o nível DC exato,
        // inclusive nas bordas onde a janela é truncada.
        out.push(if weight.abs() > 1e-12 {
            (acc / weight) as f32
        } else {
            0.0
        });
    }
    out
}

/// Reamostragem linear (ponto a ponto, sem filtro).
///
/// Mantida para casos onde só interessa velocidade — desenho de
/// waveform, por exemplo. Para áudio que vai soar, use
/// [`resample_sinc`].
pub fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (input.len() as f64 / ratio).ceil() as usize;

    (0..output_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;

            let a = input.get(idx).copied().unwrap_or(0.0);
            let b = input.get(idx + 1).copied().unwrap_or(0.0);

            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_rate_passthrough() {
        let input = vec![1.0, 2.0, 3.0];
        assert_eq!(resample_linear(&input, 44100, 44100), input);
        assert_eq!(resample_sinc(&input, 44100, 44100), input);
    }

    #[test]
    fn empty_input() {
        assert!(resample_linear(&[], 44100, 48000).is_empty());
        assert!(resample_sinc(&[], 44100, 48000).is_empty());
    }

    #[test]
    fn linear_point_samples_the_source() {
        // Interpolação linear amostra a posição exata: em 2:1 pega os
        // índices pares, sem média nenhuma. É justamente por isso que
        // ela não serve para reduzir a taxa.
        let input = vec![0.0, 1.0, 0.0, -1.0];
        let out = resample_linear(&input, 44100, 22050);
        assert_eq!(out, vec![0.0, 0.0]);
    }

    #[test]
    fn linear_interpolates_when_upsampling() {
        let input = vec![0.0, 1.0];
        let out = resample_linear(&input, 22050, 44100);
        assert_eq!(out.len(), 4);
        assert!((out[0] - 0.0).abs() < 0.01);
        assert!((out[1] - 0.5).abs() < 0.01);
        assert!((out[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn sinc_output_length_follows_ratio() {
        let input = vec![0.0; 100];
        assert_eq!(resample_sinc(&input, 48000, 96000).len(), 200);
        assert_eq!(resample_sinc(&input, 48000, 24000).len(), 50);
    }

    #[test]
    fn sinc_preserves_dc() {
        // Sinal constante tem que sair constante, borda incluída — é o
        // que a normalização pelos pesos garante.
        let input = vec![0.5f32; 64];
        for (from, to) in [(48000, 44100), (44100, 48000), (96000, 48000)] {
            let out = resample_sinc(&input, from, to);
            for (i, &s) in out.iter().enumerate() {
                assert!((s - 0.5).abs() < 1e-3, "{}→{} idx {} = {}", from, to, i, s);
            }
        }
    }

    #[test]
    fn sinc_rejects_aliasing_on_downsample() {
        // Alternado ±1 = exatamente o Nyquist da origem. Ao reduzir 2:1
        // ele não cabe na banda de destino: tem que ser filtrado, não
        // dobrado. É este o teste do "granulado".
        let input: Vec<f32> = (0..256)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let out = resample_sinc(&input, 96000, 48000);
        assert_eq!(out.len(), 128);

        let peak = out[8..out.len() - 8]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak < 0.05, "aliasing passou: pico {}", peak);

        // A versão linear deixa passar inteiro — a comparação que
        // justifica a troca.
        let linear = resample_linear(&input, 96000, 48000);
        let linear_peak = linear.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(linear_peak > 0.9, "linear deveria aliasar: {}", linear_peak);
    }

    #[test]
    fn sinc_keeps_a_low_tone_intact() {
        // Uma senoide bem abaixo do corte tem que sobreviver com
        // amplitude praticamente igual.
        let n = 512;
        let freq = 1000.0f64;
        let from = 48000u32;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                (2.0 * std::f64::consts::PI * freq * i as f64 / from as f64).sin() as f32 * 0.8
            })
            .collect();
        let out = resample_sinc(&input, from, 24000);
        let peak = out[16..out.len() - 16]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!((peak - 0.8).abs() < 0.05, "tom de 1 kHz saiu em {}", peak);
    }
}
