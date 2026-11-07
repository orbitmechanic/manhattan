import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> data: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> tint: vec4<f32>;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let uv = in.uv * 2.0 - vec2<f32>(1.0, 1.0);
    let center = vec2<f32>(data.z, data.w) * 0.35;
    let p = uv - center;
    let r = length(p);
    if r < 0.04 {
        discard;
    }

    let angle = atan2(p.y, p.x);
    let spokes = 72.0;
    let spoke = angle / 6.2831853 * spokes;
    let cell = floor(spoke);
    let rnd = hash21(vec2<f32>(cell, 3.7));
    let edge = abs(fract(spoke + rnd * 0.35) - 0.5);
    let line = smoothstep(0.15, 0.005, edge);

    // Each spoke recurs along the radius at a different rate so the lines are
    // not a single uniform ring.
    let phase = fract(r * (2.2 + 3.0 * rnd) - data.x * (1.2 + 2.0 * rnd) + rnd * 7.0);
    let streak = smoothstep(0.0, 0.06, phase) * (1.0 - smoothstep(0.20, 0.52, phase));
    let fade = smoothstep(0.12, 0.65, r) * smoothstep(1.15, 0.75, r);
    let alpha = line * streak * fade * data.y * tint.w * 1.5;
    if alpha < 0.008 {
        discard;
    }

    var out: FragmentOutput;
    out.color = vec4<f32>(tint.rgb, alpha);
    return out;
}
