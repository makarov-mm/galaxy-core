#version 330 core
// One star per point: position + speed. Speed picks a colour on a
// cold -> warm -> hot ramp; size falls off with distance.

layout(location = 0) in vec3 aPos;
layout(location = 1) in float aSpeed;

uniform mat4 uViewProj;
uniform float uPointScale;   // sprite size in pixels at clip w = 1
uniform float uSpeedScale;   // 1 / reference speed

out vec3 vColor;

void main() {
    vec4 clip = uViewProj * vec4(aPos, 1.0);
    gl_Position = clip;
    gl_PointSize = clamp(uPointScale / max(clip.w, 0.001), 1.0, 48.0);

    float t = clamp(aSpeed * uSpeedScale, 0.0, 1.4);
    const vec3 cold = vec3(0.30, 0.45, 1.00);
    const vec3 warm = vec3(1.00, 0.95, 0.86);
    const vec3 hot  = vec3(1.00, 0.78, 0.45);
    vColor = mix(mix(cold, warm, smoothstep(0.0, 0.55, t)), hot, smoothstep(0.75, 1.25, t));
}
