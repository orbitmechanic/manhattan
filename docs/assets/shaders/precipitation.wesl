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

// Cheap vertical rain streaks.  Each xz cell owns a thin column of drops that
// scroll downwards; the wind offset shifts the column as height increases so
// the streaks lean into the wind.
fn rain_mask(wp: vec3<f32>, time: f32, wind: f32) -> f32 {
    let slant = vec2<f32>(wind * 0.8, 0.0) * wp.y;
    let p = (wp.xz + slant) * 2.0;
    let cell = floor(p);
    let local = fract(p) - 0.5;
    let r = hash21(cell);
    let column = smoothstep(0.22, 0.02, abs(local.x - (r - 0.5) * 0.55));
    let depth = smoothstep(0.30, 0.04, abs(local.y));
    let speed = 7.0 + 5.0 * hash21(cell + vec2<f32>(4.1, 1.7));
    let fall = fract(wp.y * 0.9 + time * speed + r * 9.0);
    let streak = smoothstep(0.0, 0.04, fall) * (1.0 - smoothstep(0.34, 0.62, fall));
    return column * depth * streak;
}

// Slow, drifting flakes with a short vertical life so they read as falling.
fn snow_mask(wp: vec3<f32>, time: f32, wind: f32) -> f32 {
    let slant = vec2<f32>(wind * 0.25, 0.0) * wp.y;
    let p = (wp.xz + slant) * 1.2;
    let cell = floor(p);
    let local = fract(p) - 0.5;
    let r = hash21(cell + vec2<f32>(9.3, 2.4));
    let drift = vec2<f32>(
        sin(time * 0.7 + r * 6.283) * 0.16,
        cos(time * 0.53 + r * 4.0) * 0.16,
    );
    let flake = smoothstep(0.35, 0.03, length(local - drift));
    let fall = fract(wp.y * 1.3 + time * 0.55 + r * 5.0);
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
    // The cylinder wall is the only geometry, so fade only where the volume
    // is slightly wider than expected rather than at the wall itself.
    let edge = smoothstep(1.35, 1.02, radial);

    // Fade the top and bottom of the volume so its ends are never visible.
    // The mesh is uniformly scaled to sit near the camera's focal distance, so
    // its half-height tracks the radius (the base mesh is 65 wide, 70 tall).
    let half_height = radius * 1.077;
    let norm_y = (wp.y - center.y + half_height) / (2.0 * half_height);
    let height_fade = smoothstep(0.0, 0.16, norm_y) * smoothstep(1.0, 0.84, norm_y);

    let rain = max(
        rain_mask(wp, time, wind),
        rain_mask(wp + vec3<f32>(0.37, 0.0, 0.53), time + 0.13, wind) * 0.85,
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
