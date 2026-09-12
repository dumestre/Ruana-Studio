//! Rasteriza os ícones SVG do projeto (ui/icons/*.svg) e o logo
//! (app/assets/RuanaStudio.svg) em PNG (2x, nítido em DPI alto) para o
//! app Makepad consumir via `Image`.
//!
//! O makepad-widgets 1.0 só decodifica PNG/JPEG — por isso os ícones são
//! pré-rasterizados aqui. Indexa por nome de arquivo. Uso:
//!
//! ```sh
//! cargo run --manifest-path tools/svg2png/Cargo.toml
//! ```

use std::path::{Path, PathBuf};

use resvg::tiny_skia;
use resvg::usvg;

/// Escala de renderização (2x → 16px lógico vira 32px pixel).
const SCALE: f32 = 2.0;
/// Pastas de entrada → pasta de saída (PNG).
fn jobs() -> Vec<(PathBuf, PathBuf, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = root.parent().unwrap().parent().unwrap();
    let out = workspace.join("app-makepad").join("assets");

    let mut jobs = Vec::new();
    let icons_in = workspace.join("ui").join("icons");
    let icons_out = out.join("icons");
    for entry in std::fs::read_dir(&icons_in).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "svg") {
            jobs.push((icons_in.clone(), icons_out.clone(), path));
        }
    }
    jobs.push((
        workspace.join("app").join("assets"),
        out.clone(),
        workspace.join("app").join("assets").join("RuanaStudio.svg"),
    ));
    jobs
}

fn render_to_png(svg_path: &Path, out_dir: &Path, scale: f32) -> Result<(), String> {
    let data = std::fs::read(svg_path).map_err(|e| format!("{svg_path:?}: {e}"))?;

    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&data, &opt).map_err(|e| format!("{svg_path:?}: {e}"))?;

    let (w, h) = (tree.size().width(), tree.size().height());
    let (w, h) = (
        ((w * scale).max(1.0)).round() as u32,
        ((h * scale).max(1.0)).round() as u32,
    );

    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| format!("pixmap {w}x{h}"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let name = svg_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("stem de {svg_path:?}"))?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{out_dir:?}: {e}"))?;

    let mut out_path = out_dir.join(format!("{name}_2x.png"));
    if name == "RuanaStudio" {
        out_path = out_dir.join("ruana_studio_2x.png");
    }
    pixmap
        .save_png(&out_path)
        .map_err(|e| format!("{out_path:?}: {e}"))?;
    eprintln!("ok {name}: {w}x{h} → {}", out_path.display());
    Ok(())
}

fn main() {
    let mut failed = 0;
    for (in_dir, out_dir, path) in jobs() {
        let _ = in_dir;
        if let Err(e) = render_to_png(&path, &out_dir, SCALE) {
            failed += 1;
            eprintln!("erro: {e}");
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
    eprintln!("conversão concluída");
}