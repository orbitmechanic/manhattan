import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
};

@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn hash_u32(h: u32) -> u32 {
    var x = h;
    x += x >> 16u;
    x = x * 0x7feb352du;
    x += x >> 15u;
    x = x * 0x846ca68bu;
    x += x >> 16u;
    return x;
}

// Per-floor threshold / width derived from the block id, floor index and the
// current penumbra salt.  World Y is used because vertex colours interpolate
// across the facade and would smear the floors together.
fn floor_random(block: u32, fl: i32, salt: u32) -> vec2<f32> {
    var h = block * 0x9E3779B9u + u32(fl) * 0x85EBCA6Bu + salt;
    h = hash_u32(h);
    return vec2<f32>(
        f32(h & 0xFFFFu) / 65535.0,
        f32((h >> 16u) & 0xFFFFu) / 65535.0,
    );
}

struct BlockStates {
    values: array<vec4<f32>, 1536>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> block_states: BlockStates;

@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var night_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var night_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(103)
var<uniform> emissive_strength: vec4<f32>;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Block id is baked into COLOR_0 alpha by the Blender export.  On LOD0
    // floors the RGB channels carry a per-floor threshold/width pair, so each
    // floor crosses over at its own randomised rate while the block blends.
    let block_id = clamp(i32(round(in.color.a * 1536.0 - 0.5)), 0, 1535);
    let state = block_states.values[block_id];
    let block_blend = state.x;
    let progress = state.y;
    let salt = u32(max(state.z, 0.0));
    let floor_flag = emissive_strength.y;

    let floor_index = i32(floor(pbr_input.world_position.y / 3.0));
    let rnd = floor_random(u32(block_id), floor_index, salt);
    let floor_threshold = 0.12 + rnd.x * 0.76;
    let floor_width = 0.03 + rnd.y * 0.12;
    let floor_t = smoothstep(floor_threshold - floor_width, floor_threshold + floor_width, progress);
    let blend = block_blend * mix(1.0, floor_t, floor_flag);

    let day = pbr_input.material.base_color.rgb;
    let night = textureSample(night_texture, night_sampler, in.uv).rgb;

    pbr_input.material.base_color = vec4<f32>(mix(day, night, blend), 1.0);
    pbr_input.material.emissive = vec4<f32>(night * blend * emissive_strength.x, 0.0);

    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );

    var out: FragmentOutput;
@if(PREPASS_PIPELINE) {
    out = deferred_output(in, pbr_input);
} @else {
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
}
    return out;
}
