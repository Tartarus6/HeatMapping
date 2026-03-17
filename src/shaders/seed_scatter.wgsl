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
    jfa_width: f32,  // how many pixels wide the image is
    jfa_height: f32, // how many pixels high the image is
    jump_size: f32,  // jump size for JFA
}

/// [lat, lon, arrival_time, None]
@group(0) @binding(0) var<storage, read> grid_stops: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;
@group(0) @binding(2) var out_texture: texture_storage_2d<r32uint, write>;

fn pack_xy_u16(x: u32, y: u32) -> u32 {
    // low 16 bits = x, high 16 bits = y
    return (x & 0xffffu) | ((y & 0xffffu) << 16u);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
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
    let stop_x = u32(stop_uv.y * (config.width / 2));
    let stop_y = u32((1.0 - stop_uv.x) * (config.height / 2));

    // Note: if dimensions ever exceed 65535, packing breaks
    if stop_x > 65535u || stop_y > 65535u { return; }

    // let packed: vec4<u32> = vec4<u32>(pack_xy_u16(stop_x, stop_y), 0u, 0u, 0u);
    let packed: vec4<u32> = vec4<u32>(pack_xy_u16(stop_x, stop_y), 0u, 0u, 0u);

    for (var dx = -1; dx <= 1; dx++) {
        for (var dy = -1; dy <= 1; dy++) {
            let px: i32 = i32(stop_x) + dx;
            let py: i32 = i32(stop_y) + dy;

            // bounds guard
            if px < 0 || py < 0 || px >= i32(config.width) || py >= i32(config.height) {
                continue;
            }

            textureStore(
                out_texture,
                vec2<i32>(i32(stop_x) + dx, i32(stop_y) + dy),
                packed
            );
        }
    }
}
