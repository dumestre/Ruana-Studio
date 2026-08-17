fn main() {
    let ui_file =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/main.slint");
    slint_build::compile(ui_file).expect("failed to compile Slint UI");

    build_cursors().expect("failed to build cursors");
    
    // Copy cursors to target directory for runtime access
    copy_cursors_to_target().ok();
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
        let src = entry.path();
        let dst = dst_dir.join(entry.file_name());
        std::fs::copy(&src, &dst)?;
    }
    
    Ok(())
}

fn build_cursors() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let icons_dir = manifest_dir.join("../ui/icons");
    let out_dir = manifest_dir.join("cursors");

    std::fs::create_dir_all(&out_dir)?;

    let cursors = [
        ("select.svg", "select.cur", (7, 7)),
        ("slice.svg", "slice.cur", (12, 12)),
        ("glue.svg", "glue.cur", (12, 12)),
        ("stretch.svg", "stretch.cur", (12, 12)),
        ("draw.svg", "draw.cur", (12, 12)),
        ("erase.svg", "erase.cur", (12, 12)),
    ];

    for (svg_name, cur_name, hotspot) in cursors {
        let svg_path = icons_dir.join(svg_name);
        let cur_path = out_dir.join(cur_name);

        if !svg_path.exists() {
            eprintln!("Warning: SVG not found: {}", svg_path.display());
            continue;
        }

        let png_bytes = render_svg_to_png(&svg_path, 24, 24)?;
        write_cur_file(&cur_path, &png_bytes, hotspot)?;

        println!("cargo:rerun-if-changed={}", svg_path.display());
        println!("Generated: {}", cur_path.display());
    }

    Ok(())
}

fn render_svg_to_png(
    svg_path: &std::path::Path,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use resvg::render;
    use resvg::tiny_skia::{Pixmap, Transform};
    use usvg::{Options, Tree};

    let svg_data = std::fs::read(svg_path)?;
    let opt = Options::default();
    let tree = Tree::from_data(&svg_data, &opt)?;

    let mut pixmap = Pixmap::new(width, height).ok_or("Failed to create pixmap")?;
    render(&tree, Transform::default(), &mut pixmap.as_mut());

    let png_bytes = pixmap.encode_png()?;
    Ok(png_bytes)
}

fn write_cur_file(
    path: &std::path::Path,
    png_data: &[u8],
    hotspot: (i32, i32),
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    // Decode PNG first
    let mut decoder = png::Decoder::new(png_data);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    let width = info.width;
    let height = info.height;

    let mut image_data = vec![0; reader.output_buffer_size()];
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
    file.write_all(&0u8.to_le_bytes())?;    // color count (0 = no palette)
    file.write_all(&0u8.to_le_bytes())?;    // reserved
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