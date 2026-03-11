const positions: array<vec2<f32>, 3> = array(
    vec2(-1.0, -1.0),
    vec2(3.0, -1.0),
    vec2(-1.0, 3.0),
); // oversized triangle to cover full viewport after clipping

struct GpuGridCell {
    lat_index: i32,
    lon_index: i32,
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
}

// @group(0) @binding(0) var<storage, read> stops: array<vec3<f32>>;
@group(0) @binding(0) var<storage, read> grid_cells: array<GpuGridCell>;
@group(0) @binding(1) var<storage, read> grid_stops: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> config: ShaderConfig;
// @group(0) @binding(3) var<uniform> bounding_box: vec4<f32>;
// @group(0) @binding(4) var<uniform> start_max_times: vec2<f32>;
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

    var uv = (vs.frag_position.xy / vec2(config.width, config.height));

    var pixel_position = (vec2(uv.x, 1.0 - uv.y) * (bounding_box_max - bounding_box_min)) + bounding_box_min;

    // TODO: is the floor() needed here (i just want to make sure that the i32 casting doesnt sometimes round or ciel or something)
    var pixel_grid_lat_index: i32 = i32(floor(pixel_position.x / config.gpu_grid_cell_size));
    var pixel_grid_lon_index: i32 = i32(floor(pixel_position.y / config.gpu_grid_cell_size));

    var min_time: f32 = config.max_time; // initialize fastest arrival time to the maximum

    for (var i = 0u; i < arrayLength(&grid_cells); i++) { // for each cell
        // if cell is neighboring (or same index)
       // if ((1 <= grid_cells[i].lat_index - pixel_grid_lat_index && grid_cells[i].lat_index - pixel_grid_lat_index <= 1) && (1 <= grid_cells[i].lon_index - pixel_grid_lon_index && grid_cells[i].lon_index - pixel_grid_lon_index <= 1)) {
            for (var j = grid_cells[i].start; j < grid_cells[i].start + grid_cells[i].count; j++) {
                var current_time: f32 = get_walk_time(grid_stops[j].xy, pixel_position) + grid_stops[j].z;
                if (current_time < min_time) {
                    min_time = current_time ;
                }
                //  }
        }
    }

    // for (var i = 0u; i < arrayLength(&grid_stops); i++) { // for each stop
    //     var current_time:f32 = get_walk_time(grid_stops[i].xy,pixel_position) + grid_stops[i].z;
    //     if (current_time < min_time) {
    //         min_time = current_time ;
    //     }
    // }

    let t = clamp((min_time - config.begin_time) / (config.max_time - config.begin_time),0.0,1.0);// the x value is the begin tine and the y is the max time

    return travel_time_to_color(t);
}



/// gets distance in meters between 2 positions
const EARTH_RADIUS_METER: f32 = 6371000.0;
const PI: f32 = 3.14159265359;
fn haversine_distance(position_a: vec2<f32>, position_b: vec2<f32>) -> f32 {
    let rad_a = position_a;
    let rad_b = position_b;

    let phi_1: f32 = rad_a.x;
    let phi_2: f32 = rad_b.x;

    let delta_phi: f32 = rad_b.x - rad_a.x;
    let delta_lambda: f32 = rad_b.y - rad_a.y;

    let a: f32 = sin(delta_phi / 2.0) * sin(delta_phi / 2.0)
        + cos(phi_1) * cos(phi_2) * sin(delta_lambda / 2.0) * sin(delta_lambda / 2.0);
    let c: f32 = 2.0 * asin(sqrt(a));

    return EARTH_RADIUS_METER * c;
}

fn get_walk_time(from_position: vec2<f32>,to_position:vec2<f32>)->f32{
    let speed_mps = (5.0 * 1000.0) / 3600.0;
    return (haversine_distance(from_position, to_position)) / speed_mps;
}

fn travel_time_to_color(time:f32)->vec4<f32>{
    let scale = (time);
    let red    = vec3(0.55,  0.2,   0.15);  //fastest
    let orange = vec3(0.65,  0.1,  0.5);
    let yellow = vec3(0.85,  0.00,  0.18);
    let green  = vec3(0.72, -0.18,  0.08);
    let blue   = vec3(0.55, -0.05, -0.2);
    let purple = vec3(0.4,   0.15, -0.2);   //slowest

    var oklab: vec3<f32>;
    if scale < 0.20 {
        oklab = mix(red, orange, scale / 0.20);
    } else if scale < 0.40 {
        oklab = mix(orange, yellow, (scale - 0.20) / 0.20);
    } else if scale < 0.60 {
        oklab = mix(yellow, green, (scale - 0.40) / 0.20);
    } else if scale < 0.80{
        oklab = mix(green, blue, (scale - 0.60) / 0.20);
    } else {
        oklab = mix(blue, purple, (scale - 0.80) / 0.20);
    }

    return vec4(oklab_to_rgb(oklab), 1.0);
}



fn oklab_to_rgb(oklab:vec3<f32>)->vec3<f32>{
    var l = oklab.x + oklab.y * 0.3963377774 + oklab.z * 0.2158037573;
    var m = oklab.x + oklab.y * -0.1055613458 + oklab.z * -0.0638541728;
    var s = oklab.x + oklab.y * -0.0894841775 + oklab.z * -1.2914855480;
    l = l * l * l; m = m * m * m; s = s * s * s;
    var r = l * 4.0767416621 + m * -3.3077115913 + s * 0.2309699292;
    var g = l * -1.2684380046 + m * 2.6097574011 + s * -0.3413193965;
    var b = l * -0.0041960863 + m * -0.7034186147 + s * 1.7076147010;
    r =  linear_to_gamma(r); g = linear_to_gamma(g); b =  linear_to_gamma(b);
    r = clamp(r,0.0,1.0); g = clamp(g, 0.0,1.0); b = clamp(b, 0, 1.0);
    return vec3(r,g,b);
}


fn linear_to_gamma(c:f32)-> f32{
    if (c >= 0.0031308){
        return 1.055 * pow(c,1.0/2.4) - 0.055;
    }else{
        return 12.92 * c;
    }
}
