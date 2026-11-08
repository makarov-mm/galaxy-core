#version 330 core
// One direction of a separable 9-tap gaussian blur.

in vec2 vUV;
out vec4 frag;

uniform sampler2D uTex;
uniform vec2 uDir;   // one texel along x or y

const float W0 = 0.227027;
const float W[4] = float[](0.1945946, 0.1216216, 0.054054, 0.016216);

void main() {
    vec3 sum = texture(uTex, vUV).rgb * W0;
    for (int i = 0; i < 4; i++) {
        vec2 o = uDir * float(i + 1);
        sum += (texture(uTex, vUV + o).rgb + texture(uTex, vUV - o).rgb) * W[i];
    }
    frag = vec4(sum, 1.0);
}
