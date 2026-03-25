struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

struct ShaderConfig {
    width: f32,  // how many pixels wide the image is
    height: f32, // how many pixels high the image is
    bbox_min_lat: f32,
    bbox_min_lon: f32,
    bbox_max_lat: f32,
    bbox_max_lon: f32,
    gpu_grid_cell_size: f32, // size of each cell (in radians)
    begin_time: f32,         // departure time in seconds since midnight
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
};

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

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    // Fullscreen triangle
    var pos = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f(3.0, -1.0),
        vec2f(-1.0, 3.0),
    );

    // UVs (can go outside 0..1; we clamp later)
    var uv = array<vec2f, 3>(
        vec2f(0.0, 1.0),
        vec2f(2.0, 1.0),
        vec2f(0.0, -1.0),
    );

    var out: VsOut;
    out.pos = vec4f(pos[vid], 0.0, 1.0);
    out.uv = uv[vid];
    return out;
}

@group(0) @binding(0) var jfa_tex: texture_2d<u32>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;
@group(0) @binding(2) var<uniform> jfa_config: JFAConfig;
@group(0) @binding(3) var<storage, read> grid_stops: array<GpuStop>;
@group(0) @binding(4) var<storage, read> minmax: MinMax;

/// color (in oklch) of the earliest arrival_times
const COLOR_FAST = vec3f(0.9333, 0.2068, 105.88); // yellow
/// color (in oklch) of the latest arrival_times
const COLOR_SLOW = vec3f(0.44, 0.2068, 355.76); // purple

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    // Clamp UV to avoid sampling outside (because our fullscreen triangle uses UV beyond 0..1)
    let uv = clamp(in.uv, vec2f(0.0), vec2f(1.0));

    // load min and max arrival times from minmax
    let min_t = f32(atomicLoad(&minmax.min_time));
    let max_t = f32(atomicLoad(&minmax.max_time));

    // get the best stop index from the pixel (just x component since texture is r32uint)
    let dims: vec2u = textureDimensions(jfa_tex);
    let xy: vec2u = min(vec2u(uv * vec2f(dims)), dims - vec2u(1u)); // map uv in [0,1] to [0, dims-1]
    var best_stop_index: u32 = textureLoad(jfa_tex, vec2i(xy), 0).x; // offset included

    var arrival_time: u32;

    // if candidate "candidate_stop_index" is zero, that means it's invalid (because pixel value is always stored with a +1 offset when it's valid, so a valid pixel can't be zero)
    if best_stop_index == 0 {
        // set arrival_time for pixel to max_time so that it blends nicely with the end of the gradient
        arrival_time = u32(max_t);
    } else {
        best_stop_index -= 1; // remove offset

        let best_stop = grid_stops[best_stop_index];

        // normalize candidate stop position vec2f([0,1], [0,1])
        let best_stop_u: f32 = (best_stop.lon - config.bbox_min_lon) / (config.bbox_max_lon - config.bbox_min_lon);
        let best_stop_v: f32 = 1.0 - (best_stop.lat - config.bbox_min_lat) / (config.bbox_max_lat - config.bbox_min_lat);
        let best_stop_uv: vec2f = vec2f(best_stop_u, best_stop_v);

        let meters_per_uv = vec2f(jfa_config.meters_per_px_x, jfa_config.meters_per_px_y) * vec2f(jfa_config.jfa_width, jfa_config.jfa_height);

        let dist: f32 = length((uv - best_stop_uv) * meters_per_uv);

        // walk time in seconds between this pixel and candidate pixel
        let walk_s = u32(dist * config.inverse_walk_speed_mps);

        // arrival time if walking from candidate pixel to current pixel
        arrival_time = best_stop.arrival_time + walk_s;
    }

    // convert arrival time into [0,1] based on max_time
    let denom = max(1.0, max_t - min_t);
    let uniform_arrival_time = clamp((f32(arrival_time) - min_t) / denom, 0.0, 1.0);
    // let uniform_stop_index = clamp(f32(best_stop_index) / f32(arrayLength(&grid_stops)), 0.0, 1.0);

    // return the resulting color
    return vec4f(gradient_get_color(uniform_arrival_time));
    // return vec4f(gradient_get_color(uniform_stop_index));
}

/// gets color on gradient based on scale [0,1]
fn gradient_get_color(scale: f32) -> vec4<f32> {
    let oklch = mix(COLOR_FAST, COLOR_SLOW, scale);

    return vec4(oklch_to_rgb(oklch), 1.0);
}

fn oklch_to_rgb(oklch: vec3<f32>) -> vec3<f32> {
    let l: f32 = oklch.x;
    let c: f32 = oklch.y;
    let h_deg: f32 = oklch.z;

    // degrees -> radians
    let h_rad: f32 = h_deg * 0.017453292519943295; // PI / 180

    // OKLCH -> OKLab
    let a: f32 = c * cos(h_rad);
    let b: f32 = c * sin(h_rad);

    return oklab_to_rgb(vec3<f32>(l, a, b));
}

fn oklab_to_rgb(oklab: vec3<f32>) -> vec3<f32> {
    var l = oklab.x + oklab.y * 0.3963377774 + oklab.z * 0.2158037573;
    var m = oklab.x + oklab.y * -0.1055613458 + oklab.z * -0.0638541728;
    var s = oklab.x + oklab.y * -0.0894841775 + oklab.z * -1.2914855480;
    l = l * l * l; m = m * m * m; s = s * s * s;
    var r = l * 4.0767416621 + m * -3.3077115913 + s * 0.2309699292;
    var g = l * -1.2684380046 + m * 2.6097574011 + s * -0.3413193965;
    var b = l * -0.0041960863 + m * -0.7034186147 + s * 1.7076147010;
    r = clamp(r, 0.0, 1.0); g = clamp(g, 0.0, 1.0); b = clamp(b, 0.0, 1.0);
    return vec3(r, g, b);
}
