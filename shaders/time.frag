#version 460

layout(location = 0) out vec4 outColor;
layout(push_constant) uniform Globals {
// Width in pixels
    uint uWidth;
// Height in pixels
    uint uHeight;
// Aspect ratio
    float fRatio;
// Time in sec
    float uTime;
// Time since last frame in sec
    float uTimeDelta;
// The number of frame we're at
    uint uFrame;
};

void main() {
    vec2 center = vec2(uWidth / 2, uHeight / 2);
    float r = length(gl_FragCoord.xy - center);
    if (r <= 10) {
        outColor = vec4(1.0, 0.0, 0.0, 1.0);
    } else if (r <= uTimeDelta * 1000.0 * 4.0 + 10) {
        outColor = vec4(0.0, (sin(uTime * 2.0) + 1.0) / 2.0, 0.0, 1.0);
    } else {
        outColor = vec4(0.0, 0.0, 1.0, 1.0);
    }
}