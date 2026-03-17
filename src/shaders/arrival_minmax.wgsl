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
    max_walk_transfer_distance: f32, // maximum distance to walk between stops (used for culling) (this option can be too greedy, it can cull optimal paths) (distance in meters)
    inverse_walk_speed_mps: f32,     // walking speed in seconds per meter
}

struct JFAConfig {
    jfa_width: f32,       // how many pixels wide the image is
    jfa_height: f32,      // how many pixels high the image is
    jump_size: f32,       // jump size for JFA
    meters_per_px_x: f32, // approximate number of meters per x pixel
    meters_per_px_y: f32, // approximate number of meters per y pixel
}

struct MinMax {
    min_time: atomic<u32>,
    max_time: atomic<u32>,
}

@group(0) @binding(0) var jfa_tex: texture_2d<u32>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;
@group(0) @binding(2) var<uniform> jfa_config: JFAConfig;
@group(0) @binding(3) var<storage, read> grid_stops: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> minmax: MinMax;

// TODO: reduce minmax instability (panning around can lead to some sudden gradient changes)

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= u32(jfa_config.jfa_width) || gid.y >= u32(jfa_config.jfa_height) {
        return;
    }

    let xy = vec2u(gid.xy);
    var best_stop_index: u32 = textureLoad(jfa_tex, vec2i(xy), 0).x;

    // skip unreachable pixels
    if best_stop_index == 0u {
        return;
    }

    best_stop_index -= 1u;
    let best_stop = grid_stops[best_stop_index];

    let u = (best_stop.y - config.bbox_min_lon) / (config.bbox_max_lon - config.bbox_min_lon);
    let v = 1.0 - (best_stop.x - config.bbox_min_lat) / (config.bbox_max_lat - config.bbox_min_lat);
    let best_stop_pixel = vec2i(vec2f(u, v) * vec2f(jfa_config.jfa_width, jfa_config.jfa_height));

    let dist = length(vec2f(best_stop_pixel - vec2i(xy)) * vec2f(jfa_config.meters_per_px_x, jfa_config.meters_per_px_y));

    // skip pixels that are really far from any stops
    if dist > config.max_walk_transfer_distance {
        return;
    }

    let walk_s = u32(dist * config.inverse_walk_speed_mps);
    let arrival = u32(best_stop.z) + walk_s;

    atomicMin(&minmax.min_time, arrival);
    atomicMax(&minmax.max_time, arrival);
}
