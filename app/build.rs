fn main() {
    let ui_file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/main.slint");
    slint_build::compile(ui_file).expect("failed to compile Slint UI");

    build_cursors().expect("failed to build cursors");

    // Copy cursors to target directory for runtime access
    copy_cursors_to_target().ok();

    build_app_icon().expect("failed to build app icon");
}

/// Tamanhos gerados para o ícone do app (PNGs individuais + .ico multi-size).
#[cfg(windows)]
const APP_ICON_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// Renderiza assets/RuanaStudio.svg em PNGs (OUT_DIR) e num .ico multi-size.
/// O .ico é embutido no executável (ícone do Explorer); o PNG de 64px é
/// incluído no binário (include_bytes!) e aplicado às janelas em runtime.
///
/// Se o usuário fornecer arquivos próprios, eles têm prioridade:
///   - assets/ruana-studio.ico  → embutido no .exe (ICO multi-size)
///   - assets/app_icon_64.png   → ícone de runtime das janelas (PNG RGBA)
fn build_app_icon() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let svg_path = manifest_dir.join("assets/RuanaStudio.svg");
    let custom_png = manifest_dir.join("assets/app_icon_64.png");
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);

    println!("cargo:rerun-if-changed={}", svg_path.display());

    // PNG de runtime (64px): usa o fornecido pelo usuário, senão renderiza o SVG.
    if custom_png.exists() {
        println!("cargo:rerun-if-changed={}", custom_png.display());
        std::fs::copy(&custom_png, out_dir.join("app_icon_64.png"))?;
    } else {
        std::fs::write(
            out_dir.join("app_icon_64.png"),
            render_svg_to_png(&svg_path, 64, 64, 1.0)?,
        )?;
    }

    // .ico do executável: usa o fornecido pelo usuário, senão gera do SVG.
    // Apenas no Windows — no Linux o ícone de runtime (PNG) é suficiente.
    #[cfg(windows)]
    {
        let custom_ico = manifest_dir.join("assets/ruana-studio.ico");
        let ico_path = if custom_ico.exists() {
            println!("cargo:rerun-if-changed={}", custom_ico.display());
            let _ = std::fs::copy(&custom_ico, out_dir.join("ruana-studio.ico"))?;
            out_dir.join("ruana-studio.ico")
        } else {
            let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
            for &size in &APP_ICON_SIZES {
                let png_bytes = render_svg_to_png(&svg_path, size, size, 1.0)?;
                entries.push((size, png_bytes));
            }
            let generated = out_dir.join("ruana-studio.ico");
            write_bmp_ico(&generated, &entries)?;
            generated
        };

        // Embute o .ico no executável. Falha graciosamente se rc.exe/windres
        // não estiverem disponíveis (o ícone de runtime das janelas continua).
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(ico_path.to_str().expect("ico path is not valid UTF-8"));
        if let Err(e) = resource.compile() {
            eprintln!("cargo:warning=Icone nao embutido no .exe ({e}); usando apenas icone de runtime");
        }
    }

    Ok(())
}

fn copy_cursors_to_target() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("cursors");

    // Get the target directory from OUT_DIR
    let target_dir = std::env::var("OUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target/debug"))
        .ancestors()
        .nth(3) // OUT_DIR is like target/debug/build/ruana-studio-<hash>/out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| manifest_dir.join("target/debug"));

    let dst_dir = target_dir.join("cursors");
    std::fs::create_dir_all(&dst_dir)?;

    for entry in std::fs::read_dir(&src_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            continue;
        }
        let src = entry.path();
        let dst = dst_dir.join(entry.file_name());
        std::fs::copy(&src, &dst)?;
    }

    Ok(())
}

fn build_cursors() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let svg_dir = manifest_dir.join("cursors/svgcursors");
    let out_dir = manifest_dir.join("cursors");

    std::fs::create_dir_all(&out_dir)?;

    let cursors = [
        ("selectcursor.svg", "select.cur", (7, 8)),
        ("drawcursor.svg", "draw.cur", (16, 16)),
        ("erasecursor.svg", "erase.cur", (16, 16)),
        ("gluecursor.svg", "glue.cur", (16, 16)),
        ("slicecursor.svg", "slice.cur", (16, 16)),
        ("stretchcursor.svg", "stretch.cur", (16, 16)),
    ];

    for (svg_name, cur_name, hotspot) in cursors {
        let svg_path = svg_dir.join(svg_name);
        let cur_path = out_dir.join(cur_name);

        if !svg_path.exists() {
            eprintln!("Warning: SVG not found: {}", svg_path.display());
            continue;
        }

        let png_bytes = render_svg_to_png(&svg_path, 32, 32, 0.55)?;
        write_cur_file(&cur_path, &png_bytes, hotspot, false)?;

        // Variante escura (modo Paper): inverte a cor, preservando a forma.
        let dark_name = cur_name.replace(".cur", "-dark.cur");
        let dark_path = out_dir.join(&dark_name);
        write_cur_file(&dark_path, &png_bytes, hotspot, true)?;

        println!("cargo:rerun-if-changed={}", svg_path.display());
        println!("Generated: {}", cur_path.display());
    }

    println!("cargo:rerun-if-changed={}", svg_dir.display());

    Ok(())
}

