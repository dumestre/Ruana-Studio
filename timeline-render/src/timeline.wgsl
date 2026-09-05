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
// concatenados. A instância refencia seu trecho via wave_offset/count.
// No fragment shader, mapeamos local.x (0..1) -> coluna, buscamos min/max,
// e se local.y estiver dentro do envelope vertical, sobrescrevemos a cor.

struct Uniforms {
    // Matriz ortográfica conteúdo→NDC (já incorpora scroll/zoom/viewport).
    proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// Amplitudes da waveform (modo osciloscópio): 1 amostra por coluna, por clip.
@group(0) @binding(1) var<storage, read> wave_bars: array<f32>;

// Traits por instância (interleave) do buffer de instâncias.
struct Instance {
    @location(1) rect: vec4<f32>,        // x, y, w, h (conteúdo px)
    @location(2) color: vec4<f32>,       // RGBA base
    @location(3) wave_info: vec4<f32>,   // x=offset, y=count, z=clip_w, w=clip_h
    @location(4) radius: f32,            // raio de arredondamento de cantos (px)
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,       // 0..1 dentro do rect
    @location(1) frag_color: vec4<f32>,  // cor base
    @location(2) wave_info: vec4<f32>,   // passa para fragment
    @location(3) radius: f32,            // raio (px)
    @location(4) size: vec2<f32>,        // w,h do rect (px) p/ arredondamento
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
    out.size = inst.rect.zw;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let wave_offset = u32(in.wave_info.x);
    let wave_count = u32(in.wave_info.y);
    let clip_width = in.wave_info.z;
    let _clip_height = in.wave_info.w;

    // Modo "listras diagonais" (sobreposição de clips): clip_width < 0 sinaliza.
    if clip_width < 0.0 {
        let px = in.local * in.size;
        let d = px.x + px.y;                 // diagonal 45° (px)
        let sp = 18.0;                        // espaçamento das listras
        let on = fract(d / sp) < 0.5;         // listra grossa = metade do espaçamento
        if !on {
            return vec4<f32>(in.frag_color.r, in.frag_color.g, in.frag_color.b,
                             in.frag_color.a * 0.20);
        }
        return in.frag_color;
    }

    // Arredondamento de cantos: descarta fragmentos fora do raio.
    if in.radius > 0.0 {
        let r = min(in.radius, min(in.size.x, in.size.y) * 0.5);
        let px = in.local * in.size;
        let dist_x = min(px.x, in.size.x - px.x);
        let dist_y = min(px.y, in.size.y - px.y);
        let inside = min(dist_x, dist_y) >= r;
        if !inside {
            let dx = r - dist_x;
            let dy = r - dist_y;
            if dx > 0.0 && dy > 0.0 && dx * dx + dy * dy > r * r {
                discard;
            }
        }
    }

    // Sem waveform: devolve cor base.
    if wave_count == 0 {
        return in.frag_color;
    }

    // Mapeia local.x (0..1) -> índice da coluna da waveform.
    // wave_info.z = largura natural da waveform (antes do stretch).
    // Se o rect for mais largo que a natural, a waveform não estica:
    //   - dentro da largura natural: waveform normal
    //   - além: só corpo colorido (sem waveform)
    let rect_w = in.size.x;
    let natural_w = in.wave_info.z;
    if natural_w > 0.0 && in.local.x * rect_w > natural_w {
        return in.frag_color;
    }
    var col_f: f32;
    if natural_w > 0.0 {
        col_f = (in.local.x * rect_w / natural_w) * f32(wave_count);
    } else {
        col_f = in.local.x * f32(wave_count);
    }
    if col_f < 0.0 || col_f >= f32(wave_count) - 1.0 {
        return in.frag_color;
    }

    // Modo osciloscópio com SDF ponto-segmento: para cada fragmento calcula a
    // distância REAL ao segmento de reta [col, col+1] em pixels e desenha com
    // espessura constante (~1.5px) + AA. Isso mantém a mesma bitola em toda a
    // curva e conecta segmentos íngremes (acabando com pontinhos/vazios).
    if wave_count < 2 {
        return in.frag_color;
    }
    let col = u32(floor(col_f));
    let next_col = min(col + 1u, wave_count - 1u);
    let p0 = clamp(wave_bars[wave_offset + col], -1.0, 1.0);
    let p1 = clamp(wave_bars[wave_offset + next_col], -1.0, 1.0);

    // Pernas do segmento em PIXELS dentro do corpo do clip.
    let amp = 0.4; // metade da altura = 40% para cima/baixo = 80% total
    let center = 0.5;
    let px_per_col = select(
        in.size.x / f32(wave_count),
        natural_w / f32(wave_count),
        natural_w > 0.0,
    );
    let ax = f32(col) * px_per_col;
    let bx = f32(col + 1u) * px_per_col;
    let ay = (center - p0 * amp) * in.size.y;
    let by = (center - p1 * amp) * in.size.y;
    let p = vec2<f32>(in.local.x * in.size.x, in.local.y * in.size.y);

    // Distância ponto-segmento (SDF).
    let ab = vec2<f32>(bx - ax, by - ay);
    let ap = p - vec2<f32>(ax, ay);
    let ab2 = dot(ab, ab);
    let t2 = select(1.0, dot(ap, ab) / ab2, ab2 > 0.0);
    let closest = vec2<f32>(ax, ay) + ab * clamp(t2, 0.0, 1.0);
    let dpx = distance(p, closest);

    // Bitola constante com AA (feather de 1px).
    let line_w = 1.5;
    let alpha = 1.0 - smoothstep(line_w * 0.5 - 0.5, line_w * 0.5 + 0.5, dpx);
    if alpha <= 0.0 {
        return in.frag_color;
    }
    return vec4<f32>(0.012, 0.012, 0.012, 0.95 * alpha);
}
