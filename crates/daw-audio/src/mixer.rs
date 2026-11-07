/// Mix stereo profissional.
///
/// Converte um sample mono em stereo aplicando volume e pan,
/// e soma múltiplos tracks num buffer de saída stereo.

/// Calcula os ganhos L/R a partir de volume (0..1) e pan (0..1, 0.5=center).
///
/// Usa lei do cosseno (equal power) para pan stereo:
///   L = volume * cos(pan * π/2)
///   R = volume * sin(pan * π/2)
///
/// Isso mantém potência constante ao varrer o pan (padrão DAW profissional).
pub fn stereo_gains(volume: f32, pan: f32) -> (f32, f32) {
    let angle = pan as f64 * std::f64::consts::FRAC_PI_2;
    let gain_l = volume * angle.cos() as f32;
    let gain_r = volume * angle.sin() as f32;
    (gain_l, gain_r)
}

/// Calcula os ganhos L/R a partir de volume (0..1) e pan (-1..1).
///
/// Convenção do beat sequencer (`DrumTrackParams.pan`), onde 0 é o
/// centro. Converte para 0..1 e reusa a lei do cosseno.
pub fn stereo_gains_bipolar(volume: f32, pan: f32) -> (f32, f32) {
    stereo_gains(volume, (pan.clamp(-1.0, 1.0) + 1.0) * 0.5)
}

/// Aplica pan e volume num sample mono → retorna (left, right).
#[inline]
pub fn pan_stereo(sample: f32, gain_l: f32, gain_r: f32) -> (f32, f32) {
    (sample * gain_l, sample * gain_r)
}

/// Verifica se uma track deve ser ouvida no mix (mute/solo logic).
///
/// Regra DAW padrão:
/// - Se NENHUM track está soloed → todas as não-mudas são ouvidas
/// - Se PELO MENOS UM track está soloed → só os soloed (ignora mute) são ouvidos
pub fn track_is_active(muted: bool, soloed: bool, any_soloed: bool) -> bool {
    if any_soloed { soloed } else { !muted }
}

/// Mix mono para stereo buffer (1 frame).
///
/// `sample`: valor mono (-1.0..1.0)
/// `volume`: ganho linear 0..1
/// `pan`: panorama 0..1 (0.5 = centro)
/// `out_left`: acumula no canal esquerdo
/// `out_right`: acumula no canal direito
#[inline]
pub fn mix_stereo_frame(
    sample: f32,
    volume: f32,
    pan: f32,
    out_left: &mut f32,
    out_right: &mut f32,
) {
    let (gain_l, gain_r) = stereo_gains(volume, pan);
    let (l, r) = pan_stereo(sample, gain_l, gain_r);
    *out_left += l;
    *out_right += r;
}

/// Soma uma fonte STEREO no bus, com ganhos de canal já resolvidos.
///
/// Fontes stereo não passam por pan de mono: cada lado do arquivo vai
/// para o lado correspondente da saída, senão a imagem original do
/// sample seria destruída.
#[inline]
pub fn mix_stereo_source(
    src_l: f32,
    src_r: f32,
    gain_l: f32,
    gain_r: f32,
    out_left: &mut f32,
    out_right: &mut f32,
) {
    *out_left += src_l * gain_l;
    *out_right += src_r * gain_r;
}

/// Clamp stereo frame para [-1.0, 1.0].
#[inline]
pub fn clamp_stereo(left: f32, right: f32) -> (f32, f32) {
    (left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0))
}

/// Limitador macio (tanh) no lugar do clip duro.
///
/// Somar várias vozes passa de 1.0 com facilidade; cortar no ±1.0
/// gera harmônicos de ordem alta — o "granulado" clássico de saída
/// saturada. A curva só age perto do teto: abaixo de ~0.6 é
/// praticamente linear, então o material com headroom não muda.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    const THRESHOLD: f32 = 0.6;
    let a = x.abs();
    if a <= THRESHOLD {
        return x;
    }
    let over = a - THRESHOLD;
    let range = 1.0 - THRESHOLD;
    let shaped = THRESHOLD + range * (over / range).tanh();
    if x < 0.0 { -shaped } else { shaped }
}

/// Aplica o limitador macio nos dois canais.
#[inline]
pub fn limit_stereo(left: f32, right: f32) -> (f32, f32) {
    (soft_clip(left), soft_clip(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_center_equal_power() {
        let (l, r) = stereo_gains(1.0, 0.5);
        // cos(π/4) = sin(π/4) ≈ 0.707
        assert!((l - 0.707).abs() < 0.01);
        assert!((r - 0.707).abs() < 0.01);
    }

    #[test]
    fn pan_full_left() {
        let (l, r) = stereo_gains(1.0, 0.0);
        assert!((l - 1.0).abs() < 0.01);
        assert!(r.abs() < 0.01);
    }

    #[test]
    fn pan_full_right() {
        let (l, r) = stereo_gains(1.0, 1.0);
        assert!(l.abs() < 0.01);
        assert!((r - 1.0).abs() < 0.01);
    }

    #[test]
    fn zero_volume() {
        let (l, r) = stereo_gains(0.0, 0.5);
        assert!(l.abs() < 0.001);
        assert!(r.abs() < 0.001);
    }

    #[test]
    fn solo_logic() {
        assert!(track_is_active(false, false, false));
        assert!(!track_is_active(true, false, false));
        assert!(track_is_active(true, true, true));
        assert!(track_is_active(false, true, true));
        assert!(!track_is_active(false, false, true));
    }

    #[test]
    fn mix_frame_accumulates() {
        let mut l = 0.0;
        let mut r = 0.0;
        mix_stereo_frame(1.0, 1.0, 0.5, &mut l, &mut r);
        mix_stereo_frame(0.5, 0.5, 0.5, &mut l, &mut r);
        assert!(l > 0.0);
        assert!(r > 0.0);
    }

    #[test]
    fn bipolar_pan_center_is_equal_power() {
        let (l, r) = stereo_gains_bipolar(1.0, 0.0);
        assert!((l - 0.707).abs() < 0.01);
        assert!((r - 0.707).abs() < 0.01);
    }

    #[test]
    fn bipolar_pan_extremes() {
        let (l, r) = stereo_gains_bipolar(1.0, -1.0);
        assert!((l - 1.0).abs() < 0.01);
        assert!(r.abs() < 0.01);

        let (l, r) = stereo_gains_bipolar(1.0, 1.0);
        assert!(l.abs() < 0.01);
        assert!((r - 1.0).abs() < 0.01);
    }

    #[test]
    fn stereo_source_keeps_sides_apart() {
        let mut l = 0.0;
        let mut r = 0.0;
        mix_stereo_source(1.0, -1.0, 1.0, 1.0, &mut l, &mut r);
        assert_eq!(l, 1.0);
        assert_eq!(r, -1.0);
    }

    #[test]
    fn soft_clip_is_transparent_below_threshold() {
        assert_eq!(soft_clip(0.0), 0.0);
        assert_eq!(soft_clip(0.5), 0.5);
        assert_eq!(soft_clip(-0.5), -0.5);
    }

    #[test]
    fn soft_clip_stays_inside_unity() {
        for x in [1.0f32, 2.0, 8.0, -1.0, -2.0, -8.0] {
            assert!(soft_clip(x).abs() <= 1.0, "estourou em {}", x);
        }
        // Monotônico: mais sinal continua soando mais alto.
        assert!(soft_clip(1.5) > soft_clip(1.0));
        assert!(soft_clip(1.0) > soft_clip(0.7));
    }
}
