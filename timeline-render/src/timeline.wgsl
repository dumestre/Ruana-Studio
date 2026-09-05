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
    // WAVEFORM (min/max por coluna)
    // ------------------------------------------------------------
    if wave_count < 2u {
        return in.frag_color;
    }

    let col = min(u32(floor(col_f)), wave_count - 2u);
    let next_col = col + 1u;

    let p0_min = clamp(wave_bars[(wave_offset + col) * 2], -1.0, 1.0);
    let p0_max = clamp(wave_bars[(wave_offset + col) * 2 + 1], -1.0, 1.0);
    let p1_min = clamp(wave_bars[(wave_offset + next_col) * 2], -1.0, 1.0);
    let p1_max = clamp(wave_bars[(wave_offset + next_col) * 2 + 1], -1.0, 1.0);

    let p = in.local * in.size;

    let wave_width = select(in.size.x, natural_w, natural_w > 0.0);

    let x0 = f32(col) / f32(wave_count - 1u) * wave_width;
    let x1 = f32(next_col) / f32(wave_count - 1u) * wave_width;

    let center = in.size.y * 0.5;
    let amp = in.size.y * 0.4;

    let y0_min = center - p0_min * amp;
    let y0_max = center - p0_max * amp;
    let y1_min = center - p1_min * amp;
    let y1_max = center - p1_max * amp;

    let stretch = max(in.size.x / max(natural_w, 0.001), 1.0);
    let px_per_sample = clamp((natural_w / f32(wave_count - 1u)) * stretch, 0.001, 1000.0);

    let segment_width = max(x1 - x0, 0.0001);
    let t = clamp((p.x - x0) / segment_width, 0.0, 1.0);
    let wave_y_min = mix(y0_min, y1_min, t);
    let wave_y_max = mix(y0_max, y1_max, t);

    let z = clamp((px_per_sample - 0.05) / (4.0 - 0.05), 0.0, 1.0);
    let fill_amount = 1.0 - smoothstep(0.333, 0.667, z);

    if px_per_sample < 1.0 {
        let c0 = u32(floor(col_f));
        let span = min(u32(ceil(1.0 / max(px_per_sample, 0.001))), 64u);
        let c1 = min(c0 + span, wave_count - 1u);
        var env_min: f32 = 0.0;
        var env_max: f32 = 0.0;
        var k = c0;
        while k < c1 {
            env_min = min(env_min, wave_bars[(wave_offset + k) * 2]);
            env_max = max(env_max, wave_bars[(wave_offset + k) * 2 + 1]);
            k = k + 1u;
        }
        let top = center - env_max * amp;
        let bottom = center - env_min * amp;
        let outside = max(top - p.y, p.y - bottom);
        let env_aa = clamp(fwidth(outside), 0.35, 1.5);
        let cov = 1.0 - smoothstep(0.0, env_aa, outside);
        if cov * fill_amount > 0.001 {
            return vec4<f32>(0.0194, 0.0194, 0.0194, cov * fill_amount);
        }
    }

    // MODO PREENCHIDO (entre min e max da coluna).
    if fill_amount > 0.001 {
        let top = min(wave_y_max, wave_y_min);
        let bottom = max(wave_y_max, wave_y_min);
        let outside = max(top - p.y, p.y - bottom);
        let aa = clamp(fwidth(outside), 0.5, 1.5);
        let cov = 1.0 - smoothstep(0.0, aa, outside);
        let fill_alpha = cov * fill_amount;
        if fill_alpha > 0.001 {
            return vec4<f32>(0.0194, 0.0194, 0.0194, fill_alpha);
        }
    }

    // MODO FIO — curva Catmull-Rom usando min e max como pontos de controle.
    let a0_min = clamp(wave_bars[(wave_offset + max(col, 1u) - 1u) * 2], -1.0, 1.0);
    let a0_max = clamp(wave_bars[(wave_offset + max(col, 1u) - 1u) * 2 + 1], -1.0, 1.0);
    let d3_min = clamp(wave_bars[(wave_offset + min(col + 2u, wave_count - 1u)) * 2], -1.0, 1.0);
    let d3_max = clamp(wave_bars[(wave_offset + min(col + 2u, wave_count - 1u)) * 2 + 1], -1.0, 1.0);

    let tf = clamp(col_f - f32(col), 0.0, 1.0);
    let t2 = tf * tf;
    let t3 = t2 * tf;

    // Catmull-Rom para min e max separadamente.
    let s_min = 0.5 * (2.0 * p0_min + (-a0_min + p1_min) * tf
                    + (2.0 * a0_min - 5.0 * p0_min + 4.0 * p1_min - d3_min) * t2
                    + (-a0_min + 3.0 * p0_min - 3.0 * p1_min + d3_min) * t3);
    let s_max = 0.5 * (2.0 * p0_max + (-a0_max + p1_max) * tf
                    + (2.0 * a0_max - 5.0 * p0_max + 4.0 * p1_max - d3_max) * t2
                    + (-a0_max + 3.0 * p0_max - 3.0 * p1_max + d3_max) * t3);

    let ds_min = 0.5 * ((-a0_min + p1_min)
                    + (4.0 * a0_min - 10.0 * p0_min + 8.0 * p1_min - 2.0 * d3_min) * tf
                    + (-3.0 * a0_min + 9.0 * p0_min - 9.0 * p1_min + 3.0 * d3_min) * t2);
    let ds_max = 0.5 * ((-a0_max + p1_max)
                    + (4.0 * a0_max - 10.0 * p0_max + 8.0 * p1_max - 2.0 * d3_max) * tf
                    + (-3.0 * a0_max + 9.0 * p0_max - 9.0 * p1_max + 3.0 * d3_max) * t2);

    let y_curve_min = center - s_min * amp;
    let y_curve_max = center - s_max * amp;
    let slope_pp_min = ds_min * (-amp) * f32(wave_count - 1u) / max(wave_width, 0.001);
    let slope_pp_max = ds_max * (-amp) * f32(wave_count - 1u) / max(wave_width, 0.001);

    let d_px_min = abs(p.y - y_curve_min) / sqrt(1.0 + slope_pp_min * slope_pp_min);
    let d_px_max = abs(p.y - y_curve_max) / sqrt(1.0 + slope_pp_max * slope_pp_max);
    let d_px = min(d_px_min, d_px_max);

    let line_width = 1.5;
    let radius = line_width * 0.5;
    let aa = clamp(fwidth(d_px), 0.35, 0.55);
    let alpha = 1.0 - smoothstep(radius - aa, radius + aa, d_px);
    if alpha <= 0.001 {
        return in.frag_color;
    }

    return vec4<f32>(0.0194, 0.0194, 0.0194, alpha);
}
