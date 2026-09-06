// Timeline renderer — WGSL.
//
// Um quad unitário (0..1) é expandido por instância. Cada instância
// representa um retângulo da timeline (recorte de clip, linha de grade,
// barra do ruler, faixa alternada).
//
// wave_info:
//   x = wave_offset
//   y = wave_count
//   z = clip_width / modo especial
//   w = clip_height
//
// IMPORTANTE:
// - wave_count == 0 => retângulo normal, pinta todo o corpo.
// - wave_count > 0 => instância de waveform.
//   Fora da waveform usamos DISCARD, nunca frag_color.
//   Assim a waveform não cria um "retângulo de fundo".
//
// A câmera ortográfica converte conteúdo (pixels, com scroll + zoom)
// para NDC numa única multiplicação pela matriz `proj`.
//
// Waveforms:
// storage buffer `wave_bars` contém L/R interleaved:
//   [L0, R0, L1, R1, L2, R2, ...]
//

struct Uniforms {
    proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@group(0) @binding(1)
var<storage, read> wave_bars: array<f32>;

struct Instance {
    @location(1) rect: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) wave_info: vec4<f32>,
    @location(4) radius: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) frag_color: vec4<f32>,
    @location(2) wave_info: vec4<f32>,
    @location(3) radius: f32,
    @location(4) size: vec2<f32>,
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

    // ========================================================================
    // MODO LISTRAS DIAGONAIS
    // clip_width == -1.0
    // ========================================================================

    if abs(clip_width - (-1.0)) < 0.1 {
        let px = in.local * in.size;
        let d = px.x + px.y;

        let sp = 18.0;
        let on = fract(d / sp) < 0.5;

        if !on {
            return vec4<f32>(
                in.frag_color.r,
                in.frag_color.g,
                in.frag_color.b,
                in.frag_color.a * 0.20
            );
        }

        return in.frag_color;
    }

    // ========================================================================
    // MODO FADE IN
    // clip_width == -2.0
    // ========================================================================

    if abs(clip_width - (-2.0)) < 0.1 {
        let cx = clamp(in.wave_info.x / 1000.0, 0.02, 0.98);
        let cy = clamp(in.wave_info.y / 1000.0, 0.0, 1.0);

        let px = in.local * in.size;
        let x = clamp(in.local.x, 0.0, 1.0);

        var curve_u: f32;

        let denom = 1.0 - 2.0 * cx;

        if abs(denom) < 0.001 {
            curve_u = x;
        } else {
            let discr = max(cx * cx + denom * x, 0.0);

            curve_u = clamp(
                (-cx + sqrt(discr)) / denom,
                0.0,
                1.0
            );
        }

        let one_minus_u = 1.0 - curve_u;

        let y_norm =
            2.0 * one_minus_u * curve_u * cy +
            curve_u * curve_u;

        let dx_du = max(
            2.0 * (1.0 - 2.0 * cx) * curve_u + 2.0 * cx,
            0.0001
        );

        let dy_du =
            2.0 * (1.0 - 2.0 * cy) * curve_u +
            2.0 * cy;

        let dy_norm_dx = dy_du / dx_du;

        let y_curve = in.size.y * (1.0 - y_norm);

        let slope =
            -(in.size.y / max(in.size.x, 0.001)) *
            dy_norm_dx;

        let d_line =
            abs(px.y - y_curve) /
            sqrt(1.0 + slope * slope);

        let line_width = 1.8;
        let fade_radius = line_width * 0.5;

        let aa = clamp(
            fwidth(d_line),
            0.35,
            0.65
        );

        let line_alpha =
            1.0 -
            smoothstep(
                fade_radius - aa,
                fade_radius + aa,
                d_line
            );

        let d_fill = px.y - y_curve;

        let fill_aa = clamp(
            fwidth(d_fill),
            0.35,
            0.75
        );

        let fill_mask =
            1.0 -
            smoothstep(
                0.0,
                fill_aa,
                d_fill
            );

        let shadow_alpha =
            fill_mask * 0.35;

        if line_alpha <= 0.001 &&
           shadow_alpha <= 0.001 {
            discard;
        }

        let stroke_color =
            vec4<f32>(
                1.0,
                1.0,
                1.0,
                line_alpha * 0.95
            );

        let shadow_color =
            vec4<f32>(
                0.05,
                0.05,
                0.08,
                shadow_alpha
            );

        return mix(
            shadow_color,
            stroke_color,
            stroke_color.a
        );
    }

    // ========================================================================
    // MODO FADE OUT
    // clip_width == -3.0
    // ========================================================================

    if abs(clip_width - (-3.0)) < 0.1 {
        let cx = clamp(in.wave_info.x / 1000.0, 0.02, 0.98);
        let cy = clamp(in.wave_info.y / 1000.0, 0.0, 1.0);

        let px = in.local * in.size;
        let x = clamp(in.local.x, 0.0, 1.0);

        var curve_u: f32;

        let denom = 1.0 - 2.0 * cx;

        if abs(denom) < 0.001 {
            curve_u = x;
        } else {
            let discr = max(cx * cx + denom * x, 0.0);

            curve_u = clamp(
                (-cx + sqrt(discr)) / denom,
                0.0,
                1.0
            );
        }

        let one_minus_u = 1.0 - curve_u;

        let y_norm =
            one_minus_u * one_minus_u +
            2.0 * one_minus_u * curve_u * cy;

        let dx_du = max(
            2.0 * (1.0 - 2.0 * cx) * curve_u + 2.0 * cx,
            0.0001
        );

        let dy_du =
            -2.0 * one_minus_u +
            2.0 * (1.0 - 2.0 * curve_u) * cy;

        let dy_norm_dx = dy_du / dx_du;

        let y_curve =
            in.size.y * (1.0 - y_norm);

        let slope =
            -(in.size.y / max(in.size.x, 0.001)) *
            dy_norm_dx;

        let d_line =
            abs(px.y - y_curve) /
            sqrt(1.0 + slope * slope);

        let line_width = 1.8;
        let fade_radius = line_width * 0.5;

        let aa = clamp(
            fwidth(d_line),
            0.35,
            0.65
        );

        let line_alpha =
            1.0 -
            smoothstep(
                fade_radius - aa,
                fade_radius + aa,
                d_line
            );

        let d_fill = px.y - y_curve;

        let fill_aa = clamp(
            fwidth(d_fill),
            0.35,
            0.75
        );

        let fill_mask =
            1.0 -
            smoothstep(
                0.0,
                fill_aa,
                d_fill
            );

        let shadow_alpha =
            fill_mask * 0.35;

        if line_alpha <= 0.001 &&
           shadow_alpha <= 0.001 {
            discard;
        }

        let stroke_color =
            vec4<f32>(
                1.0,
                1.0,
                1.0,
                line_alpha * 0.95
            );

        let shadow_color =
            vec4<f32>(
                0.05,
                0.05,
                0.08,
                shadow_alpha
            );

        return mix(
            shadow_color,
            stroke_color,
            stroke_color.a
        );
    }

    // ========================================================================
    // ARREDONDAMENTO DOS CANTOS
    // ========================================================================

    if in.radius > 0.0 {
        let r = min(
            in.radius,
            min(in.size.x, in.size.y) * 0.5
        );

        let px = in.local * in.size;

        let dist_x =
            min(px.x, in.size.x - px.x);

        let dist_y =
            min(px.y, in.size.y - px.y);

        let inside =
            min(dist_x, dist_y) >= r;

        if !inside {
            let dx = r - dist_x;
            let dy = r - dist_y;

            if dx > 0.0 &&
               dy > 0.0 &&
               dx * dx + dy * dy > r * r {
                discard;
            }
        }
    }

    // ========================================================================
    // RETÂNGULO NORMAL / CORPO DO CLIP
    //
    // ESTE É O ÚNICO CASO EM QUE O RETÂNGULO INTEIRO É PINTADO.
    // ========================================================================

    if wave_count == 0u {
        return in.frag_color;
    }

    // ========================================================================
    // A PARTIR DAQUI:
    // ESTAMOS DENTRO DE UMA INSTÂNCIA DE WAVEFORM.
    //
    // FORA DA ONDA:
    // DISCARD.
    //
    // NUNCA retornamos frag_color como fundo aqui.
    // ========================================================================

    let rect_w = in.size.x;
    let natural_w = in.wave_info.z;

    // Se a waveform natural acabou, não há mais nada para desenhar.
    if natural_w > 0.0 &&
       in.local.x * rect_w > natural_w {
        discard;
    }

    let n = wave_count - 1u;
    let n_f = f32(n);

    var col_f: f32;

    if natural_w > 0.0 {
        col_f =
            (in.local.x * rect_w / natural_w) *
            n_f;
    } else {
        col_f =
            in.local.x *
            n_f;
    }

    if col_f < 0.0 {
        discard;
    }

    if wave_count < 2u {
        discard;
    }

    let col =
        min(
            u32(floor(col_f)),
            wave_count - 2u
        );

    let next_col = col + 1u;

    // ========================================================================
    // MODOS
    //
    // -4 = stereo L
    // -5 = stereo R
    // -6 = mono center
    // ========================================================================

    let is_stereo_l =
        abs(in.wave_info.z - (-4.0)) < 0.1;

    let is_stereo_r =
        abs(in.wave_info.z - (-5.0)) < 0.1;

    // ========================================================================
    // LEITURA DO BUFFER
    // ========================================================================

    let p0_l =
        clamp(
            wave_bars[(wave_offset + col) * 2],
            -1.0,
            1.0
        );

    let p0_r =
        clamp(
            wave_bars[(wave_offset + col) * 2 + 1],
            -1.0,
            1.0
        );

    let p1_l =
        clamp(
            wave_bars[(wave_offset + next_col) * 2],
            -1.0,
            1.0
        );

    let p1_r =
        clamp(
            wave_bars[(wave_offset + next_col) * 2 + 1],
            -1.0,
            1.0
        );

    let p = in.local * in.size;

    let wave_width =
        select(
            in.size.x,
            natural_w,
            natural_w > 0.0
        );

    let x0 =
        f32(col) /
        f32(wave_count - 1u) *
        wave_width;

    let x1 =
        f32(next_col) /
        f32(wave_count - 1u) *
        wave_width;

    // ========================================================================
    // TAPER
    // ========================================================================

    let taper =
        1.0 -
        in.local.x * 0.55;

    let line_width =
        2.2 * taper;

    let wave_radius =
        line_width * 0.5;

    // Cor fixa da waveform.
    let wave_color =
        vec3<f32>(
            0.0194,
            0.0194,
            0.0194
        );

    // ========================================================================
    // SEGMENTO HORIZONTAL
    // ========================================================================

    let segment_width =
        max(
            x1 - x0,
            0.0001
        );

    let t =
        clamp(
            (p.x - x0) /
            segment_width,
            0.0,
            1.0
        );

    // ========================================================================
    // CANAL
    // ========================================================================

    var v0 = p0_l;
    var v1 = p1_l;

    if is_stereo_l {
        v0 = p0_l;
        v1 = p1_l;
    } else if is_stereo_r {
        v0 = p0_r;
        v1 = p1_r;
    }

    // ========================================================================
    // CENTRO / AMPLITUDE
    // ========================================================================

    let body_h = in.size.y;

    let center_y =
        body_h * 0.5;

    let amp_y =
        body_h * 0.4;

    var c = center_y;
    var a = amp_y;

    // ========================================================================
    // TAMANHO DE AMOSTRA EM PIXELS
    // ========================================================================

    let actual_w =
        select(
            natural_w,
            in.size.x,
            natural_w <= 0.0
        );

    let stretch =
        max(
            in.size.x /
            max(actual_w, 0.001),
            1.0
        );

    let px_per_sample =
        clamp(
            (
                actual_w /
                f32(wave_count - 1u)
            ) * stretch,
            0.001,
            1000.0
        );

    // ========================================================================
    // TRANSAÇÃO: ENVELOPE (zoom baixo) ↔ CURVA CATMULL-ROM (zoom alto)
    //
    // env_weight = 1.0 → envelope cheio (preenchimento)
    // env_weight = 0.0 → curva Catmull-ROM (traço fino)
    //
    // O envelope domina até px_per_sample = 1.0.
    // A transição suave dura de 1.0 a 2.5, exigindo mais zoom para o traço.
    // ========================================================================

    let env_weight =
        smoothstep(
            2.5,
            1.0,
            px_per_sample
        );

    // ========================================================================
    // ENVELOPE (preenchimento simétrico, zoom baixo)
    // ========================================================================

    let env_val =
        mix(
            abs(v0),
            abs(v1),
            t
        );

    let y_top =
        c - env_val * a;

    let y_bottom =
        c + env_val * a;

    let env_outside =
        max(
            y_top - p.y,
            p.y - y_bottom
        );

    let env_aa =
        0.35;

    let env_cov =
        max(
            0.0,
            1.0 -
            smoothstep(
                0.0,
                env_aa,
                env_outside
            )
        );

    // ========================================================================
    // CURVA CATMULL-ROM (zoom alto)
    // ========================================================================

    let prev_col =
        max(
            col,
            1u
        ) - 1u;

    let next2_col =
        min(
            col + 2u,
            wave_count - 1u
        );

    // Seleciona o canal correto também para Catmull-Rom.
    var a0 = clamp(
        wave_bars[
            (wave_offset + prev_col) * 2
        ],
        -1.0,
        1.0
    );

    var d3 = clamp(
        wave_bars[
            (wave_offset + next2_col) * 2
        ],
        -1.0,
        1.0
    );

    if is_stereo_r {
        a0 = clamp(
            wave_bars[
                (wave_offset + prev_col) * 2 + 1
            ],
            -1.0,
            1.0
        );

        d3 = clamp(
            wave_bars[
                (wave_offset + next2_col) * 2 + 1
            ],
            -1.0,
            1.0
        );
    }

    let tf =
        clamp(
            col_f - f32(col),
            0.0,
            1.0
        );

    let t2 = tf * tf;
    let t3 = t2 * tf;

    // ========================================================================
    // CATMULL-ROM
    // ========================================================================

    let s =
        0.5 * (
            2.0 * v0 +
            (-a0 + v1) * tf +
            (
                2.0 * a0 -
                5.0 * v0 +
                4.0 * v1 -
                d3
            ) * t2 +
            (
                -a0 +
                3.0 * v0 -
                3.0 * v1 +
                d3
            ) * t3
        );

    let ds =
        0.5 * (
            (-a0 + v1) +
            (
                4.0 * a0 -
                10.0 * v0 +
                8.0 * v1 -
                2.0 * d3
            ) * tf +
            (
                -3.0 * a0 +
                9.0 * v0 -
                9.0 * v1 +
                3.0 * d3
            ) * t2
        );

    let y_curve =
        c - s * a;

    let slope_pp =
        ds *
        (-a) *
        f32(wave_count - 1u) /
        max(
            wave_width,
            0.001
        );

    let d_px =
        abs(
            p.y - y_curve
        ) /
        sqrt(
            1.0 +
            slope_pp * slope_pp
        );

    // ========================================================================
    // ESPESSURA COM TAPER
    // ========================================================================

    let curve_aa =
        clamp(
            fwidth(d_px),
            0.35,
            0.55
        );

    let curve_alpha =
        1.0 -
        smoothstep(
            wave_radius - curve_aa,
            wave_radius + curve_aa,
            d_px
        );

    // ========================================================================
    // BLEND ENTRE ENVELOPE E CURVA
    // ========================================================================

    let final_alpha =
        mix(
            curve_alpha,
            env_cov,
            env_weight
        );

    // Fora da waveform => transparente
    if final_alpha <= 0.001 {
        discard;
    }

    // Somente a waveform chega aqui.
    return vec4<f32>(
        wave_color.r,
        wave_color.g,
        wave_color.b,
        final_alpha
    );
}