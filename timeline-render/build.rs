// Gera os cursores (.cur) a partir dos SVGs em ../app/cursors/svgcursors,
// usando o mesmo pipeline do app/build.rs (render_svg_to_png + write_cur_file).
// Saída: timeline-render/cursors (e cópia para target/<profile>/cursors).
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

fn main() {
    // Para não bloquear o build em ambiente sem a pasta de SVGs.
    if let Err(e) = build_cursors() {
        eprintln!("cargo:warning=timeline-render: cursores nao gerados ({e})");
    }
    if let Err(e) = build_icons() {
        eprintln!("cargo:warning=timeline-render: atlas de icones nao gerado ({e})");
    }
}

fn build_cursors() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let svg_dir = manifest_dir.join("../app/cursors/svgcursors");
    let out_dir = manifest_dir.join("cursors");
    std::fs::create_dir_all(&out_dir)?;

    let cursors = [
        ("selectcursor.svg", "select.cur", (7, 8)),
        ("drawcursor.svg", "draw.cur", (16, 16)),
        ("erasecursor.svg", "erase.cur", (16, 16)),
        ("gluecursor.svg", "glue.cur", (16, 16)),
        ("slicecursor.svg", "slice.cur", (16, 16)),
        ("stretchcursor.svg", "stretch.cur", (16, 16)),
        ("gain.svg", "gain.cur", (16, 16)),
        ("hand.svg", "hand.cur", (16, 16)),
    ];

    for (svg_name, cur_name, hotspot) in cursors {
        let svg_path = svg_dir.join(svg_name);
        if !svg_path.exists() {
            println!("cargo:rerun-if-changed={}", svg_path.display());
            continue;
        }
        println!("cargo:rerun-if-changed={}", svg_path.display());

        let png_bytes = render_svg_to_png(&svg_path, 32, 32, 0.55)?;
        let cur_path = out_dir.join(cur_name);
        write_cur_file(&cur_path, &png_bytes, hotspot, false)?;

        // Variante escura (modo Paper): inverte a cor, preservando a forma.
        let dark_name = cur_name.replace(".cur", "-dark.cur");
        write_cur_file(&out_dir.join(&dark_name), &png_bytes, hotspot, true)?;

        println!("Generated: {}", cur_path.display());
    }

    copy_cursors_to_target(&out_dir)?;
    Ok(())
}