fn render_svg_to_png(
    svg_path: &std::path::Path,
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

    // `scale` = fração do alvo (width×height) que a arte deve ocupar.
    // A arte do SVG pode ter qualquer tamanho; normalizamos pelo tamanho
    // real do SVG para caber no pixmap (ex: RuanaStudio.svg é 512×512).
    let art_w = tree.size().width().max(0.001) as f32;
    let art_h = tree.size().height().max(0.001) as f32;

    let offset_x = (width as f32 * (1.0 - scale)) / 2.0;
    let offset_y = (height as f32 * (1.0 - scale)) / 2.0;
    let transform = Transform::from_translate(offset_x, offset_y).pre_scale(
        width as f32 * scale / art_w,
        height as f32 * scale / art_h,
    );

    let mut pixmap = Pixmap::new(width, height).ok_or("Failed to create pixmap")?;
    render(&tree, transform, &mut pixmap.as_mut());

    let png_bytes = pixmap.encode_png()?;
    Ok(png_bytes)
}

/// Decodifica um PNG (RGBA 8-bit) para `(width, height, rgba)`.
#[cfg(windows)]
fn decode_png_rgba(png_bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), Box<dyn std::error::Error>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut data)?;
    let info = reader.info();
    Ok((info.width, info.height, data))
}

/// Escreve um .ico multi-size com entradas BMP clássicas (32-bit BGRA
/// com alpha premultiplicado + máscara AND) — formato aceito por todo
/// o shell do Windows, sem depender de decodificação de PNG embutido.
#[cfg(windows)]
fn write_bmp_ico(
    path: &std::path::Path,
    entries: &[(u32, Vec<u8>)],
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    // Decodifica todos os PNGs antes de calcular offsets.
    let decoded: Vec<(u32, u32, Vec<u8>)> = entries
        .iter()
        .map(|(size, png)| {
            let (w, h, rgba) = decode_png_rgba(png)?;
            debug_assert_eq!((w, h), (*size, *size));
            Ok((w, h, rgba))
        })
        .collect::<Result<_, Box<dyn std::error::Error>>>()?;

    let mut file = std::fs::File::create(path)?;

    // ICONDIR (6 bytes)
    file.write_all(&0u16.to_le_bytes())?; // reserved
    file.write_all(&1u16.to_le_bytes())?; // type: 1 = icon
    file.write_all(&(decoded.len() as u16).to_le_bytes())?;

    // ICONDIRENTRY (16 bytes cada) — offsets calculados sequencialmente.
    let mut data_offset = 6u32 + 16 * decoded.len() as u32;
    for &(w, h, _) in &decoded {
        let dim = if w >= 256 { 0 } else { w as u8 };
        file.write_all(&[dim])?; // width
        file.write_all(&[if h >= 256 { 0 } else { h as u8 }])?; // height
        file.write_all(&[0])?; // color count
        file.write_all(&[0])?; // reserved
        file.write_all(&1u16.to_le_bytes())?; // planes
        file.write_all(&32u16.to_le_bytes())?; // bpp
        let pixel_size = w * h * 4;
        let mask_stride = (((w + 31) / 32) * 4) as usize;
        let dib_size = 40 + pixel_size + mask_stride as u32 * h;
        file.write_all(&dib_size.to_le_bytes())?;
        file.write_all(&data_offset.to_le_bytes())?;
        data_offset += dib_size;
    }

    // Imagens: BITMAPINFOHEADER + XOR (BGRA premultiplied, bottom-up) + AND mask.
    for &(w, h, ref rgba) in &decoded {
        let pixel_size = w * h * 4;
        file.write_all(&40u32.to_le_bytes())?; // biSize
        file.write_all(&(w as i32).to_le_bytes())?; // biWidth
        file.write_all(&((h * 2) as i32).to_le_bytes())?; // biHeight (XOR + AND)
        file.write_all(&1u16.to_le_bytes())?; // biPlanes
        file.write_all(&32u16.to_le_bytes())?; // biBitCount
        file.write_all(&0u32.to_le_bytes())?; // biCompression (BI_RGB)
        file.write_all(&pixel_size.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?; // biXPelsPerMeter
        file.write_all(&0u32.to_le_bytes())?; // biYPelsPerMeter
        file.write_all(&0u32.to_le_bytes())?; // biClrUsed
        file.write_all(&0u32.to_le_bytes())?; // biClrImportant

        for y in (0..h).rev() {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                if idx + 3 < rgba.len() {
                    let r = rgba[idx] as u16;
                    let g = rgba[idx + 1] as u16;
                    let b = rgba[idx + 2] as u16;
                    let a = rgba[idx + 3];
                    file.write_all(&[
                        ((b * a as u16) / 255) as u8,
                        ((g * a as u16) / 255) as u8,
                        ((r * a as u16) / 255) as u8,
                        a,
                    ])?;
                } else {
                    file.write_all(&[0, 0, 0, 0])?;
                }
            }
        }

        let stride = (((w + 31) / 32) * 4) as usize;
        for y in 0..h {
            let mut row = vec![0u8; stride];
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                if idx + 3 < rgba.len() && rgba[idx + 3] == 0 {
                    row[(x / 8) as usize] |= 1 << (7 - x % 8);
                }
            }
            file.write_all(&row)?;
        }
    }

    Ok(())
}

