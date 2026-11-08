#version 330 core
// Soft round sprite with a gaussian core, drawn additively into the HDR target.

in vec3 vColor;
out vec4 frag;

uniform float uBrightness;

void main() {
    vec2 d = gl_PointCoord - vec2(0.5);
    float r2 = dot(d, d);
    if (r2 > 0.25) discard;      // round sprite
    float a = exp(-r2 * 11.0);   // gaussian falloff
    frag = vec4(vColor * uBrightness * a, a);
}
