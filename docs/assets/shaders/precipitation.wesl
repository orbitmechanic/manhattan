import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

// data:  x = time (s), y = intensity, z = snow mix, w = signed wind drift
// center: xyz = camera position, w = volume radius
// tint:  rgb = streak colour, a = opacity
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> data: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> center: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> tint: vec4<f32>;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        hash21(p),
        hash21(p + vec2<f32>(17.3, 4.1)),
    );
}

fn rotate2(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(p.x * c - p.y * s, p.x * s + p.y * c);
}

// Each xz cell owns one drop, but the drop is jittered inside the cell and two
// rotated layers are combined.  This breaks the visible lattice that plain
// grid rain produces against facade textures.  The fall is deliberately slow
// so the streak direction and wind lean remain visible.
fn rain_layer(wp: vec3<f32>, time: f32, wind: f32, scale: f32, angle: f32) -> f32 {
    let slant = vec2<f32>(wind * 0.55, 0.0) * wp.y;
    let p = rotate2(wp.xz + slant, angle) * scale;
    let cell = floor(p);
    let local = fract(p) - 0.5;
    let rnd = hash22(cell);
    let drop = (rnd - 0.5) * 0.72;
    let width = 0.10 + 0.06 * hash21(cell + vec2<f32>(7.1, 2.3));
    let column = smoothstep(width, 0.0, length(local - drop));

    let fall = fract(wp.y * 0.42 + time * (1.8 + 2.2 * rnd.x) + rnd.y * 9.0);
    let streak = smoothstep(0.0, 0.05, fall) * (1.0 - smoothstep(0.18, 0.52, fall));
    return column * streak;
}

// Slow, drifting flakes with a short vertical life so they read as falling.
fn snow_mask(wp: vec3<f32>, time: f32, wind: f32) -> f32 {
    let slant = vec2<f32>(wind * 0.25, 0.0) * wp.y;
    let p = rotate2(wp.xz + slant, 0.37) * 1.2;
    let cell = floor(p);
    let local = fract(p) - 0.5;
    let r = hash22(cell + vec2<f32>(9.3, 2.4));
    let drift = vec2<f32>(
        sin(time * 0.55 + r.x * 6.283) * 0.16,
        cos(time * 0.43 + r.y * 4.0) * 0.16,
    );
    let flake = smoothstep(0.35, 0.03, length(local - drift));
    let fall = fract(wp.y * 1.1 + time * 0.38 + r.y * 5.0);
    let life = smoothstep(0.0, 0.2, fall) * (1.0 - smoothstep(0.26, 0.52, fall));
    return flake * life;
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let time = data.x;
    let intensity = clamp(data.y, 0.0, 1.0);
    let snow = clamp(data.z, 0.0, 1.0);
    let wind = data.w;

    let wp = in.world_position.xyz;
    let radius = max(center.w, 1.0);
    let radial = length(wp.xz - center.xz) / radius;
    let edge = smoothstep(1.35, 1.02, radial);

    let half_height = radius * 1.077;
    let norm_y = (wp.y - center.y + half_height) / (2.0 * half_height);
    let height_fade = smoothstep(0.0, 0.16, norm_y) * smoothstep(1.0, 0.84, norm_y);

    let rain = max(
        rain_layer(wp, time, wind, 2.0, 0.0),
        rain_layer(wp, time, wind, 3.2, 1.07) * 0.72,
    );
    let flakes = snow_mask(wp, time, wind);
    let coverage = mix(rain, flakes, snow);
    let alpha = coverage * intensity * edge * height_fade * tint.a;

    if alpha < 0.012 {
        discard;
    }

    let colour = mix(vec3<f32>(0.66, 0.74, 0.86), vec3<f32>(0.95, 0.97, 1.0), snow);
    var out: FragmentOutput;
    out.color = vec4<f32>(colour * tint.rgb, alpha);
    return out;
}
