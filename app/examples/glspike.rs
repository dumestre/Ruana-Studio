use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::{DisplayApiPreference, GetGlDisplay};
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

struct Spike;

impl ApplicationHandler<()> for Spike {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match run_spike(event_loop) {
            Ok(px) => println!("spike OK pixel={px:?}"),
            Err(e) => println!("spike FAIL: {e:?}"),
        }
        event_loop.exit();
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, _event: WindowEvent) {}
}

fn run_spike(event_loop: &ActiveEventLoop) -> Result<[u8; 4], String> {
    let window: Window = event_loop
        .create_window(
            Window::default_attributes()
                .with_title("glhidden")
                .with_visible(false)
                .with_inner_size(LogicalSize::new(16.0, 16.0)),
        )
        .map_err(|e| e.to_string())?;

    let raw_display: RawDisplayHandle = window.display_handle().map_err(|e| e.to_string())?.as_raw();
    let raw_window: RawWindowHandle = window.window_handle().map_err(|e| e.to_string())?.as_raw();

    let display = unsafe {
        glutin::display::Display::new(raw_display, DisplayApiPreference::WglThenEgl(Some(raw_window)))
            .map_err(|e| e.to_string())?
    };
    println!("display api = {}", display.version_string());

    let template = ConfigTemplateBuilder::new().build();

    let config = unsafe {
        display
            .find_configs(template)
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("no config".to_string())?
    };

    let surface = unsafe {
        display
            .create_window_surface(
                &config,
                &SurfaceAttributesBuilder::<WindowSurface>::new()
                    .build(raw_window, std::num::NonZeroU32::new(16).unwrap(), std::num::NonZeroU32::new(16).unwrap()),
            )
            .map_err(|e| e.to_string())?
    };

    let ctx_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(Some(raw_window));
    let not_current = unsafe {
        display
            .create_context(&config, &ctx_attrs)
            .map_err(|e| e.to_string())?
    };
    let _current = not_current.make_current(&surface).map_err(|e| e.to_string())?;

    let gl = unsafe { glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name)) };

    unsafe {
        gl.clear_color(0.0, 1.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        let mut px = [0u8; 4];
        gl.read_pixels(
            0,
            0,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut px)),
        );
        Ok(px)
    }
}

fn main() -> Result<(), winit::error::EventLoopError> {
    let event_loop: EventLoop<()> = EventLoop::builder().build()?;
    event_loop.run_app(&mut Spike)
}