// Waveform "Bin Visual" renderer — WGSL.
//
// Um quad unitário (0..1) é expandido por instância: cada instância é
// uma coluna da waveform (1px). O storage buffer de barras traz um par
// vec2 (min, max) por coluna em amplitude [-1,1]. A vertex shader mapeia
// essa amplitude para NDC: horizontalmente pela coluna i, verticalmente
// por hscale (fração da meia-altura do corpo que a onda ocupa).

struct Uniforms {
    count: f32,      // nº de colunas
    hscale: f32,     // fração da meia-altura NDC (0..1), tipicamente ~0.85
    pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> bars: array<vec2<f32>>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @builtin(instance_index) i: u32,
) -> VsOut {
    let b = bars[i];
    let min = b.x;
    let max = b.y;

    // Coordenada horizontal: coluna i ocupa [left, right] no NDC.
    let left = (f32(i) + 0.0) / u.count * 2.0 - 1.0;
    let right = (f32(i) + 1.0) / u.count * 2.0 - 1.0;

    // Coordenada vertical: amplitude [-1,1] -> NDC y [-1,1] * hscale.
    // (NDC y é invertido: +1 em cima. Amplitude +max deve subir, logo top
    //  (maior y NDC) corresponde a -max.)
    let top = -max * u.hscale;       // valor NDC em cima (menos negativo = sobe)
    let bottom = -min * u.hscale;    // valor NDC embaixo

    let x = mix(left, right, pos.x);
    let y = mix(bottom, top, pos.y);

    var out: VsOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = b;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Branco puro pré-multiplicado? Não: cor aplicada na composição final.
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
