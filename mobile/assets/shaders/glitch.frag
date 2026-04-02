#version 460 core
#include <flutter/runtime_effect.glsl>

uniform float uTime;
uniform vec2 uResolution;
out vec4 fragColor;

float hash(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

void main() {
    vec2 uv = FlutterFragCoord().xy / uResolution;
    
    float noise = hash(uv + uTime);
    float line_glitch = step(0.98, hash(vec2(uTime, floor(uv.y * 50.0))));
    
    vec3 base_color = vec3(0.02, 0.02, 0.05); // Dark Blue/Black
    // Cyberpunk Neon Green: #39FF14
    vec3 neon_green = vec3(0.22, 1.0, 0.08);
    // Magenta
    vec3 neon_magenta = vec3(1.0, 0.0, 1.0);
    
    float scanline = sin(uv.y * 800.0) * 0.04;
    
    vec3 color = mix(base_color, neon_green, line_glitch * 0.5);
    color = mix(color, neon_magenta, step(0.99, hash(vec2(uTime, floor(uv.y * 10.0)))) * 0.3);
    
    color += scanline;
    
    fragColor = vec4(color, 1.0);
}
