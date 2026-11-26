import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> data: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> tint: vec4<f32>;

// data: x = time (s), y = strength 0..1, z/w = sun or moon position in
// screen space (-1..1, y up). Rays radiate FROM that point: the convergence
// the eye reads at the horizon is perspective, and it comes for free here.

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let uv = in.uv * 2.0 - vec2<f32>(1.0, 1.0);
    let origin = vec2<f32>(data.z, data.w);
    let d = uv - origin;
    let r = length(d);

    // Rays are long and soft: a slow angular fan with per-ray random phase, so
    // the fan reads as separate beams rather than a halo.
    let angle = atan2(d.y, d.x);
    let spokes = 26.0;
    let spoke = angle / 6.2831853 * spokes;
    let cell = floor(spoke);
    let rnd = hash21(vec2<f32>(cell, 7.3));
    let edge = abs(fract(spoke + rnd * 0.55) - 0.5);
    let width = 0.06 + 0.05 * rnd;
    let line = smoothstep(width, 0.0, edge);

    // Fade in away from the source, out at the frame edge.
    let reach = smoothstep(0.05, 0.35, r) * (1.0 - smoothstep(0.75, 1.45, r));
    // Slow shimmer along the ray so the fan breathes.
    let shimmer = 0.75 + 0.25 * sin(data.x * 0.35 + rnd * 12.0);
    let alpha = line * reach * shimmer * data.y * tint.w;

    if alpha < 0.004 {
        discard;
    }

    var out: FragmentOutput;
    out.color = vec4<f32>(tint.rgb, alpha);
    return out;
}
