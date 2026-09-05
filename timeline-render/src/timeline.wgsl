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
    // Para N pontos o intervalo horizontal é 0..N-1 (não N), senão a
    // última amostra fica deslocada.
    let rect_w = in.size.x;
    let natural_w = in.wave_info.z;
    if natural_w > 0.0 && in.local.x * rect_w > natural_w {
        return in.frag_color;
    }
    let n = wave_count - 1u;
    let n_f = f32(n);
    var col_f: f32;
    if natural_w > 0.0 {
        col_f = (in.local.x * rect_w / natural_w) * n_f;
    } else {
        col_f = in.local.x * n_f;
    }
    if col_f < 0.0 {
        return in.frag_color;
    }

    // ------------------------------------------------------------
    // WAVEFORM
    //
    // Zoom alto  -> linha contínua tipo osciloscópio (fio),
    //               bitola constante ~1.5px.
    // Zoom baixo -> preenchimento entre a waveform e o zero.
    // Transição contínua via px_per_sample, sem saltos visuais.
    // ------------------------------------------------------------
    if wave_count < 2u {
        return in.frag_color;
    }

    let col = min(u32(floor(col_f)), wave_count - 2u);
    let next_col = col + 1u;

    let p0 = clamp(wave_bars[wave_offset + col], -1.0, 1.0);
    let p1 = clamp(wave_bars[wave_offset + next_col], -1.0, 1.0);

    let p = in.local * in.size;

    let wave_width = select(in.size.x, natural_w, natural_w > 0.0);

    let x0 = f32(col) / f32(wave_count - 1u) * wave_width;
    let x1 = f32(next_col) / f32(wave_count - 1u) * wave_width;

    let center = in.size.y * 0.5;
    let amp = in.size.y * 0.4;

    let y0 = center - p0 * amp;
    let y1 = center - p1 * amp;

    // Tamanho real (em pixels) de cada amostra na tela, na direção X.
    // Usa wave_count (nº de colunas) e a largura REAL em que o clip aparece.
    // `wave_width` é a largura natural da curva (clip_w, antes do stretch);
    // a razão clip_w/wave_count é constante por clip, mas multiplicada por
    // quanto o corpo foi esticado na tela (in.size.x/clip_w) — isso reflete
    // o ZOOM. Em zoom alto cada amostra vira vários pixels (>=2) -> fio;
    // em zoom baixo várias amostras caem dentro de um pixel (<1) -> preenchido.
    let stretch = max(in.size.x / max(natural_w, 0.001), 1.0);
    let px_per_sample = clamp((natural_w / f32(wave_count - 1u)) * stretch, 0.001, 1000.0);

    // POSIÇÃO DA WAVEFORM NO X ATUAL (segmento linear).
    let segment_width = max(x1 - x0, 0.0001);
    let t = clamp((p.x - x0) / segment_width, 0.0, 1.0);
    let wave_y = mix(y0, y1, t);

    // DIVIDIDO EM 3 ZONAS DE ZOOM, ancorado nos limites da timeline
    // (px_per_beat: min 8, max 640). A wave é gerada com 4 amostras por
    // beat-pixel no zoom base (40), então px_per_sample ≈ px_per_beat/160:
    //   zoom 8   → 0.05 px/sample
    //   zoom 640 → 4.0 px/sample
    // Normaliza em z ∈ [0,1]:
    //   z ≤ 1/3 (zoom baixo)  → WAVEFORM PREENCHIDA (até o zero)
    //   1/3 < z < 2/3         → transição contínua preenchida → fio
    //   z ≥ 2/3 (zoom alto)   → OSCILOSCÓPIO (só a linha fina)
    let z = clamp((px_per_sample - 0.05) / (4.0 - 0.05), 0.0, 1.0);
    let fill_amount = 1.0 - smoothstep(0.333, 0.667, z);

    // MODO PREENCHIDO.
    if fill_amount > 0.001 {
        let top = min(wave_y, center);
        let bottom = max(wave_y, center);
        let aa = max(fwidth(wave_y), 0.5);
        // Distância EXTERNA simétrica ao intervalo [top, bottom]:
        // <=0 dentro, >0 fora. Funciona tanto para wave_y > center
        // quanto para wave_y < center (corrige a versão com
        // inside/inside_bottom assimétricos).
        let outside = max(top - p.y, p.y - bottom);
        let cov = 1.0 - smoothstep(0.0, aa, outside);
        let fill_alpha = cov * fill_amount;
        if fill_alpha > 0.001 {
            // 0.0194 = #262626 em linear (alvo sRGB converte na escrita).
            // 100% opaco.
            return vec4<f32>(0.0194, 0.0194, 0.0194, fill_alpha);
        }
    }

    // MODO FIO — curva Catmull-Rom (C1 contínua) com SDF à reta tangente:
    // cancela o serrilhado das curvas mantendo bitola constante em qualquer
    // zoom. A distância projetada |Δy|/sqrt(1+slope²) é a distância real até
    // a tangente local da curva.
    let a0 = clamp(wave_bars[wave_offset + max(col, 1u) - 1u], -1.0, 1.0);
    let d3 = clamp(wave_bars[wave_offset + min(col + 2u, wave_count - 1u)], -1.0, 1.0);

    let tf = clamp(col_f - f32(col), 0.0, 1.0);
    let t2 = tf * tf;
    let t3 = t2 * tf;

    // Catmull-Rom (tensão 0.5) entre p0(col) e p1(col+1).
    let s = 0.5 * (2.0 * p0 + (-a0 + p1) * tf
                   + (2.0 * a0 - 5.0 * p0 + 4.0 * p1 - d3) * t2
                   + (-a0 + 3.0 * p0 - 3.0 * p1 + d3) * t3);
    let ds = 0.5 * ((-a0 + p1)
                    + (4.0 * a0 - 10.0 * p0 + 8.0 * p1 - 2.0 * d3) * tf
                    + (-3.0 * a0 + 9.0 * p0 - 9.0 * p1 + 3.0 * d3) * t2);

    let y_curve = center - s * amp;
    // Inclinação em pixels verticais por pixel horizontal.
    let slope_pp = ds * (-amp) * f32(wave_count - 1u) / max(wave_width, 0.001);
    let d_px = abs(p.y - y_curve) / sqrt(1.0 + slope_pp * slope_pp);

    // BITOLA CONSTANTE.
    let line_width = 1.5;
    let radius = line_width * 0.5;

    // AA curto: núcleo sólido (#262626) com feather de ~0.5px.
    let aa = clamp(fwidth(d_px), 0.35, 0.55);

    let alpha = 1.0 - smoothstep(radius - aa, radius + aa, d_px);
    if alpha <= 0.001 {
        return in.frag_color;
    }

    return vec4<f32>(0.0194, 0.0194, 0.0194, alpha);
}
