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

/// [lat, lon, arrival_time, None]
@group(0) @binding(0) var<storage, read> grid_stops: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;
@group(0) @binding(2) var out_texture: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // if gid.x == 0u {
    //     textureStore(out_texture, vec2i(10, 10), vec4f(1.0, 1.0, 1.0, 1.0));
    // }

    let i = gid.x;
    if i >= arrayLength(&grid_stops) { return; }

    var bounding_box_min = vec2(config.bbox_min_lat, config.bbox_min_lon);
    var bounding_box_max = vec2(config.bbox_max_lat, config.bbox_max_lon);

    let stop = grid_stops[i];

    // stop position (normalized where [0,1] is within bounding box)
    let stop_uv = (stop.xy - bounding_box_min) / (bounding_box_max - bounding_box_min);

    // TODO: stops some distance around the bounding box should be included, since they might still be the fastest way to somewhere in the bounding box
    // cull stop if not within bounding box
    if 0 > stop_uv.x || stop_uv.x > 1 || 0 > stop_uv.y || stop_uv.y > 1 { return; }

    // stop poitions (float pixel coordinates)
    let stop_xy = vec2<f32>(
        stop_uv.y * config.width,
        (1.0 - stop_uv.x) * config.height
    );

    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            textureStore(
                out_texture,
                vec2<i32>(i32(stop_xy.x) + dx, i32(stop_xy.y) + dy),
                vec4<f32>(stop_xy.xy, stop.z, 1.0)
            );
        }
    }
}
