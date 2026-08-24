//! Renderização da waveform em imagem (RGBA em memória).
//!
//! Em vez de desenhar 1 retângulo por pico no Slint (até 2048
//! por clip, todo frame), o Rust rasteriza a waveform UMA vez
//! com tiny-skia e a UI desenha um único `Image`.
//!
//! QUALIDADE NO ZOOM: `waveform_image_rgba_at` agrega os picos
//! por COLUNA DE PIXEL da largura pedida e traça um envelope
//! contínuo simétrico. A UI pede re-render na largura real de
//! exibição quando ela muda — igual fazem as DAWs.

use tiny_skia::*;

/// Largura do cache padrão (import). Re-renders de zoom usam a
/// largura exata pedida pela UI.
pub const WAVEFORM_IMG_WIDTH: usize = 2048;

/// Altura do cache. A UI estica verticalmente (a curva é
/// simétrica, então esticar não distorce a forma).
const H: u32 = 512;

/// Rasteriza os picos num envelope contínuo, largura `w` px.
///
/// Os valores já chegam como amplitude real 0..1 (RMS + crista,
/// sem normalização por arquivo): áudio baixo desenha baixo.
pub fn waveform_image_rgba_at(peaks: &[f32], w: u32) -> Option<Vec<u8>> {
    if peaks.is_empty() || w == 0 {
        return None;
    }
    let h = H;

    let mut pixmap = Pixmap::new(w, h)?;
    let center = h as f32 / 2.0;
    let n = peaks.len() as f32;

    let mut paint = Paint::default();
    paint.set_color_rgba8(0x0A, 0x0A, 0x0A, 255);
    paint.anti_alias = true;

    // Amplitude máxima das colunas [i0..i1) de picos que caem no pixel x.
    let amp_at = |x: usize| -> f32 {
        let i0 = ((x as f32 / w as f32) * n).floor() as usize;
        let i1 = ((((x + 1) as f32 / w as f32) * n).ceil() as usize).max(i0 + 1);
        peaks[i0.min(peaks.len() - 1)..i1.min(peaks.len())]
            .iter()
            .fold(0.0f32, |a, &p| a.max(p))
    };

    let mut pb = PathBuilder::new();

    // Contorno superior, esquerda → direita.
    for x in 0..w {
        let half = (amp_at(x as usize).clamp(0.0, 1.0) * center).max(1.0);
        if x == 0 {
            pb.move_to(0.0, center - half);
        } else {
            pb.line_to(x as f32, center - half);
        }
    }
    // Contorno inferior espelhado, direita → esquerda.
    for x in (0..w).rev() {
        let half = (amp_at(x as usize).clamp(0.0, 1.0) * center).max(1.0);
        pb.line_to(x as f32, center + half);
    }
    pb.close();

    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    Some(pixmap.data().to_vec())
}

/// Compatibilidade: rasteriza na largura padrão.
pub fn waveform_image_rgba(peaks: &[f32], _color_index: i32, _is_midi: bool) -> Option<Vec<u8>> {
    waveform_image_rgba_at(peaks, WAVEFORM_IMG_WIDTH as u32)
}
