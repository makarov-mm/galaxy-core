#version 330 core
// Scene + bloom, Reinhard tone mapped (black stays black).

in vec2 vUV;
out vec4 frag;

uniform sampler2D uScene;
uniform sampler2D uBloom;
uniform float uBloomIntensity;

void main() {
    vec3 c = texture(uScene, vUV).rgb + texture(uBloom, vUV).rgb * uBloomIntensity;
    frag = vec4(c / (c + vec3(1.0)), 1.0);
}
