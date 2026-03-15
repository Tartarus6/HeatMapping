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
    max_time: f32, // latest arrival time in seconds since midnight
    inverse_walk_speed_mps: f32, // walking speed in meters per second
}

/// [lat, lon, arrival_time, None]
@group(0) @binding(0) var<storage, read> grid_stops: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;
@group(0) @binding(2) var seedTex: texture_storage_2d<rgba32sint, write>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= arrayLength(&grid_stops) { return; }

    var bounding_box_min = vec2(config.bbox_min_lat, config.bbox_min_lon);
    var bounding_box_max = vec2(config.bbox_max_lat, config.bbox_max_lon);

    let stop = grid_stops[i];

    // normalized in[0,1]
    let stop_uv = (vec2(stop.x, stop.y) - bounding_box_min) / (bounding_box_max - bounding_box_min);

    // convert to pixel coords (float)
    // Careful: decide your exact orientation once.
    // Example: lat -> y, lon -> x is common:
    let stop_xy = vec2<f32>(
        stop_uv.y * config.width,
        stop_uv.x * config.height
    );

    if stop.x >= config.width || stop.y >= config.height { return; }

    // Write the seed coordinate at that pixel.
    // For init pass, the value is the seed’s own coordinate.
    // seedTex is rg32sint, but textureStore expects a vec4<i32> value.
    textureStore(
        seedTex,
        vec2<i32>(i32(stop_xy.x), i32(stop_xy.y)),
        vec4<i32>(i32(stop_xy.x), i32(stop_xy.y), i32(stop.z), 0)
    );
}
