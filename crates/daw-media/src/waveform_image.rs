//! Renderização da waveform em imagem (PNG em memória).
//!
//! Em vez de desenhar 1 retângulo por pico no Slint (até 1024
//! por clip, todo frame), o Rust rasteriza a waveform UMA vez
//! com tiny-skia e a UI só desenha um `Image`. Repaint da
//! timeline fica praticamente gratuito.
//!
//! A imagem é desenhada na cor do clip: como a cor muda via
//! ciclo de paleta, re-renderizamos quando `color_index` difere.

use tiny_skia::*;

/// Largura fixa do cache (px). A UI estica para a largura real
/// do clip — 1024 cobre clips largos sem serrilhar.
pub const WAVEFORM_IMG_WIDTH: usize = 1024;
const WAVEFORM_IMG_HEIGHT: usize = 256;

/// Converte um índice de cor da paleta em RGBA. Deve refletir
/// ClipPalette.colors + os padrões MIDI/áudio da UI.
///
/// Mantido em sincronia com ui/clip_palette.slint.
pub fn palette_rgba(color_index: i32, is_midi: bool) -> [u8; 4] {
    // Padrões (índice 0): verde áudio / roxo MIDI — mesmos da UI.
    const PALETTE: [[u8; 3]; 20] = [
        [0x00, 0xC8, 0x53], // 0 = padrão
        [0xFF, 0x52, 0x52],
        [0x4E, 0x9A, 0xFF],
        [0xFF, 0xB8, 0x30],
        [0xB4, 0x6B, 0xFF],
        [0x2E, 0xD6, 0xCA],
        [0xFF, 0x7A, 0xB2],
        [0xA5, 0xD6, 0x39],
        [0x63, 0x7B, 0xFF],
        [0xE8, 0x54, 0x37],
        [0x00, 0xBC, 0xD4],
        [0xC0, 0xCA, 0x33],
        [0xAB, 0x47, 0xBC],
        [0x26, 0xA6, 0x9A],
        [0xEF, 0x6C, 0x00],
        [0x42, 0xA5, 0xF5],
        [0xEC, 0x40, 0x7A],
        [0x9C, 0xCC, 0x65],
        [0x5C, 0x6B, 0xC0],
        [0x8D, 0x6E, 0x63],
    ];
    let idx = if color_index == 0 {
        if is_midi {
            return [0x9C, 0x64, 0xFF, 255]; // roxo MIDI padrão
        }
        0
    } else {
        (color_index as usize) % PALETTE.len()
    };
    let c = PALETTE[idx];
    [c[0], c[1], c[2], 255]
}

/// Rasteriza os picos numa waveform RGBA (corpo RMS já embutido
/// nos valores, mesma escala 1:1 do renderer de fallback).
pub fn waveform_image_rgba(
    peaks: &[f32],
    color_index: i32,
    is_midi: bool,
) -> Option<Vec<u8>> {
    if peaks.is_empty() {
        return None;
    }

    let w = WAVEFORM_IMG_WIDTH as u32;
    let h = WAVEFORM_IMG_HEIGHT as u32;
    let mut pixmap = Pixmap::new(w, h)?;

    // Waveform sempre PRETA (#0A0A0A) sobre o corpo translúcido
    // do clip — padrão de DAW. O índice de cor não afeta mais o
    // traço da waveform, só o header/borda.
    let mut paint = Paint::default();
    paint.set_color_rgba8(0x0A, 0x0A, 0x0A, 255);
    paint.anti_alias = false;

    let n = peaks.len();
    let bar_w = w as f32 / n as f32;

    for (i, &p) in peaks.iter().enumerate() {
        if p <= 0.004 {
            continue; // silêncio não desenha nada (fiel ao áudio)
        }
        let bar_h = (p * h as f32).round().max(1.0);
        let y = ((h as f32 - bar_h) / 2.0).floor();
        let x = (i as f32 * bar_w).floor();

        let rect = Rect::from_xywh(
            x,
            y,
            bar_w.ceil().max(1.0),
            bar_h,
        )?;

        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }

    Some(pixmap.data().to_vec())
}
