const positions: array<vec2<f32>, 3> = array(
    vec2(-1.0, -1.0),
    vec2(3.0, -1.0),
    vec2(-1.0, 3.0),
); // oversized triangle to cover full viewport after clipping

struct GpuGridCellKey {
    lat: i32,
    lon: i32,
}

struct GpuGridCellVal {
    start: u32,
    count: u32,
}

struct ShaderConfig {
    width: u32,  // how many pixels wide the image is
    height: u32, // how many pixels high the image is
    bbox_min_lat: f32,
    bbox_min_lon: f32,
    bbox_max_lat: f32,
    bbox_max_lon: f32,
    max_walk_transfer_distance: f32, // maximum distance to walk between stops (used for culling) (this option can be too greedy, it can cull optimal paths) (distance in meters)
    inverse_walk_speed_mps: f32,     // walking speed in seconds per meter
}

struct JFAConfig {
    jfa_width: u32,       // how many pixels wide the image is
    jfa_height: u32,      // how many pixels high the image is
    jump_size: u32,       // jump size for JFA
    meters_per_px_x: f32, // approximate number of meters per x pixel
    meters_per_px_y: f32, // approximate number of meters per y pixel
}

struct GpuStop {
    /// Latitude
    lat: f32,
    /// Longitude
    lon: f32,
    /// Arrival time to stop in seconds since midnight
    arrival_time: u32,
    /// Just padding to 16-byte allignment, not for use
    _pad0: u32,
}

@group(0) @binding(0) var<storage, read> grid_stops: array<GpuStop>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;
@group(0) @binding(2) var<uniform> jfa_config: JFAConfig;
@group(0) @binding(3) var<storage, read_write> seeds: array<atomic<u32>>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= arrayLength(&grid_stops) { return; }

    let stop = grid_stops[i];

    // TODO: stops some distance around the bounding box should be included, since they might still be the fastest way to somewhere in the bounding box
    // cull stop if not within bounding box
    if config.bbox_min_lat > stop.lat || stop.lat > config.bbox_max_lat ||
        config.bbox_min_lon > stop.lon || stop.lon > config.bbox_max_lon { return; }

    var bounding_box_min = vec2f(config.bbox_min_lat, config.bbox_min_lon);
    var bounding_box_max = vec2f(config.bbox_max_lat, config.bbox_max_lon);

    // stop position (normalized where [0,1] is within bounding box)
    let stop_uv = (vec2f(stop.lat, stop.lon) - bounding_box_min) / (bounding_box_max - bounding_box_min);

    // stop poitions (float pixel coordinates)
    let stop_x = u32(stop_uv.y * f32(jfa_config.jfa_width));
    let stop_y = u32((1.0 - stop_uv.x) * f32(jfa_config.jfa_height));

    // store stop index in texture
    let packed: vec4<u32> = vec4<u32>(i + 1, 0u, 0u, 0u); // offset added to differentiate index 0 from a cleared pixel

    // bounds guard
    if stop_x < 0 || stop_y < 0 || stop_x >= jfa_config.jfa_width || stop_y >= jfa_config.jfa_height {
        return;
    }

    try_claim(stop_x + jfa_config.jfa_width * stop_y, i + 1u);
}

fn is_better(my_idx1: u32, cur_idx1: u32) -> bool {
    // idx1 is 1-based stored index; convert to 0-based for stop lookup
    let my_idx0 = my_idx1 - 1u;
    let cur_idx0 = cur_idx1 - 1u;

    let my_arr = grid_stops[my_idx0].arrival_time;
    let cur_arr = grid_stops[cur_idx0].arrival_time;

    // tie-breaker for determinism: lower index wins
    return (my_arr < cur_arr) || (my_arr == cur_arr && my_idx1 < cur_idx1);
}

fn try_claim(pixel_i: u32, my_idx1: u32) {
    loop {
        let cur = atomicLoad(&seeds[pixel_i]);

        if cur == 0 {
            // Note: as of writing this comment, wgsl analyzer does not think `atomicCompareExchangeWeak` is a valid function. But it is though
            let r = atomicCompareExchangeWeak(&seeds[pixel_i], 0, my_idx1);
            if r.exchanged { break; }
            continue;
        }

        if !is_better(my_idx1, cur) {
            break; // current winner is better (or equal with tie-break)
        }

        // Note: as of writing this comment, wgsl analyzer does not think `atomicCompareExchangeWeak` is a valid function. But it is though
        let r = atomicCompareExchangeWeak(&seeds[pixel_i], cur, my_idx1);
        if r.exchanged { break; }
        // else: lost race, retry
    }
}
