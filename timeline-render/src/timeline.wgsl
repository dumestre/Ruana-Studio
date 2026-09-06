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
// Waveforms: storage buffer `wave_bars` contém MIN/MAX interleaved (2 amostras por coluna)
// em amplitude [-1,1] para todas as colunas de todos os clips de áudio
// concatenados. A instância refencia seu trecho via wave_offset/count.
// No fragment shader, mapeamos local.x (0..1) -> coluna, buscamos min/max,
// e desenhamos a forma real da onda (positiva/negativa).

struct Uniforms {
    // Matriz ortográfica conteúdo→NDC (já incorpora scroll/zoom/viewport).
    proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// Amplitudes da waveform (min/max interleaved): 2 amostras por coluna, por clip.
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

    // Modo "listras diagonais" (sobreposição de clips): clip_width == -1.0 sinaliza.
    if abs(clip_width - (-1.0)) < 0.1 {
        let px = in.local * in.size;
        let d = px.x + px.y;
        let sp = 18.0;
        let on = fract(d / sp) < 0.5;
        if !on {
            return vec4<f32>(in.frag_color.r, in.frag_color.g, in.frag_color.b,
                             in.frag_color.a * 0.20);
        }
        return in.frag_color;
    }

    // ========================================================================
    // MODO FADE IN (clip_width == -2.0)
    // ========================================================================
    if abs(clip_width - (-2.0)) < 0.1 {
        let cx = clamp(in.wave_info.x / 1000.0, 0.02, 0.98);
        let cy = clamp(in.wave_info.y / 1000.0, 0.0, 1.0);
        let px = in.local * in.size;
        let x = clamp(in.local.x, 0.0, 1.0);

        var u: f32;
        let denom = 1.0 - 2.0 * cx;
        if abs(denom) < 0.001 {
            u = x;
        } else {
            let discr = max(cx * cx + denom * x, 0.0);
            u = clamp((-cx + sqrt(discr)) / denom, 0.0, 1.0);
        }

        let one_minus_u = 1.0 - u;
        let y_norm = 2.0 * one_minus_u * u * cy + u * u;

        let dx_du = max(2.0 * (1.0 - 2.0 * cx) * u + 2.0 * cx, 0.0001);
        let dy_du = 2.0 * (1.0 - 2.0 * cy) * u + 2.0 * cy;
        let dy_norm_dx = dy_du / dx_du;

        let y_curve = in.size.y * (1.0 - y_norm);
        let slope = - (in.size.y / max(in.size.x, 0.001)) * dy_norm_dx;

        let d_line = abs(px.y - y_curve) / sqrt(1.0 + slope * slope);
        let line_width = 1.8;
        let radius = line_width * 0.5;
        let aa = clamp(fwidth(d_line), 0.35, 0.65);
        let line_alpha = 1.0 - smoothstep(radius - aa, radius + aa, d_line);

        let d_fill = px.y - y_curve;
        let fill_aa = clamp(fwidth(d_fill), 0.35, 0.75);
        let fill_mask = 1.0 - smoothstep(0.0, fill_aa, d_fill);
        let shadow_alpha = fill_mask * 0.35;

        if line_alpha <= 0.001 && shadow_alpha <= 0.001 {
            discard;
        }

        let stroke_color = vec4<f32>(1.0, 1.0, 1.0, line_alpha * 0.95);
        let shadow_color = vec4<f32>(0.05, 0.05, 0.08, shadow_alpha);
        return mix(shadow_color, stroke_color, stroke_color.a);
    }

