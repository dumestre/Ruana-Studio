//! Renderer GPU principal — roda na main thread via timer Slint.
//!
//! Não usa janela overlay separada. Renderiza direto no framebuffer do Slint
//! (ou em textura própria) usando o contexto OpenGL compartilhado.

use crate::waveform_gpu::{
    instance_pool::{InstancePool, ClipRange, WaveInstance},
    shader::WaveformShader,
};
use crate::waveform::{WaveformCache, pick_level, bake_bars, LodBars};
use glow::HasContext;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Estado de um clip no renderer GPU
#[derive(Debug, Clone)]
pub struct GpuClipState {
    pub clip_id: u64,
    pub range: ClipRange,
    pub mvp: [f32; 16],
    pub color: [f32; 4],
    pub bounds: ClipBounds,
}

/// Bounds do clip em coordenadas de tela (px físicos)
#[derive(Debug, Clone, Copy)]
pub struct ClipBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Renderer GPU para waveforms — roda na main thread
pub struct WaveformGpuRenderer {
    gl: Arc<glow::Context>,
    pool: InstancePool,
    shader: WaveformShader,
    clips: HashMap<u64, GpuClipState>,
    waveforms_cache: Arc<Mutex<WaveformCache>>,
    window_size: (u32, u32),
    hwnd: *mut std::ffi::c_void, // HWND para swap buffers
}

impl WaveformGpuRenderer {
    /// Inicializa renderer na main thread
    pub fn new(
        gl: Arc<glow::Context>,
        hwnd: *mut std::ffi::c_void,
        window_size: (u32, u32),
        waveforms_cache: Arc<Mutex<WaveformCache>>,
    ) -> Result<Self, String> {
        // Instance pool
        const POOL_CAPACITY: usize = 131072;
        let pool = InstancePool::new(gl.clone(), POOL_CAPACITY)?;

        // Shader program
        let shader = WaveformShader::new(gl.clone())?;

        Ok(Self {
            gl,
            pool,
            shader,
            clips: HashMap::new(),
            waveforms_cache,
            window_size,
            hwnd,
        })
    }

    /// Atualiza tamanho da janela (chamado no resize)
    pub fn resize(&mut self, window_size: (u32, u32)) {
        self.window_size = window_size;
        for clip in self.clips.values_mut() {
            clip.mvp = Self::compute_mvp(&clip.bounds, window_size);
        }
    }

    /// Atualiza/registra um clip no renderer
    pub fn update_clip(
        &mut self,
        clip_id: u64,
        bounds: ClipBounds,
        color: [f32; 4],
        gain_linear: f32,
        fade_in_px: f32,
        fade_out_px: f32,
    ) {
        // Busca waveform no cache CPU e clona o LOD necessário
        let lod = {
            let cache = self.waveforms_cache.lock().unwrap();
            cache.get(&clip_id).and_then(|cw| {
                let lod = pick_level(&cw.lods, bounds.width as usize);
                Some(LodBars {
                    mins: lod.mins.clone(),
                    maxs: lod.maxs.clone(),
                })
            })
        };

        let Some(lod) = lod else { return };

        // Bake gain + fades
        let (mins, maxs) = bake_bars(&lod, bounds.width as u32, gain_linear, fade_in_px, fade_out_px);
        if mins.is_empty() { return; }

        // Converte para WaveInstance
        let instances: Vec<WaveInstance> = mins.into_iter()
            .zip(maxs.into_iter())
            .map(|(min, max)| WaveInstance { min, max })
            .collect();

        // Reserva/atualiza no pool
        let range = match self.pool.reserve(clip_id, instances.len()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[waveform_gpu] Pool reserve failed: {e}");
                return;
            }
        };
        self.pool.update(&range, &instances);

        // MVP matrix
        let mvp = Self::compute_mvp(&bounds, self.window_size);

        // Estado do clip
        let clip_state = GpuClipState {
            clip_id,
            range,
            mvp,
            color,
            bounds,
        };
        self.clips.insert(clip_id, clip_state);
    }

    /// Remove clip
    pub fn remove_clip(&mut self, clip_id: u64) {
        self.clips.remove(&clip_id);
        self.pool.release(clip_id);
    }

    /// Render frame — chamado via timer Slint
    pub fn render(&mut self) {
        if self.clips.is_empty() { return; }

        unsafe {
            let gl = &self.gl;
            
            // Clear transparente
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            // Bind shader + VAO
            self.shader.use_program();
            self.pool.bind();

            // Draw cada clip
            for clip in self.clips.values() {
                if clip.range.count == 0 { continue; }
                
                self.shader.set_mvp(&clip.mvp);
                self.shader.set_color(clip.color[0], clip.color[1], clip.color[2], clip.color[3]);
                self.shader.set_instance_count(clip.range.count as i32);
                
                // Clip bounds em NDC para fragment shader
                let h = self.window_size.1 as f32;
                let clip_top_ndc = 1.0 - 2.0 * (clip.bounds.y + clip.bounds.height) / h;
                let clip_bottom_ndc = 1.0 - 2.0 * clip.bounds.y / h;
                self.shader.set_clip_bounds(clip_top_ndc, clip_bottom_ndc);
                
                gl.draw_arrays_instanced(
                    glow::TRIANGLE_STRIP,
                    0, 4,
                    clip.range.count as i32,
                );
            }

            // Swap buffers no Windows
            #[cfg(windows)]
            {
                use crate::waveform_gpu::win32_gdi::{get_dc, wgl_swap_layer_buffers, WGL_SWAP_MAIN_PLANE};
                
                let dc = get_dc(self.hwnd);
                wgl_swap_layer_buffers(dc, WGL_SWAP_MAIN_PLANE);
            }
        }
    }

    /// MVP orthographic: px físicos → NDC
    fn compute_mvp(bounds: &ClipBounds, window_size: (u32, u32)) -> [f32; 16] {
        let w = window_size.0 as f32;
        let h = window_size.1 as f32;
        
        let sx = 2.0 * bounds.width / w;
        let sy = 2.0 * bounds.height / h;
        let tx = 2.0 * (bounds.x + bounds.width * 0.5) / w - 1.0;
        let ty = 1.0 - 2.0 * (bounds.y + bounds.height * 0.5) / h;

        [
            sx,  0.0, 0.0, 0.0,
            0.0, sy,  0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            tx,  ty,  0.0, 1.0,
        ]
    }
}