// ============================================================
// Waveform GPU Instanced Rendering Shaders
// ============================================================
// Vertex Shader: expande 1 quad unitário → N barras via instanced data
// Fragment Shader: desenha barra vertical min→max com anti-alias
// ============================================================

// Vertex Shader
pub const VERTEX_SHADER: &str = r#"
#version 330 core

layout(location = 0) in vec2 a_pos;      // quad unitário: (-1,-1) a (1,1)
layout(location = 1) in float a_min;     // instance: min (-1.0 a 0.0)
layout(location = 2) in float a_max;     // instance: max (0.0 a 1.0)

layout(location = 0) out float v_min;
layout(location = 1) out float v_max;
layout(location = 2) out float v_y_rel;  // posição Y relativa no quad (0..1)

uniform mat4 u_mvp;                      // model-view-projection por clip
uniform int u_instance_count;            // total de instâncias deste clip

void main() {
    v_min = a_min;
    v_max = a_max;
    v_y_rel = (a_pos.y + 1.0) * 0.5;  // 0.0 (base) a 1.0 (topo)
    
    // Posição X: gl_InstanceID determina coluna
    float instance_count_f = float(u_instance_count);
    float col_x = (float(gl_InstanceID) / instance_count_f) * 2.0 - 1.0;
    float col_width = 2.0 / instance_count_f;
    
    // Quad unitário escalado para largura da coluna
    vec4 pos = vec4(col_x + a_pos.x * col_width * 0.5, a_pos.y, 0.0, 1.0);
    gl_Position = u_mvp * pos;
}
"#;

// Fragment Shader
pub const FRAGMENT_SHADER: &str = r#"
#version 330 core

in float v_min;
in float v_max;
in float v_y_rel;

layout(location = 0) out vec4 out_color;

uniform vec4 u_color;      // cor da waveform (RGBA)
uniform float u_clip_top;  // topo do clip em NDC
uniform float u_clip_bottom; // base do clip em NDC

void main() {
    // v_y_rel: 0.0 (base) → 1.0 (topo)
    // min é negativo/zero, max é positivo/zero
    float upper = v_max;      // 0.0 a 1.0
    float lower = v_min;      // -1.0 a 0.0
    
    // Desenha apenas se v_y_rel estiver entre lower e upper
    float in_bar = step(lower, v_y_rel) * step(v_y_rel, upper);
    
    // Anti-alias nas bordas (~1px em NDC)
    float aa = 0.0;
    if (in_bar > 0.0) {
        float dist_to_edge = min(v_y_rel - lower, upper - v_y_rel);
        aa = smoothstep(0.0, 1.0/512.0, dist_to_edge);
    }
    
    out_color = u_color * vec4(1.0, 1.0, 1.0, aa);
}
"#;