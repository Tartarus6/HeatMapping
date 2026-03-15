struct ShaderConfig {
    width: f32,  // how many pixels wide the image is
    height: f32, // how many pixels tall the image is
    bbox_min_lat: f32,
    bbox_min_lon: f32,
    bbox_max_lat: f32,
    bbox_max_lon: f32,
    gpu_grid_cell_size: f32, // size of each cell (in radians)
    begin_time: f32,         // departure time in seconds since midnight
    // TODO: fix max time
    max_time: f32,               // latest arrival time in seconds since midnight
    inverse_walk_speed_mps: f32, // walking speed in seconds per meter
    jump_size: f32,              // jump size for JFA
}

@group(0) @binding(0) var prev_texture: texture_storage_2d<rgba16float, read>;  // read-only
@group(0) @binding(1) var next_texture: texture_storage_2d<rgba16float, write>; // write-only
@group(0) @binding(2) var<uniform> config: ShaderConfig;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= u32(config.width) || gid.y >= u32(config.height) { return; }

    let point = vec2<i32>(i32(gid.x), i32(gid.y));

    // current best (from prev_texture)
    var best = vec4<f32>(0, 0, 0, 0);
    var bestDist = 1e30; // initialize to high value to override later

    // TODO: is there a fancier, better way to iterate through the 9 neighbors?
    // check 8 neighbors at distance jump
    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            let neighbor_point = point + vec2<i32>(dx * i32(config.jump_size), dy * i32(config.jump_size));

            // skip if neighbor not in bounds
            if !(in_bounds(neighbor_point)) {
                continue;
            }

            let candidate = load_seed(neighbor_point);
            if candidate.z == 0.0 {
                continue;
            }

            let delta = vec2<f32>(candidate.x - f32(point.x), candidate.y - f32(point.y));

            let dist = dot(delta, delta);

            if dist < bestDist {
                bestDist = dist;
                best = candidate;
            }
        }
    }

    textureStore(next_texture, point, vec4<f32>(best.xy, best.z, 1.0));
}

fn in_bounds(point: vec2<i32>) -> bool {
    return 0 <= point.x && point.x < i32(config.width) &&
           0 <= point.y && point.y < i32(config.height);
}

// TODO: change this to vec4 maybe?
// returns (x, y, validity, None) (1=valid, 0=invalid)
fn load_seed(point: vec2<i32>) -> vec4<f32> {
    let v = textureLoad(prev_texture, point);
    return v;
}
