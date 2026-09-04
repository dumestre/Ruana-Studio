// Timeline renderer — WGSL.
//
// Um quad unitário (0..1) é expandido por instância. Cada instância
// representa um retângulo da timeline (recorte de clip, linha de grade,
// barra do ruler, faixa alternada). O instance buffer traz, por instância:
//   - rect: vec4 (x, y, w, h) em coordenadas de CONTEÚDO (pixels);
//   - color: vec4 RGBA (cor base do retângulo);
//   - wave_info: vec4 (wave_offset, wave_count, clip_width, clip_height).
//     Se wave_count > 0, esta instância é um CORPO de clip de áudio e
//     deve desenhar a waveform por cima do preenchimento.
//
// A câmera ortográfica converte conteúdo (pixels, com scroll + zoom) para
// NDC numa única multiplicação pela matriz `proj` (uniform). Assim o zoom
// e o pan são operações puramente matemáticas, independentes de quantos
// clips existem — pilares do AGENST.md.
//
// Waveforms: storage buffer `wave_bars` contém pares vec2(min, max) em
// amplitude [-1,1] para todas as colunas de todos os clips de áudio
// concatenados. A instância referencia seu trecho via wave_offset/count.
// No fragment shader, mapeamos local.x (0..1) -> coluna, buscamos min/max,
// e se local.y estiver dentro do envelope vertical, sobrescrevemos a cor.

struct Uniforms {
    // Matriz ortográfica conteúdo→NDC (já incorpora scroll/zoom/viewport).
    proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> wave_bars: array<vec2<f32>>;

// Traits por instância (interleave) do buffer de instâncias.
struct Instance {
    @location(1) rect: vec4<f32>,        // x, y, w, h (conteúdo px)
    @location(2) color: vec4<f32>,       // RGBA base
    @location(3) wave_info: vec4<f32>,   // x=offset, y=count, z=clip_w, h=clip_h
    @location(4) radius: f32,            // radius for circular clips
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,       // 0..1 dentro do rect
    @location(1) frag_color: vec4<f32>,  // cor base
    @location(2) wave_info: vec4<f32>,   // passa para fragment
    @location(3) radius: f32,            // radius for circle clipping
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
    out.wave_info = inst.wave_info;
    out.radius = inst.radius;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let wave_offset = u32(in.wave_info.x);
    let wave_count = u32(in.wave_info.y);
    let clip_width = in.wave_info.z;
    let _clip_height = in.wave_info.w;

    let head_radius = in.radius;

    // Sem waveform: devolve cor base.
    if wave_count == 0 {
        // Aplicar cantos arredondados se radius > 0
        if head_radius > 0.0 {
            let2 p = in.local; // 0..1 dentro do rect
            // Distância do centro para criar círculo
            let dist = distance(p, vec2(0.5, 0.5));
            if dist > head_radius {
                // Fora do círculo - tornar transparente
                return vec4<f32>(0.0, 0.0, 0.0, 0.0);
            }
        }
        return in.frag_color;
    }

    // Mapeia local.x (0..1) -> índice da coluna da waveform.
    let col_f = in.local.x * f32(wave_count);
    let col = u32(col_f);
    if col >= wave_count {
        return in.frag_color;
    }

    // Busca min/max da coluna no storage buffer.
    let bar = wave_bars[wave_offset + col];
    let mn = clamp(bar.x, -1.0, 1.0);
    let mx = clamp(bar.y, -1.0, 1.0);

    // Waveform centrada verticalmente no corpo do clip (0..1).
    // O envelope vai de center - mx*amp até center - mn*amp (NDC y invertido).
    // amp = fração da altura do clip que a onda ocupa (ex: 0.8).
    let amp = 0.4; // metade da altura = 40% para cima/baixo = 80% total
    let center = 0.5;
    let top = center - mx * amp;
    let bottom = center - mn * amp;

    // Se o fragmento está dentro do envelope vertical, desenha waveform.
    // Cor da waveform: branco semi-transparente sobre o corpo.
    if in.local.y >= top && in.local.y <= bottom {
        // Branco com alpha aditivo sobre o corpo (melhora visibilidade).
        // Mistura: 70% waveform + 30% corpo original.
        return vec4<f32>(0.9, 0.9, 0.9, 0.85);
    }

    return in.frag_color;
}