fn copy_cursors_to_target(out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let target_dir = std::env::var("OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| out_dir.to_path_buf())
        .ancestors()
        .nth(3) // target/debug/build/timeline-render-<hash>/out
        .map(|p| p.to_path_buf());

    if let Some(target_dir) = target_dir {
        let dst_dir = target_dir.join("cursors");
        std::fs::create_dir_all(&dst_dir)?;
        for entry in std::fs::read_dir(out_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                continue;
            }
            let src = entry.path();
            let dst = dst_dir.join(entry.file_name());
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

fn render_svg_to_png(
    svg_path: &Path,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use resvg::render;
    use resvg::tiny_skia::{Pixmap, Transform};
    use usvg::{Options, Tree};

    let svg_data = std::fs::read(svg_path)?;
    let opt = Options::default();
    let tree = Tree::from_data(&svg_data, &opt)?;

    let art_w = tree.size().width().max(0.001) as f32;
    let art_h = tree.size().height().max(0.001) as f32;

    let offset_x = (width as f32 * (1.0 - scale)) / 2.0;
    let offset_y = (height as f32 * (1.0 - scale)) / 2.0;
    let transform = Transform::from_translate(offset_x, offset_y)
        .pre_scale(width as f32 * scale / art_w, height as f32 * scale / art_h);

    let mut pixmap = Pixmap::new(width, height).ok_or("Failed to create pixmap")?;
    render(&tree, transform, &mut pixmap.as_mut());

    let png_bytes = pixmap.encode_png()?;
    Ok(png_bytes)
}

fn write_cur_file(
    path: &Path,
    png_data: &[u8],
    hotspot: (i32, i32),
    invert: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut decoder = png::Decoder::new(Cursor::new(png_data));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    let width = info.width;
    let height = info.height;

    let mut image_data = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut image_data)?;

    let mut file = std::fs::File::create(path)?;

    // ICONDIR header (6 bytes)
    file.write_all(&0u16.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;

    // ICONDIRENTRY (16 bytes)
    file.write_all(&(width as u8).to_le_bytes())?;
    file.write_all(&(height as u8).to_le_bytes())?;
    file.write_all(&0u8.to_le_bytes())?;
    file.write_all(&0u8.to_le_bytes())?;
    file.write_all(&(hotspot.0 as u16).to_le_bytes())?;
    file.write_all(&(hotspot.1 as u16).to_le_bytes())?;

    let pixel_data_size = width * height * 4;
    let and_mask_size = ((width + 7) / 8) * height;
    let dib_size = 40 + pixel_data_size + and_mask_size;

    file.write_all(&(dib_size as u32).to_le_bytes())?;
    file.write_all(&22u32.to_le_bytes())?;

    // BITMAPINFOHEADER (40 bytes)
    file.write_all(&40u32.to_le_bytes())?;
    file.write_all(&(width as i32).to_le_bytes())?;
    file.write_all(&((height * 2) as i32).to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&32u16.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&(pixel_data_size as u32).to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;

    // XOR mask (BGRA premultiplied, bottom-up)
    for y in (0..height).rev() {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            if idx + 3 < image_data.len() {
                let b = image_data[idx];
                let g = image_data[idx + 1];
                let r = image_data[idx + 2];
                let a = image_data[idx + 3];
                let (r, g, b) = if invert {
                    (255 - r, 255 - g, 255 - b)
                } else {
                    (r, g, b)
                };
                let r = ((r as u16 * a as u16) / 255) as u8;
                let g = ((g as u16 * a as u16) / 255) as u8;
                let b = ((b as u16 * a as u16) / 255) as u8;
                file.write_all(&[b, g, r, a])?;
            } else {
                file.write_all(&[0, 0, 0, 0])?;
            }
        }
    }

    // AND mask (1-bit por pixel, linha alinhada a 32 bits)
    let stride = (((width + 31) / 32) * 4) as usize;
    for y in 0..height {
        let mut row = vec![0u8; stride];
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            if idx + 3 < image_data.len() {
                let a = image_data[idx + 3];
                if a == 0 {
                    let byte_idx = (x / 8) as usize;
                    let bit_idx = 7 - (x % 8);
                    row[byte_idx] |= 1 << bit_idx;
                }
            }
        }
        file.write_all(&row)?;
    }

    Ok(())
}

// ============================================================================
// Atlas de ícones da edit toolbar
// ============================================================================
// Rasteriza os 12 SVGs da toolbar (6 normais + 6 "filled") em 16x16 RGBA e
// monta um atlas 4x3, emitindo um módulo Rust em $OUT_DIR/icons_atlas.rs com
// os pixels (alpha straight). O código usa `include!` via env!("OUT_DIR").

/// Rasteriza um SVG em RGBA straight (des-premultiplicando o alpha do
/// tiny_skia, que é premultiplied), pronto para textura wgpu.
fn render_svg_to_rgba(
    svg_path: &Path,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use resvg::render;
    use resvg::tiny_skia::{Pixmap, Transform};
    use usvg::{Options, Tree};

    let svg_data = std::fs::read(svg_path)?;
    let opt = Options::default();
    let tree = Tree::from_data(&svg_data, &opt)?;

    let art_w = tree.size().width().max(0.001) as f32;
    let art_h = tree.size().height().max(0.001) as f32;

    let scale = (width as f32 / art_w).min(height as f32 / art_h);
    let offset_x = (width as f32 - art_w * scale) / 2.0;
    let offset_y = (height as f32 - art_h * scale) / 2.0;
    let transform = Transform::from_translate(offset_x, offset_y).pre_scale(scale, scale);

    let mut pixmap = Pixmap::new(width, height).ok_or("Failed to create pixmap")?;
    render(&tree, transform, &mut pixmap.as_mut());

    let premul = pixmap.data();
    let mut out = vec![0u8; (width * height * 4) as usize];
    for (i, px) in premul.chunks_exact(4).enumerate() {
        let a = px[3] as u16;
        if a == 0 {
            continue;
        }
        out[i * 4] = ((px[0] as u16 * 255 / a).min(255)) as u8;
        out[i * 4 + 1] = ((px[1] as u16 * 255 / a).min(255)) as u8;
        out[i * 4 + 2] = ((px[2] as u16 * 255 / a).min(255)) as u8;
        out[i * 4 + 3] = px[3];
    }
    Ok(out)
}

fn build_icons() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let svg_dir = manifest_dir.join("../ui/icons");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);

    // Mesma ordem do EditToolbar no Slint: cada entrada é o par (normal, filled)
    // e o índice no atlas é a ordem desta lista.
    let icons = [
        ("select.svg", "select_filled.svg"),
        ("slice.svg", "slice_filled.svg"),
        ("glue.svg", "glue_filled.svg"),
        ("expand_diag.svg", "stretch_filled.svg"),
        ("draw.svg", "draw_filled.svg"),
        ("erase.svg", "erase_filled.svg"),
    ];

    const CELL: u32 = 16;
    const COLS: u32 = 4;
    let rows = ((icons.len() * 2) as u32 + COLS - 1) / COLS; // 12 ícones → 3 linhas
    let w = CELL * COLS;
    let h = CELL * rows;
    let mut atlas = vec![0u8; (w * h * 4) as usize];

    let mut emitted = false;
    for (pair_idx, (normal_name, filled_name)) in icons.iter().enumerate() {
        for (offset, name) in [*normal_name, *filled_name].iter().enumerate() {
            let svg_path = svg_dir.join(name);
            println!("cargo:rerun-if-changed={}", svg_path.display());
            if !svg_path.exists() {
                continue;
            }
            let rgba = render_svg_to_rgba(&svg_path, CELL, CELL)?;
            let idx = (pair_idx * 2 + offset) as u32;
            let col = idx % COLS;
            let row = idx / COLS;
            let ox = (col * CELL) as usize * 4;
            let oy = (row * CELL) as usize * w as usize * 4;
            for y in 0..CELL as usize {
                let src = y * (CELL as usize) * 4;
                let dst = oy + y * (w as usize) * 4 + ox;
                atlas[dst..dst + (CELL as usize) * 4]
                    .copy_from_slice(&rgba[src..src + (CELL as usize) * 4]);
            }
            emitted = true;
        }
    }

    // Gera $OUT_DIR/icons_atlas.rs
    let mut code = String::new();
    code.push_str(&format!("pub const ICON_ATLAS_W: u32 = {w};\n"));
    code.push_str(&format!("pub const ICON_ATLAS_H: u32 = {h};\n"));
    code.push_str("pub const ICON_CELL: u32 = 16;\n");
    code.push_str(&format!("pub const ICON_COLS: u32 = {COLS};\n"));
    code.push_str("pub const ICON_ATLAS_RGBA: &[u8] = &[\n");
    if emitted {
        for chunk in atlas.chunks(16) {
            let s: Vec<String> = chunk.iter().map(|b| format!("{b},")).collect();
            code.push_str("    ");
            code.push_str(&s.join(""));
            code.push('\n');
        }
    }
    code.push_str("];\n");

    let out_path = out_dir.join("icons_atlas.rs");
    std::fs::write(&out_path, code)?;
    println!("Generated: {}", out_path.display());
    Ok(())
}
