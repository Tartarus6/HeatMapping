struct ShaderConfig {
    width: f32,  // how many pixels wide the image is
    height: f32, // how many pixels high the image is
    bbox_min_lat: f32,
    bbox_min_lon: f32,
    bbox_max_lat: f32,
    bbox_max_lon: f32,
    gpu_grid_cell_size: f32, // size of each cell (in radians)
    begin_time: f32,         // departure time in seconds since midnight
    // TODO: fix max time
    max_time: f32,               // latest arrival time in seconds since midnight
    inverse_walk_speed_mps: f32, // walking speed in seconds per meter
}

struct JFAConfig {
    jfa_width: f32,       // how many pixels wide the image is
    jfa_height: f32,      // how many pixels high the image is
    jump_size: f32,       // jump size for JFA
    meters_per_px_x: f32, // approximate number of meters per x pixel
    meters_per_px_y: f32, // approximate number of meters per y pixel
}

@group(0) @binding(0) var prev_texture: texture_storage_2d<r32uint, read>;  // read-only
@group(0) @binding(1) var next_texture: texture_storage_2d<r32uint, write>; // write-only
@group(0) @binding(2) var<uniform> config: ShaderConfig;
@group(0) @binding(3) var<uniform> jfa_config: JFAConfig;

// fn unpack_xy(packed: u32) -> vec2<u32> {
//     return vec2(packed & 0xffffu, (packed >> 16u) & 0xffffu);
// }

// fn pack_xy_u16(x: u32, y: u32) -> u32 {
//     // low 16 bits = x, high 16 bits = y
//     return (x & 0xffffu) | ((y & 0xffffu) << 16u);
// }

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= u32(jfa_config.jfa_width) || gid.y >= u32(jfa_config.jfa_height) { return; }

    let point = vec2<i32>(i32(gid.x), i32(gid.y));

    // current best (from prev_texture)
    var best: u32 = u32(config.max_time);

    // TODO: is there a fancier, better way to iterate through the 9 neighbors?
    // check 8 neighbors at distance jump
    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            let neighbor_point = point + vec2<i32>(dx * i32(jfa_config.jump_size), dy * i32(jfa_config.jump_size));

            // skip if neighbor not in bounds
            if !(in_bounds(neighbor_point)) {
                continue;
            }

            let candidate: u32 = load_seed(neighbor_point);
            if candidate == 0 { // if candudate "arrival_time" is zero, it's probably invalid (just an uninitialized pixel)
                continue;
            }

            // let delta = vec2<i32>(candidate.xy) - point;
            let delta: vec2f = vec2f(f32(dx) * jfa_config.jump_size, f32(dy) * jfa_config.jump_size) * vec2f(jfa_config.meters_per_px_x, jfa_config.meters_per_px_y);

            let dist: f32 = length(delta);

            let walk_s = u32(dist * config.inverse_walk_speed_mps);
            let total: u32 = candidate + walk_s;

            if total < best {
                best = total;
            }
        }
    }

    textureStore(next_texture, point, vec4(best, 0u, 0u, 0u));
}

fn in_bounds(point: vec2<i32>) -> bool {
    return 0 <= point.x && point.x < i32(jfa_config.jfa_width) &&
           0 <= point.y && point.y < i32(jfa_config.jfa_height);
}

// TODO: change this to vec4 maybe?
// returns (x, y, validity, None) (1=valid, 0=invalid)
fn load_seed(point: vec2<i32>) -> u32 {
    let v = textureLoad(prev_texture, point);
    // let xy = unpack_xy(v.x);
    return v.x;
}
