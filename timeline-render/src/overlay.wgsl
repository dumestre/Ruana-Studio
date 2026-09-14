// Overlay de UI — desenha uma textura RGBA (ex.: Slint renderizado offscreen)
// cobrindo a janela inteira, por cima da cena da timeline.
//
// Uma UI renderizada por software chega como RGBA straight; o blend padrão
// (alpha blending) compõe por cima da timeline já desenhada.
//
// Bind group:
//   binding 0: uniform proj (câmera ortográfica do viewport)
//   binding 1: sampler (linear, clamp)
//   binding 2: texture_2d — RGBA sRGB (linha 0 do buffer = topo da imagem)

struct Uniforms {
    proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@group(0) @binding(1)
var samp: sampler;

@group(0) @binding(2)
var tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
) -> VsOut {
    var out: VsOut;
    out.pos = u.proj * vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv);
    return vec4<f32>(c.rgb, c.a);
}