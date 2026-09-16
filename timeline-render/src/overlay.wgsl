// Overlay de UI — desenha uma textura RGBA (ex.: Slint renderizado offscreen)
// cobrindo a janela inteira, por cima da cena da timeline.
//
// Uma UI renderizada por software chega como RGBA straight; o blend padrão
// (alpha blending) compõe por cima da timeline já desenhada.
//
// Bind group:
//   binding 0: uniform proj (câmera ortográfica do viewport) + mask
//   binding 1: sampler (linear, clamp)
//   binding 2: texture_2d — RGBA sRGB (linha 0 do buffer = topo da imagem)
//
// `u.mask` = (largura do quad em px, altura do quad em px, raio dos cantos,
// flag ligado). Quando ligado, aplica um canto arredondado (SDF) no quad —
// os cantos ficam transparentes com anti-aliasing de ~1.5px, igual aos
// painéis do Slint (explorer/clip pool usam raio 6).

struct Uniforms {
    proj: mat4x4<f32>,
    mask: vec4<f32>,
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
    if u.mask.w < 0.5 {
        return vec4<f32>(c.rgb, c.a);
    }
    let size = max(u.mask.xy, vec2<f32>(1.0));
    let radius = min(u.mask.z, min(size.x, size.y) * 0.5);
    let p = in.uv * size;
    let q = abs(p - size * 0.5) - (size * 0.5 - radius);
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    let corner = 1.0 - smoothstep(-0.75, 0.75, d);
    // RGB já vem premultiplicado; multiplica RGB+A pelo fator do canto.
    return vec4<f32>(c.rgb * corner, c.a * corner);
}