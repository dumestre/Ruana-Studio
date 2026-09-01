//! Instance Pool — VAO/VBO global para instanced rendering de waveform.
//!
//! Um único VAO combinado com:
//! - VBO 0: quad vertices (4 vértices, vec2) — location 0, divisor 0
//! - VBO 1: instance data (min/max) — locations 1,2, divisor 1
//!
//! Clips escrevem seus dados min/max via `bufferSubData` no offset designado.
//! Zero alocações por frame, zero `glGenBuffers` por frame.

use glow::{Context, HasContext};
use std::mem::size_of;
use std::sync::Arc;

/// Dados por instância (8 bytes): min + max
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaveInstance {
    pub min: f32,
    pub max: f32,
}

/// Faixa do pool pertencente a um clip
#[derive(Debug, Clone, Copy)]
pub struct ClipRange {
    pub clip_id: u64,
    pub offset: usize,       // índice inicial no pool (em instâncias)
    pub count: usize,        // nº de instâncias (cols do LOD ativo)
    pub capacity: usize,     // capacidade reservada para este clip
}

/// Pool global de instâncias
pub struct InstancePool {
    gl: Arc<Context>,
    vao: glow::VertexArray,
    instance_vbo: glow::Buffer,
    capacity: usize,         // total de instâncias no pool
    used: usize,             // instâncias atualmente alocadas
    clip_ranges: Vec<ClipRange>,
}

impl InstancePool {
    /// Cria o pool com capacidade fixa (ex: 131072 instâncias = ~1 MB)
    pub fn new(gl: Arc<Context>, capacity: usize) -> Result<Self, String> {
        unsafe {
            // VAO combinado
            let vao = gl.create_vertex_array()
                .map_err(|e| format!("VAO creation failed: {e}"))?;
            gl.bind_vertex_array(Some(vao));

            // VBO 0: Quad vertices (4 vértices para TRIANGLE_STRIP)
            // (-1,-1), (1,-1), (-1,1), (1,1)
            let quad_vertices: [f32; 8] = [
                -1.0, -1.0,
                 1.0, -1.0,
                -1.0,  1.0,
                 1.0,  1.0,
            ];
            let quad_vbo = gl.create_buffer()
                .map_err(|e| format!("Quad VBO failed: {e}"))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(quad_vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&quad_vertices), glow::STATIC_DRAW);
            
            // Location 0 = a_pos (vec2), per-vertex
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.vertex_attrib_divisor(0, 0);

            // VBO 1: Instance data (min/max)
            let instance_vbo = gl.create_buffer()
                .map_err(|e| format!("Instance VBO failed: {e}"))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(instance_vbo));
            
            // Aloca buffer vazio (capacity * 8 bytes)
            let byte_size = capacity * size_of::<WaveInstance>();
            gl.buffer_data_size(glow::ARRAY_BUFFER, byte_size as i32, glow::DYNAMIC_DRAW);

            // Instance attributes
            // Location 1 = a_min (float)
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 1, glow::FLOAT, false, size_of::<WaveInstance>() as i32, 0);
            gl.vertex_attrib_divisor(1, 1);
            
            // Location 2 = a_max (float)
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 1, glow::FLOAT, false, size_of::<WaveInstance>() as i32, 4);
            gl.vertex_attrib_divisor(2, 1);

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            Ok(Self {
                gl,
                vao,
                instance_vbo,
                capacity,
                used: 0,
                clip_ranges: Vec::new(),
            })
        }
    }

    /// Reserva espaço para um clip (retorna offset e capacity)
    pub fn reserve(&mut self, clip_id: u64, count: usize) -> Result<ClipRange, String> {
        // Procura slot existente
        if let Some(idx) = self.clip_ranges.iter().position(|r| r.clip_id == clip_id) {
            let range = self.clip_ranges[idx];
            if range.capacity >= count {
                self.clip_ranges[idx].count = count;
                return Ok(self.clip_ranges[idx]);
            }
        }

        // Aloca no final do pool
        let offset = self.used;
        let new_used = offset + count;
        
        if new_used > self.capacity {
            return Err(format!("Instance pool exhausted: used={}, need={}, cap={}", 
                self.used, count, self.capacity));
        }

        let range = ClipRange {
            clip_id,
            offset,
            count,
            capacity: count,
        };
        self.clip_ranges.push(range);
        self.used = new_used;
        Ok(range)
    }

    /// Atualiza dados de instância para um clip (bufferSubData no instance VBO)
    pub fn update(&self, range: &ClipRange, instances: &[WaveInstance]) {
        if instances.is_empty() { return; }
        if instances.len() != range.count {
            return;
        }

        unsafe {
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instance_vbo));
            let byte_offset = (range.offset * size_of::<WaveInstance>()) as i32;
            let byte_size = (instances.len() * size_of::<WaveInstance>()) as i32;
            let bytes = bytemuck::cast_slice(instances);
            self.gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, byte_offset, bytes);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
    }

    /// Bind do VAO combinado (pronto para drawArraysInstanced)
    pub fn bind(&self) {
        unsafe {
            self.gl.bind_vertex_array(Some(self.vao));
        }
    }

    pub fn unbind(&self) {
        unsafe {
            self.gl.bind_vertex_array(None);
        }
    }

    /// Libera slot de um clip
    pub fn release(&mut self, clip_id: u64) {
        self.clip_ranges.retain(|r| r.clip_id != clip_id);
    }

    pub fn get_range(&self, clip_id: u64) -> Option<ClipRange> {
        self.clip_ranges.iter().find(|r| r.clip_id == clip_id).copied()
    }

    pub fn stats(&self) -> (usize, usize, usize) {
        (self.used, self.capacity, self.clip_ranges.len())
    }
}

impl Drop for InstancePool {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.instance_vbo);
            // quad_vbo não guardamos referência mas VAO tem referência forte
            // Em produção: guardar quad_vbo e deletar aqui também
        }
    }
}