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

// Sample one tile from a grid atlas.  `local` is already in 0..1 tile space;
// the inset keeps linear filtering away from the neighbouring tile.
fn sample_atlas(
    tex: texture_2d<f32>,
    samp: sampler,
    index: u32,
    cols: u32,
    rows: u32,
    local: vec2<f32>,
    inset: vec2<f32>,
) -> vec3<f32> {
    let safe = min(index, cols * rows - 1u);
    let origin = vec2<f32>(f32(safe % cols), f32(safe / cols));
    let inner = mix(inset, vec2<f32>(1.0) - inset, local);
    let uv = (origin + inner) * vec2<f32>(1.0 / f32(cols), 1.0 / f32(rows));
    return textureSample(tex, samp, uv).rgb;
}

struct BlockStates {
    values: array<vec4<f32>, 1536>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> block_states: BlockStates;

@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var legacy_night_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var legacy_night_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(103)
var<uniform> emissive_strength: vec4<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(104)
var facade_day_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(105)
var facade_day_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(106)
var facade_night_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(107)
var facade_night_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(108)
var roof_day_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(109)
var roof_day_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(110)
var roof_night_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(111)
var roof_night_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(114)
var facade_night_b_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(115)
var facade_night_b_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(116)
var facade_night_all_texture: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(117)
var facade_night_all_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(112)
var<uniform> atlas_info: vec4<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(113)
var<uniform> atlas_uv: vec4<f32>;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // COLOR_0 RGB carries the per-building facade/roof choice, tint and U
    // offset.  Alpha carries the block id plus a few per-building flags:
    //   bits 0..10 block id, bit 11 financial, bits 12..14 height bucket,
    //   bit 15 signage-eligible neighbourhood.
    let is_ground = emissive_strength.z > 0.5;
    let packed_color = u32(round(in.color.a * 65535.0));
    var block_id = 0i;
    var extra = 0u;
    if (!is_ground) {
        block_id = clamp(i32(packed_color & 2047u), 0, 1535);
        extra = packed_color >> 11u;
    }
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

    var day = pbr_input.material.base_color.rgb;
    var night = vec3<f32>(0.0);
    var tint = vec3<f32>(1.0);

    if (is_ground) {
        night = textureSample(legacy_night_texture, legacy_night_sampler, in.uv).rgb;
    } else {
        let packed =
            u32(round(in.color.r * 255.0)) * 65536u
            + u32(round(in.color.g * 255.0)) * 256u
            + u32(round(in.color.b * 255.0));
        let facade_index = packed & 511u;
        let roof_index = (packed >> 9u) & 255u;
        let tint_t = f32((packed >> 17u) & 7u) / 7.0;
        let offset = f32((packed >> 20u) & 15u) / 16.0;
        tint = mix(vec3<f32>(0.9, 0.92, 0.94), vec3<f32>(1.08, 1.04, 1.0), tint_t);

        let cols = u32(atlas_info.x);
        let rows = u32(atlas_info.y);
        let roof_cols = u32(atlas_info.z);
        let roof_rows = u32(atlas_info.w);
        let is_roof = abs(pbr_input.world_normal.y) > 0.55;

        if (is_roof) {
            let local = fract(in.uv * atlas_uv.w + vec2<f32>(offset));
            day = sample_atlas(
                roof_day_texture, roof_day_sampler, roof_index,
                roof_cols, roof_rows, local, vec2<f32>(atlas_uv.z)
            ) * tint;
            night = sample_atlas(
                roof_night_texture, roof_night_sampler, roof_index,
                roof_cols, roof_rows, local, vec2<f32>(atlas_uv.z)
            ) * tint;
        } else {
            let local = vec2<f32>(
                fract(in.uv.x + offset),
                clamp(in.uv.y, 0.0, 1.0)
            );
            day = sample_atlas(
                facade_day_texture, facade_day_sampler, facade_index,
                cols, rows, local, vec2<f32>(atlas_uv.x, atlas_uv.y)
            ) * tint;
            let financial = (extra & 1u) != 0u;
            let height_bucket = (extra >> 1u) & 7u;
            let floor_variant = floor_random(u32(block_id), floor_index, salt ^ 0x9E37u);
            let all_lit = financial && height_bucket >= 2u && floor_variant.y < 0.35;
            if (all_lit) {
                night = sample_atlas(
                    facade_night_all_texture, facade_night_all_sampler, facade_index,
                    cols, rows, local, vec2<f32>(atlas_uv.x, atlas_uv.y)
                ) * tint * 1.15;
            } else if (floor_variant.x < 0.5) {
                night = sample_atlas(
                    facade_night_texture, facade_night_sampler, facade_index,
                    cols, rows, local, vec2<f32>(atlas_uv.x, atlas_uv.y)
                ) * tint;
            } else {
                night = sample_atlas(
                    facade_night_b_texture, facade_night_b_sampler, facade_index,
                    cols, rows, local, vec2<f32>(atlas_uv.x, atlas_uv.y)
                ) * tint;
            }
        }
    }

    pbr_input.material.base_color = vec4<f32>(day, 1.0);
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