fn write_cur_file(
    path: &std::path::Path,
    png_data: &[u8],
    hotspot: (i32, i32),
    invert: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    // Decode PNG first
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_data));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    let width = info.width;
    let height = info.height;

    let mut image_data = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut image_data)?;

    // Write proper Windows .cur format (based on ICO format)
    // .cur is essentially an ICO file with type=2 and hotspot in the directory entry

    let mut file = std::fs::File::create(path)?;

    // ICONDIR header (6 bytes)
    file.write_all(&0u16.to_le_bytes())?; // reserved
    file.write_all(&2u16.to_le_bytes())?; // type: 2 = cursor
    file.write_all(&1u16.to_le_bytes())?; // count: 1 image

    // ICONDIRENTRY (16 bytes)
    file.write_all(&(width as u8).to_le_bytes())?; // width
    file.write_all(&(height as u8).to_le_bytes())?; // height
    file.write_all(&0u8.to_le_bytes())?; // color count (0 = no palette)
    file.write_all(&0u8.to_le_bytes())?; // reserved
    file.write_all(&(hotspot.0 as u16).to_le_bytes())?; // hotspot X
    file.write_all(&(hotspot.1 as u16).to_le_bytes())?; // hotspot Y

    // Calculate image size: BITMAPINFOHEADER (40) + pixel data (w*h*4) + AND mask (w*h/8)
    let pixel_data_size = width * height * 4;
    let and_mask_size = ((width + 7) / 8) * height; // 1 bit per pixel, padded to byte
    let dib_size = 40 + pixel_data_size + and_mask_size;

    file.write_all(&(dib_size as u32).to_le_bytes())?; // bytes in resource
    file.write_all(&22u32.to_le_bytes())?; // image offset (6 + 16 = 22)

    // BITMAPINFOHEADER (40 bytes)
    file.write_all(&40u32.to_le_bytes())?; // biSize
    file.write_all(&(width as i32).to_le_bytes())?; // biWidth
    file.write_all(&((height * 2) as i32).to_le_bytes())?; // biHeight (double for XOR + AND)
    file.write_all(&1u16.to_le_bytes())?; // biPlanes
    file.write_all(&32u16.to_le_bytes())?; // biBitCount
    file.write_all(&0u32.to_le_bytes())?; // biCompression (BI_RGB)
    file.write_all(&(pixel_data_size as u32).to_le_bytes())?; // biSizeImage
    file.write_all(&0u32.to_le_bytes())?; // biXPelsPerMeter
    file.write_all(&0u32.to_le_bytes())?; // biYPelsPerMeter
    file.write_all(&0u32.to_le_bytes())?; // biClrUsed
    file.write_all(&0u32.to_le_bytes())?; // biClrImportant

    // XOR mask (color data) - bottom-up, 32-bit BGRA with premultiplied alpha
    for y in (0..height).rev() {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            if idx + 3 < image_data.len() {
                let b = image_data[idx];
                let g = image_data[idx + 1];
                let r = image_data[idx + 2];
                let a = image_data[idx + 3];
                // Variante escura: inverte a cor antes do premultiply.
                let (r, g, b) = if invert {
                    (255 - r, 255 - g, 255 - b)
                } else {
                    (r, g, b)
                };
                // Premultiply alpha
                let r = ((r as u16 * a as u16) / 255) as u8;
                let g = ((g as u16 * a as u16) / 255) as u8;
                let b = ((b as u16 * a as u16) / 255) as u8;
                file.write_all(&[b, g, r, a])?;
            } else {
                file.write_all(&[0, 0, 0, 0])?;
            }
        }
    }

    // AND mask (1-bit per pixel, padded to 32-bit boundary per row)
    let stride = (((width + 31) / 32) * 4) as usize; // 4 bytes per row
    for y in 0..height {
        let mut row = vec![0u8; stride];
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            if idx + 3 < image_data.len() {
                let a = image_data[idx + 3];
                if a == 0 {
                    // Transparent
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