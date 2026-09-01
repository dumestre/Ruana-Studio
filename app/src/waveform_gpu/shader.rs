//! Shader Program wrapper para waveform instanced rendering.

use glow::{Context, HasContext, NativeUniformLocation};
use std::sync::Arc;

// Include shader sources
const VERTEX_SHADER_SRC: &str = include_str!("shader.vert");
const FRAGMENT_SHADER_SRC: &str = include_str!("shader.frag");

pub const VERTEX_SHADER: &str = VERTEX_SHADER_SRC;
pub const FRAGMENT_SHADER: &str = FRAGMENT_SHADER_SRC;

pub struct WaveformShader {
    gl: Arc<Context>,
    program: glow::Program,
    // Uniform locations
    u_mvp: Option<glow::NativeUniformLocation>,
    u_color: Option<glow::NativeUniformLocation>,
    u_instance_count: Option<glow::NativeUniformLocation>,
    u_clip_top: Option<glow::NativeUniformLocation>,
    u_clip_bottom: Option<glow::NativeUniformLocation>,
}

impl WaveformShader {
    pub fn new(gl: Arc<Context>) -> Result<Self, String> {
        Self::from_source(gl, VERTEX_SHADER_SRC, FRAGMENT_SHADER_SRC)
    }

    pub fn from_source(gl: Arc<Context>, vert_src: &str, frag_src: &str) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()
                .map_err(|e| format!("Program creation failed: {e}"))?;

            // Vertex shader
            let vert = gl.create_shader(glow::VERTEX_SHADER)
                .map_err(|e| format!("Vertex shader creation failed: {e}"))?;
            gl.shader_source(vert, vert_src);
            gl.compile_shader(vert);
            if !gl.get_shader_compile_status(vert) {
                let log = gl.get_shader_info_log(vert);
                return Err(format!("Vertex shader compile error: {log}"));
            }
            gl.attach_shader(program, vert);

            // Fragment shader
            let frag = gl.create_shader(glow::FRAGMENT_SHADER)
                .map_err(|e| format!("Fragment shader creation failed: {e}"))?;
            gl.shader_source(frag, frag_src);
            gl.compile_shader(frag);
            if !gl.get_shader_compile_status(frag) {
                let log = gl.get_shader_info_log(frag);
                return Err(format!("Fragment shader compile error: {log}"));
            }
            gl.attach_shader(program, frag);

            // Link
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                return Err(format!("Program link error: {log}"));
            }

            // Cleanup shaders
            gl.detach_shader(program, vert);
            gl.detach_shader(program, frag);
            gl.delete_shader(vert);
            gl.delete_shader(frag);

            // Uniform locations
            let u_mvp = gl.get_uniform_location(program, "u_mvp");
            let u_color = gl.get_uniform_location(program, "u_color");
            let u_instance_count = gl.get_uniform_location(program, "u_instance_count");
            let u_clip_top = gl.get_uniform_location(program, "u_clip_top");
            let u_clip_bottom = gl.get_uniform_location(program, "u_clip_bottom");

            Ok(Self {
                gl,
                program,
                u_mvp,
                u_color,
                u_instance_count,
                u_clip_top,
                u_clip_bottom,
            })
        }
    }

    pub fn use_program(&self) {
        unsafe {
            self.gl.use_program(Some(self.program));
        }
    }

    pub fn set_mvp(&self, mvp: &[f32; 16]) {
        unsafe {
            if let Some(loc) = self.u_mvp {
                self.gl.uniform_matrix_4_f32_slice(Some(&loc), true, mvp);
            }
        }
    }

    pub fn set_color(&self, r: f32, g: f32, b: f32, a: f32) {
        unsafe {
            if let Some(loc) = self.u_color {
                self.gl.uniform_4_f32(Some(&loc), r, g, b, a);
            }
        }
    }

    pub fn set_instance_count(&self, count: i32) {
        unsafe {
            if let Some(loc) = self.u_instance_count {
                self.gl.uniform_1_i32(Some(&loc), count);
            }
        }
    }

    pub fn set_clip_bounds(&self, top: f32, bottom: f32) {
        unsafe {
            if let Some(loc) = self.u_clip_top {
                self.gl.uniform_1_f32(Some(&loc), top);
            }
            if let Some(loc) = self.u_clip_bottom {
                self.gl.uniform_1_f32(Some(&loc), bottom);
            }
        }
    }

    pub fn program_id(&self) -> glow::Program {
        self.program
    }
}

impl Drop for WaveformShader {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
        }
    }
}