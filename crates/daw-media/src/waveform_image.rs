//! Renderização da waveform em imagem (RGBA em memória).
//!
//! Em vez de desenhar 1 retângulo por pico no Slint (até 2048
//! por clip, todo frame), o Rust rasteriza a waveform UMA vez
//! com tiny-skia e a UI desenha um único `Image`.
//!
//! QUALIDADE: para cada COLUNA DE PIXEL da imagem agregamos os
//! picos que caem nela e desenhamos o envelope completo
//! (simétrico em volta do centro). A resolução horizontal é
//! então a própria largura da imagem — zoom não serrilha e
//! nada vira "tijolo quadrado". Anti-alias suaviza as bordas.

use tiny_skia::*;

/// Largura do cache (px). Cada coluna é um sample visual do
/// envelope — 2048 colunas cobrem qualquer zoom com folga.
pub const WAVEFORM_IMG_WIDTH: usize = 2048;

/// Altura do cache (px). A UI estica para a altura do corpo do
/// clip; 512 mantém o arquivo pequeno e a curva suave.
const H: u32 = 512;

/// Rasteriza os picos numa imagem RGBA (premultiplied, layout
/// BGRA em memória — formato nativo do tiny-skia).
///
/// Os valores já chegam como amplitude real 0..1 (RMS + crista,
/// sem normalização por arquivo): áudio baixo desenha baixo.
pub fn waveform_image_rgba(peaks: &[f32], _color_index: i32, _is_midi: bool) -> Option<Vec<u8>> {
    if peaks.is_empty() {
        return None;
    }

    let w = WAVEFORM_IMG_WIDTH as u32;
    let mut pixmap = Pixmap::new(w, H)?;
    let center = H as f32 / 2.0;

    let mut paint = Paint::default();
    paint.set_color_rgba8(0x0A, 0x0A, 0x0A, 255);
    paint.anti_alias = true;

    let n = peaks.len() as f32;
    let mut pb = PathBuilder::new();

    // Contorno superior do envelope, esquerda → direita.
    for x in 0..w as usize {
        // Faixa de picos que esta coluna representa.
        let i0 = ((x as f32 / w as f32) * n).floor() as usize;
        let i1 = ((((x + 1) as f32 / w as f32) * n).ceil() as usize).max(i0 + 1);
        let amp = peaks[i0.min(peaks.len() - 1)..i1.min(peaks.len())]
            .iter()
            .fold(0.0f32, |a, &p| a.max(p));

        let half = (amp.clamp(0.0, 1.0) * center).max(1.0);
        let y = center - half;
        if x == 0 {
            pb.move_to(0.0, y);
        } else {
            pb.line_to(x as f32, y);
        }
    }
    // Contorno inferior, direita → esquerda (espelho).
    for x in (0..w as usize).rev() {
        let i0 = ((x as f32 / w as f32) * n).floor() as usize;
        let i1 = ((((x + 1) as f32 / w as f32) * n).ceil() as usize).max(i0 + 1);
        let amp = peaks[i0.min(peaks.len() - 1)..i1.min(peaks.len())]
            .iter()
            .fold(0.0f32, |a, &p| a.max(p));
        let half = (amp.clamp(0.0, 1.0) * center).max(1.0);
        pb.line_to(x as f32, center + half);
    }
    pb.close();

    if let Some(path) = pb.finish() {
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    Some(pixmap.data().to_vec())
}
