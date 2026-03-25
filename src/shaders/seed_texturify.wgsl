const positions: array<vec2<f32>, 3> = array(
    vec2(-1.0, -1.0),
    vec2(3.0, -1.0),
    vec2(-1.0, 3.0),
); // oversized triangle to cover full viewport after clipping

struct JFAConfig {
    jfa_width: f32,       // how many pixels wide the image is
    jfa_height: f32,      // how many pixels high the image is
    jump_size: f32,       // jump size for JFA
    meters_per_px_x: f32, // approximate number of meters per x pixel
    meters_per_px_y: f32, // approximate number of meters per y pixel
}

@group(0) @binding(0) var<uniform> jfa_config: JFAConfig;
@group(0) @binding(1) var out_texture: texture_storage_2d<r32uint, write>;
@group(0) @binding(2) var<storage, read_write> seeds: array<u32>;

const u32_max: u32 = 0xFFFFFFFF;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= arrayLength(&seeds) { return; }

    let xy: vec2u = vec2u(i % u32(jfa_config.jfa_width), i / u32(jfa_config.jfa_width));

    let seed = seeds[i];

    if seed == u32_max {
        return;
    }

    textureStore(out_texture, xy, vec4u(seed, 0u, 0u, 0u));
}
