import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

// data:  x = time (s), y = cloud cover 0..1, z/w = wind drift (x, z)
// lit:   rgb = sunlit cloud colour, a = layer opacity
// shade: rgb = cloud shadow colour
// extra: x = lightning flash, y = noise scale (metres), z/w unused
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> data: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> lit: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> shade: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<uniform> extra: vec4<f32>;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Three octaves keep the cloudscape soft while staying cheap enough for the
// web build; two full-screen cloud layers share this shader.
fn fbm(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var point = p;
    for (var i = 0; i < 3; i += 1) {
        value += amplitude * value_noise(point);
        point = point * 2.03 + vec2<f32>(1.7, 9.2);
        amplitude *= 0.5;
    }
    return value;
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    let scale = max(extra.y, 1.0);
    let wind_offset = data.zw * data.x * 0.0016;
    let p = in.world_position.xz / scale + wind_offset;
    let n = fbm(p * 1.45) * 0.72 + fbm(p * 3.2 + vec2<f32>(11.3, 7.1)) * 0.28;

    let cover = clamp(data.y, 0.0, 1.0);
    let threshold = 0.80 - cover * 0.65;
    let alpha = smoothstep(threshold, threshold + 0.16, n)
        * (0.20 + 0.55 * cover)
        * lit.a;

    // The deck is a disc; fade its rim so the cloud sheet never shows a hard
    // circular or square border at the horizon.
    let radial = length(in.uv - vec2<f32>(0.5, 0.5)) * 2.0;
    let rim_fade = 1.0 - smoothstep(0.72, 0.98, radial);
    let faded_alpha = alpha * rim_fade;

    if faded_alpha < 0.008 {
        discard;
    }

    let body = mix(shade.rgb, lit.rgb, smoothstep(0.24, 0.86, n));
    // The ragged edge catches a little extra light, like a sunlit cloud rim.
    let rim = smoothstep(0.12, 0.0, abs(n - threshold - 0.16));
    var colour = body + rim * lit.rgb * 0.10;
    colour += vec3<f32>(extra.x * 0.85);

    var out: FragmentOutput;
    out.color = vec4<f32>(colour, faded_alpha);
    return out;
}
