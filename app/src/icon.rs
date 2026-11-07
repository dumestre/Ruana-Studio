use i_slint_backend_winit::{WinitWindowAccessor, winit};

/// PNG 64x64 gerado no build (build.rs) a partir de assets/RuanaStudio.svg.
const ICON_PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app_icon_64.png"));

/// Decodifica o PNG do ícone para RGBA e cria um `Icon` do winit.
pub fn app_icon() -> Option<winit::window::Icon> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON_PNG));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut data = vec![0; reader.output_buffer_size().unwrap_or(0)];
    reader.next_frame(&mut data).ok()?;
    let info = reader.info();
    winit::window::Icon::from_rgba(data, info.width, info.height).ok()
}

/// Aplica o ícone da Ruana Studio numa janela Slint via backend winit.
pub fn apply<W: WinitWindowAccessor>(window: &W) {
    if let Some(icon) = app_icon() {
        let _ = window.with_winit_window(move |w| w.set_window_icon(Some(icon)));
    }
}

/// Aplica o ícone directamente numa janela winit (modo hosted/AGENTS.md).
pub fn apply_to_window(window: &winit::window::Window) {
    if let Some(icon) = app_icon() {
        window.set_window_icon(Some(icon));
    }
}
