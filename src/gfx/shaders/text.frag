#version 330 core
// Glyph coverage from the single-channel atlas, tinted.

in vec2 vUV;
out vec4 frag;

uniform sampler2D uAtlas;
uniform vec4 uColor;

void main() {
    frag = vec4(uColor.rgb, uColor.a * texture(uAtlas, vUV).r);
}
