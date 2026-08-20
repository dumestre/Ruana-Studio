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
    if any_soloed {
        soloed
    } else {
        !muted
    }
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

/// Clamp stereo frame para [-1.0, 1.0].
#[inline]
pub fn clamp_stereo(left: f32, right: f32) -> (f32, f32) {
    (left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0))
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
}
