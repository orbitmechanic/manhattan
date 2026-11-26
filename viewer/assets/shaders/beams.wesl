import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> data: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> tint: vec4<f32>;

// data: x = time, y = strength, z/w unused.
// World-anchored volume: soft across the width, fading at both ends so the
// shaft reads as a beam leaving the deck and landing on the ground.

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let uv = in.uv;
    // Across: soft edges, brightest in the middle.
    let across = 1.0 - abs(uv.x * 2.0 - 1.0);
    let across_fade = smoothstep(0.0, 0.55, across);
    // Along: the top comes from the cloud (soft), the bottom lands on the
    // ground (softer still) - a beam, not a column with hard ends.
    let along = smoothstep(0.0, 0.18, uv.y) * (1.0 - smoothstep(0.72, 1.0, uv.y));
    let shimmer = 0.85 + 0.15 * sin(data.x * 0.6 + uv.y * 9.0);
    let alpha = across_fade * along * data.y * tint.w * shimmer;

    if alpha < 0.004 {
        discard;
    }

    var out: FragmentOutput;
    out.color = vec4<f32>(tint.rgb, alpha * 0.75);
    return out;
}
