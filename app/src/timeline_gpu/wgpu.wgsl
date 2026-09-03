// Timeline renderer — WGSL.
//
// Um quad unitário (0..1) é expandido por instância. Cada instância
// representa um retângulo da timeline (recorte de clip, linha de grade,
// barra do ruler, faixa alternada). O instance buffer traz, por instância:
//   - rect: vec4 (x, y, w, h) em coordenadas de CONTEÚDO (pixels);
//   - color: vec4 RGBA.
//
// A câmera ortográfica converte conteúdo (pixels, com scroll + zoom) para
// NDC numa única multiplicação pela matriz `proj` (uniform). Assim o zoom
// e o pan são operações puramente matemáticas, independentes de quantos
// clips existem — pilares do AGENST.md.

struct Uniforms {
    // Matriz ortográfica conteúdo→NDC (já incorpora scroll/zoom/viewport).
    proj: mat4x4<f32>,
    _pad0: vec4<f32>,
    _pad1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// Traits por instância (interleave) do buffer de instâncias.
struct Instance {
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) frag_color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    inst: Instance,
) -> VsOut {
    // Corner do quad unitário [0..1]; posição final = retângulo da instância.
    let px = inst.rect.xy + corner * inst.rect.zw;

    var out: VsOut;
    out.pos = u.proj * vec4<f32>(px, 0.0, 1.0);
    out.local = corner;
    out.frag_color = inst.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.frag_color;
}