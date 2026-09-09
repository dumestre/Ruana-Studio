// Renderer de ícones — quad unitário (0..1) expandido por instância,
// texturizado pelo atlas de ícones e tintado por uma cor.
//
// Bind group:
//   binding 0: uniform proj (câmera ortográfica)
//   binding 1: sampler (nearest, clamp)
//   binding 2: texture_2d — atlas RGBA (alpha straight)
//
// A textura é sRGB; a amostra sai em espaço linear e o tint também é
// informado linear (mesma convenção do pipeline principal).

struct Uniforms {
    proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@group(0) @binding(1)
var samp: sampler;

@group(0) @binding(2)
var tex: texture_2d<f32>;

struct Instance {
    @location(1) rect: vec4<f32>,
    @location(2) uv: vec4<f32>,
    @location(3) tint: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    inst: Instance,
) -> VsOut {
    let px = inst.rect.xy + corner * inst.rect.zw;

    var out: VsOut;
    out.pos = u.proj * vec4<f32>(px, 0.0, 1.0);
    out.uv = inst.uv.xy + corner * inst.uv.zw;
    out.tint = inst.tint;

    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv);
    return vec4<f32>(c.rgb * in.tint.rgb, c.a * in.tint.a);
}