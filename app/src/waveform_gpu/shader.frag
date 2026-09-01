// Waveform GPU Fragment Shader
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