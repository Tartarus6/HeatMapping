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
@group(0) @binding(4) var<storage, read> grid_stops: array<vec4<f32>>;

const HALF_SQRT_2: f32 = 0.707106781;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= u32(jfa_config.jfa_width) || gid.y >= u32(jfa_config.jfa_height) { return; }

    // the position of current pixel
    let point = vec2<i32>(i32(gid.x), i32(gid.y));

    // index current best stop (from prev_texture)
    var best_index: u32 = 0u;
    var best_arrival_time: u32 = u32(config.max_time);

    // TODO: is there a fancier, better way to iterate through the 9 neighbors?
    // check 8 neighbors at distance jump
    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            // octagonal neighbor pattern (looks a bit better than using square neighbors)
            // var delta_px: vec2i;
            // if abs(dx) + abs(dy) == 2 { // if we're on a diagonal neighbor
            //     // then scale delta by half sqrt(2) in order to normalize distance to 1
            //     delta_px = vec2<i32>(i32(f32(dx) * jfa_config.jump_size * HALF_SQRT_2), i32(f32(dy) * jfa_config.jump_size * HALF_SQRT_2));
            // } else {
            //     delta_px = vec2<i32>(dx * i32(jfa_config.jump_size), dy * i32(jfa_config.jump_size));
            // }

            // square neighbor pattern (standard JFA shape)
            let delta_px = vec2<i32>(dx * i32(jfa_config.jump_size), dy * i32(jfa_config.jump_size));

            let neighbor_point = point + delta_px;

            // skip if neighbor not in bounds
            if !(in_bounds(neighbor_point)) {
                continue;
            }

            // get neighbor point arrival time as new candidate to get to current point
            var candidate_stop_index: u32 = load_seed(neighbor_point);
            // if candudate "candidate_stop_index" is zero, that means it's invalid (because pixel value is always stored with a +1 offset when it's valid, so a valid pixel can't be zero)
            if candidate_stop_index == 0 {
                continue;
            }

            candidate_stop_index -= 1; // removing offset

            // get candidate stop (lat, lon, arrival_time, _)
            let candidate_stop = grid_stops[candidate_stop_index];

            // normalize candidate stop position vec2f([0,1], [0,1])
            let u: f32 = (candidate_stop.y - config.bbox_min_lon) / (config.bbox_max_lon - config.bbox_min_lon);
            let v: f32 = 1.0 - (candidate_stop.x - config.bbox_min_lat) / (config.bbox_max_lat - config.bbox_min_lat);
            let candidate_stop_norm: vec2f = vec2f(u, v);
            // get candidate stop pixel vec2i(x, y)
            let candidate_stop_pixel: vec2i = vec2i(candidate_stop_norm * vec2f(jfa_config.jfa_width, jfa_config.jfa_height));

            // TODO: could precompute sqrt(2) and use that as a factor when diagonal, or just use the x or y if orthogonal (to prevent need to use length())
            // approx. distance in meters between this point and candidate point
            let dist: f32 = length(vec2f(candidate_stop_pixel - point) * vec2f(jfa_config.meters_per_px_x, jfa_config.meters_per_px_y));

            // walk time in seconds between this pixel and candidate pixel
            let walk_s = dist * config.inverse_walk_speed_mps;

            // arrival time if walking from candidate pixel to current pixel
            let total: f32 = candidate_stop.z + walk_s;

            // update best arrival time if new path is better
            if total < f32(best_arrival_time) {
                best_index = candidate_stop_index + 1;
                best_arrival_time = u32(total);
            }
        }
    }

    // update pixel with new best arrival time
    textureStore(next_texture, point, vec4(best_index, 0u, 0u, 0u));
}

fn in_bounds(point: vec2<i32>) -> bool {
    return 0 <= point.x && point.x < i32(jfa_config.jfa_width) &&
           0 <= point.y && point.y < i32(jfa_config.jfa_height);
}

// returns arrival time of pixel at given (x, y) screen-coordinate point
fn load_seed(point: vec2<i32>) -> u32 {
    let v = textureLoad(prev_texture, point);
    return v.x;
}
