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

@group(0) @binding(0) var<storage, read> grid_cell_keys: array<GpuGridCellKey>;
@group(0) @binding(1) var<storage, read> grid_cell_vals: array<GpuGridCellVal>;
@group(0) @binding(2) var<storage, read> grid_stops: array<vec4<f32>>;
@group(0) @binding(3) var<uniform> config: ShaderConfig;
struct VsOut {
    @builtin(position) frag_position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    return VsOut(vec4(positions[index], 0.0, 1.0));
}

@fragment
fn fs_main(vs: VsOut) -> @location(0) vec4<f32> {
    var bounding_box_min = vec2(config.bbox_min_lat, config.bbox_min_lon);
    var bounding_box_max = vec2(config.bbox_max_lat, config.bbox_max_lon);

    // (x, y) pixel position on a 0.0-1.0 scale (based on width and height)
    var uv = (vec2(config.width - vs.frag_position.x, vs.frag_position.y) / vec2(config.width, config.height));

    // (lat, lon) position of the pixel
    var pixel_position = (vec2(uv.y, 1.0 - uv.x) * (bounding_box_max - bounding_box_min)) + bounding_box_min;

    // precomputing cosine of latitude for equirectangular distance
    var cos_phi = cos(pixel_position.x);

    // TODO: is the floor() needed here (i just want to make sure that the i32 casting doesnt sometimes round or ciel or something)
    var pixel_grid_lat_index: i32 = i32(floor(pixel_position.x / config.gpu_grid_cell_size));
    var pixel_grid_lon_index: i32 = i32(floor(pixel_position.y / config.gpu_grid_cell_size));

    var min_time: f32 = config.max_time; // initialize fastest arrival time to the maximum so that it can be overridden later

    var in_stop_dot = false; // flag to say whether pixel is part of the dot marking a stop

    // TODO: increase neighbor range near the poles to account for smaller cells
    // for the 3x3 of cells around the pixel...
    for (var d_lat_index = -1; d_lat_index <= 1; d_lat_index++) {
        for (var d_lon_index = -1; d_lon_index <= 1; d_lon_index++) {
            var cell_val = lookup_cell(pixel_grid_lat_index + d_lat_index, pixel_grid_lon_index + d_lon_index); // get the cell info (location of stops in stops array)

            // for each stop within current cell
            for (var stop_index = cell_val.start; stop_index < cell_val.start + cell_val.count; stop_index++) {
                var current_time: f32 = get_walk_time(grid_stops[stop_index].xy, pixel_position, cos_phi) + grid_stops[stop_index].z;
                if equirectangular_distance(grid_stops[stop_index].xy, pixel_position, cos_phi) < 100 {
                    in_stop_dot = true;
                }
                if current_time < min_time {
                    min_time = current_time;
                }
            }
        }
    }

    let t = clamp((min_time - config.begin_time) / (config.max_time - config.begin_time), 0.0, 1.0);// the x value is the begin tine and the y is the max time

    let color = travel_time_to_color(t);

    if in_stop_dot {
        return invert_color(color);
    }

    return color;
}

fn lookup_cell(lat_i: i32, lon_i: i32) -> GpuGridCellVal {
    let cap: u32 = arrayLength(&grid_cell_keys);
    let mask: u32 = cap - 1u;
    var idx: u32 = hash2_i32(lat_i, lon_i) & mask;

    for (var probe = 0u; probe < cap; probe++) {
        let k = grid_cell_keys[idx];

        // empty => not found
        if k.lat == i32(0x80000000u) && k.lon == i32(0x80000000u) {
            return GpuGridCellVal(0u, 0u); // count=0 means missing
        }

        if k.lat == lat_i && k.lon == lon_i {
            let v = grid_cell_vals[idx];
            return GpuGridCellVal(v.start, v.count);
        }

        idx = (idx + 1u) & mask;
    }

    return GpuGridCellVal(0u, 0u);
}

/// gets time in seconds to walk between 2 positions (based on distance)
fn get_walk_time(from_position: vec2<f32>, to_position: vec2<f32>, cos_phi: f32) -> f32 {
    return equirectangular_distance(from_position, to_position, cos_phi) * config.inverse_walk_speed_mps;
}

const EARTH_RADIUS_METER: f32 = 6371000.0;
/// fast approximation of distance in meters between 2 positions
fn equirectangular_distance(position_a: vec2<f32>, position_b: vec2<f32>, cos_phi: f32) -> f32 {
    let delta_phi: f32 = position_b.x - position_a.x;
    let delta_lambda: f32 = position_b.y - position_a.y;

    return EARTH_RADIUS_METER * sqrt((delta_lambda * cos_phi) * (delta_lambda * cos_phi) + (delta_phi * delta_phi));
}

fn travel_time_to_color(time: f32) -> vec4<f32> {
    let scale = (time);
    let red = vec3(0.55, 0.2, 0.15);  //fastest
    let orange = vec3(0.65, 0.1, 0.5);
    let yellow = vec3(0.85, 0.00, 0.18);
    let green = vec3(0.72, -0.18, 0.08);
    let blue = vec3(0.55, -0.05, -0.2);
    let purple = vec3(0.4, 0.15, -0.2);   //slowest

    var oklab: vec3<f32>;
    if scale < 0.20 {
        oklab = mix(red, orange, scale / 0.20);
    } else if scale < 0.40 {
        oklab = mix(orange, yellow, (scale - 0.20) / 0.20);
    } else if scale < 0.60 {
        oklab = mix(yellow, green, (scale - 0.40) / 0.20);
    } else if scale < 0.80 {
        oklab = mix(green, blue, (scale - 0.60) / 0.20);
    } else {
        oklab = mix(blue, purple, (scale - 0.80) / 0.20);
    }

    return vec4(oklab_to_rgb(oklab), 1.0);
}

fn oklab_to_rgb(oklab: vec3<f32>) -> vec3<f32> {
    var l = oklab.x + oklab.y * 0.3963377774 + oklab.z * 0.2158037573;
    var m = oklab.x + oklab.y * -0.1055613458 + oklab.z * -0.0638541728;
    var s = oklab.x + oklab.y * -0.0894841775 + oklab.z * -1.2914855480;
    l = l * l * l; m = m * m * m; s = s * s * s;
    var r = l * 4.0767416621 + m * -3.3077115913 + s * 0.2309699292;
    var g = l * -1.2684380046 + m * 2.6097574011 + s * -0.3413193965;
    var b = l * -0.0041960863 + m * -0.7034186147 + s * 1.7076147010;
    r = linear_to_gamma(r); g = linear_to_gamma(g); b = linear_to_gamma(b);
    r = clamp(r, 0.0, 1.0); g = clamp(g, 0.0, 1.0); b = clamp(b, 0, 1.0);
    return vec3(r, g, b);
}

fn linear_to_gamma(c: f32) -> f32 {
    if c >= 0.0031308 {
        return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
    } else {
        return 12.92 * c;
    }
}

fn invert_color(color: vec4<f32>) -> vec4<f32> {
    return vec4(1.0 - color.rgb, color.a);
}

fn hash2_i32(a: i32, b: i32) -> u32 {
    var x: u32 = u32(a);
    var y: u32 = u32(b);

    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);

    y = y ^ (y >> 16u);
    y = y * 0x7feb352du;
    y = y ^ (y >> 15u);
    y = y * 0x846ca68bu;
    y = y ^ (y >> 16u);

    return x ^ ((y << 16u) | (y >> 16u));
}