    // ========================================================================
    // MODO FADE OUT (clip_width == -3.0)
    // ========================================================================
    if abs(clip_width - (-3.0)) < 0.1 {
        let cx = clamp(in.wave_info.x / 1000.0, 0.02, 0.98);
        let cy = clamp(in.wave_info.y / 1000.0, 0.0, 1.0);
        let px = in.local * in.size;
        let x = clamp(in.local.x, 0.0, 1.0);

        var u: f32;
        let denom = 1.0 - 2.0 * cx;
        if abs(denom) < 0.001 {
            u = x;
        } else {
            let discr = max(cx * cx + denom * x, 0.0);
            u = clamp((-cx + sqrt(discr)) / denom, 0.0, 1.0);
        }

        let one_minus_u = 1.0 - u;
        let y_norm = one_minus_u * one_minus_u + 2.0 * one_minus_u * u * cy;

        let dx_du = max(2.0 * (1.0 - 2.0 * cx) * u + 2.0 * cx, 0.0001);
        let dy_du = -2.0 * one_minus_u + 2.0 * (1.0 - 2.0 * u) * cy;
        let dy_norm_dx = dy_du / dx_du;

        let y_curve = in.size.y * (1.0 - y_norm);
        let slope = - (in.size.y / max(in.size.x, 0.001)) * dy_norm_dx;

        let d_line = abs(px.y - y_curve) / sqrt(1.0 + slope * slope);
        let line_width = 1.8;
        let radius = line_width * 0.5;
        let aa = clamp(fwidth(d_line), 0.35, 0.65);
        let line_alpha = 1.0 - smoothstep(radius - aa, radius + aa, d_line);

        let d_fill = px.y - y_curve;
        let fill_aa = clamp(fwidth(d_fill), 0.35, 0.75);
        let fill_mask = 1.0 - smoothstep(0.0, fill_aa, d_fill);
        let shadow_alpha = fill_mask * 0.35;

        if line_alpha <= 0.001 && shadow_alpha <= 0.001 {
            discard;
        }

        let stroke_color = vec4<f32>(1.0, 1.0, 1.0, line_alpha * 0.95);
        let shadow_color = vec4<f32>(0.05, 0.05, 0.08, shadow_alpha);
        return mix(shadow_color, stroke_color, stroke_color.a);
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
    // WAVEFORM (simétrica: oscila + e - a partir do centro)
    // wave_info.z == -4.0 => stereo L (topo)
    // wave_info.z == -5.0 => stereo R (embaixo)
    // wave_info.z == -6.0 => mono na faixa central (1/3)
    // senão => mono (linha central, usado pelo fallback)
    // buffer: 2 floats por coluna (L,R) com sinal
    // Modos:
    //   - zoom baixo : preenchimento entre +val e -val
    //   - zoom alto  : linha Catmull-Rom simétrica
    // ------------------------------------------------------------
    if wave_count < 2u {
        return in.frag_color;
    }

    let col = min(u32(floor(col_f)), wave_count - 2u);
    let next_col = col + 1u;

    let is_stereo_l = abs(in.wave_info.z - (-4.0)) < 0.1;
    let is_stereo_r = abs(in.wave_info.z - (-5.0)) < 0.1;

    // Acesso ao buffer (sempre 2 floats por coluna: L, R)
    let p0_l = clamp(wave_bars[(wave_offset + col) * 2], -1.0, 1.0);
    let p0_r = clamp(wave_bars[(wave_offset + col) * 2 + 1], -1.0, 1.0);
    let p1_l = clamp(wave_bars[(wave_offset + next_col) * 2], -1.0, 1.0);
    let p1_r = clamp(wave_bars[(wave_offset + next_col) * 2 + 1], -1.0, 1.0);

    let p = in.local * in.size;

    let wave_width = select(in.size.x, natural_w, natural_w > 0.0);

    let x0 = f32(col) / f32(wave_count - 1u) * wave_width;
    let x1 = f32(next_col) / f32(wave_count - 1u) * wave_width;

    // Taper: começa mais grossa e termina mais afinada.
    let taper = 1.0 - in.local.x * 0.55;
    let line_width = 1.6 * taper;
    let radius = line_width * 0.5;

    // Cor fixa da waveform (cinza escuro).
    let wave_color = vec3<f32>(0.0194, 0.0194, 0.0194);

    // Segmento X atual
    let segment_width = max(x1 - x0, 0.0001);
    let t = clamp((p.x - x0) / segment_width, 0.0, 1.0);

    // Escolhe canal conforme a faixa.
    var v0: f32 = p0_l;
    var v1: f32 = p1_l;
    if is_stereo_l {
        v0 = p0_l;
        v1 = p1_l;
    } else if is_stereo_r {
        v0 = p0_r;
        v1 = p1_r;
    }

    // Valor interpolarizado no X atual (com sinal, pode ser + ou -).
    let val = mix(v0, v1, t);

    // Ajusta centro e amplitude por modo.
    // Para stereo L (-4.0), R (-5.0) e mono-center (-6.0), o rect já tem
    // altura proporcional (1/5 para stereo, 1/3 para mono) posicionado pelo
    // Rust-side push_clip. O center/amplitude default (body_h*0.5 / *0.4) já
    // centra a onda no rect corretamente.
    let body_h = in.size.y;
    let center_y = body_h * 0.5;
    let amp_y = body_h * 0.4;
    let half_h = body_h / 2.0;

    // Stereo L/R e mono-center: c e a usam os defaults acima.
    // (As checagens de canal L/R acima determinam qual amostra ler,
    // mas não precisam reajustar center/amplitude — o rect já está posicionado.)
    var c: f32 = center_y;
    var a: f32 = amp_y;

    // Tamanho real (em pixels) de cada amostra na tela.
    // Quando natural_w <= 0 (marcador stereo L/mono-center), usa a
    // largura do rect — senão o cálculo dá valores absurdo e o fill
    // mode ativa sempre, mesmo no zoom alto.
    let actual_w = select(natural_w, in.size.x, natural_w <= 0.0);
    let stretch = max(in.size.x / max(actual_w, 0.001), 1.0);
    let px_per_sample = clamp((actual_w / f32(wave_count - 1u)) * stretch, 0.001, 1000.0);

    // Modo preenchido (zoom baixo): entre y_bottom e y_top.
    if px_per_sample < 1.0 {
        let y_top = c - val * a;
        let y_bottom = c + val * a;
        let outside = max(y_bottom - p.y, p.y - y_top);
        let aa = clamp(fwidth(outside), 0.5, 1.5);
        let cov = 1.0 - smoothstep(0.0, aa, outside);
        if cov > 0.001 {
            return vec4<f32>(wave_color.r, wave_color.g, wave_color.b, cov);
        }
    }

    // Modo fio (zoom alto): curva Catmull-Rom simétrica.
    let a0 = clamp(wave_bars[(wave_offset + max(col, 1u) - 1u) * 2], -1.0, 1.0);
    let d3 = clamp(wave_bars[(wave_offset + min(col + 2u, wave_count - 1u)) * 2], -1.0, 1.0);

    let tf = clamp(col_f - f32(col), 0.0, 1.0);
    let t2 = tf * tf;
    let t3 = t2 * tf;

    // Catmull-Rom (tensão 0.5) para o valor com sinal.
    let s = 0.5 * (2.0 * v0 + (-a0 + v1) * tf
                   + (2.0 * a0 - 5.0 * v0 + 4.0 * v1 - d3) * t2
                   + (-a0 + 3.0 * v0 - 3.0 * v1 + d3) * t3);
    let ds = 0.5 * ((-a0 + v1)
                    + (4.0 * a0 - 10.0 * v0 + 8.0 * v1 - 2.0 * d3) * tf
                    + (-3.0 * a0 + 9.0 * v0 - 9.0 * v1 + 3.0 * d3) * t2);

    let y_curve = c - s * a;
    let slope_pp = ds * (-a) * f32(wave_count - 1u) / max(wave_width, 0.001);
    let d_px = abs(p.y - y_curve) / sqrt(1.0 + slope_pp * slope_pp);

    // BITOLA COM TAPER.
    let aa = clamp(fwidth(d_px), 0.35, 0.55);
    let alpha = 1.0 - smoothstep(radius - aa, radius + aa, d_px);
    if alpha <= 0.001 {
        return in.frag_color;
    }

    return vec4<f32>(wave_color.r, wave_color.g, wave_color.b, alpha);
}
