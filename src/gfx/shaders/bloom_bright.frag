#version 330 core
// Bright pass: keeps what is above the threshold, with a soft knee.

in vec2 vUV;
out vec4 frag;

uniform sampler2D uScene;
uniform float uThreshold;

void main() {
    vec3 c = texture(uScene, vUV).rgb;
    float peak = max(c.r, max(c.g, c.b));
    frag = vec4(c * smoothstep(uThreshold, uThreshold + 0.6, peak), 1.0);
}
