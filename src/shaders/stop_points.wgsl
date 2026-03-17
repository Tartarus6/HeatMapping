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
    jfa_width: f32,
    jfa_height: f32,
    jump_size: f32,
    meters_per_px_x: f32,
    meters_per_px_y: f32,
}

// [lat, lon, arrival_time, _]
@group(0) @binding(0) var<storage, read> grid_stops: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;
@group(0) @binding(2) var<uniform> jfa_config: JFAConfig;

/// radius of the circles drawn around the stops
const RADIUS_M: f32 = 100.0;
/// color of the circled drawn around the stops
const COLOR: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 0.90);

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>, // in [-1,1]x[-1,1], for circle mask
};

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    @builtin(instance_index) iid: u32
) -> VsOut {
    // 2 triangles (6 verts) for a quad around each stop
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );

    let stop = grid_stops[iid];
    let lat = stop.x;
    let lon = stop.y;

    // cull stop if not within bounding box
    if lat < config.bbox_min_lat || lat > config.bbox_max_lat ||
        lon < config.bbox_min_lon || lon > config.bbox_max_lon {
        var o: VsOut;
        o.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0); // outside clip
        o.local = vec2<f32>(0.0, 0.0);
        return o;
    }

    // world -> uv
    let u = (lon - config.bbox_min_lon) / (config.bbox_max_lon - config.bbox_min_lon);
    let v = (config.bbox_max_lat - lat) / (config.bbox_max_lat - config.bbox_min_lat);

    // uv -> pixel
    let x_px = u * config.width;
    let y_px = v * config.height;

    // meters -> pixels (approx)
    let mpp = min(jfa_config.meters_per_px_x, jfa_config.meters_per_px_y);
    let radius_px = max(1.0, RADIUS_M / mpp);

    let local = corners[vid];
    let px = vec2<f32>(x_px, y_px) + local * radius_px;

    // pixel -> NDC
    let ndc_x = (px.x / config.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (px.y / config.height) * 2.0;

    var out: VsOut;
    out.pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.local = local;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // circle mask in quad-local coordinates
    let r2 = dot(in.local, in.local);
    if r2 > 1.0 {
        discard;
    }

    // TODO: somehow have points be inverse of color underneath them (reversing heatmap) to improve visibility
    return COLOR;
}
