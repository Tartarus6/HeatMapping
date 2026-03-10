const positions: array<vec2<f32>, 3> = array(
    vec2(-1.0, -1.0),
    vec2(3.0, -1.0),
    vec2(-1.0, 3.0),
); // oversized triangle to cover full viewport after clipping

@group(0) @binding(0) var<storage, read> stops: array<vec2<f32>>;
@group(0) @binding(1) var<uniform> dimensions: vec2<f32>;
@group(0) @binding(2) var<uniform> bounding_box: vec4<f32>;

struct VsOut {
    @builtin(position) frag_position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    return VsOut(vec4(positions[index], 0.0, 1.0));
}

@fragment
fn fs_main(vs: VsOut) -> @location(0) vec4<f32> {
    var bounding_box_min = bounding_box.xy; // first 2 elements of bounding_box are min_lat and min_lon
    var bounding_box_max = bounding_box.zw; // last  2 elements of bounding_box are max_lat and max_lon

    var uv = (vs.frag_position.xy / dimensions);

    var position = (vec2(uv.x, 1.0 - uv.y) * (bounding_box_max - bounding_box_min)) + bounding_box_min;

    for (var i = 0u; i < arrayLength(&stops); i++) { // for each stop
        if haversine_distance(position, stops[i]) < 100.0 { // if pixel is close to stop
            return vec4(1.0, 0.4, 0.1, 1.0); // orange
        }
    }

    return vec4(0.08, 0.08, 0.12, 1.0); // dark background
}

/// gets distance in meters between 2 positions
const EARTH_RADIUS_METER: f32 = 6371000.0;
const PI: f32 = 3.14159265359;
const DEG_TO_RAD_MULT = PI / 180.0;
fn haversine_distance(position_a: vec2<f32>, position_b: vec2<f32>) -> f32 {
    let rad_a = position_a * DEG_TO_RAD_MULT;
    let rad_b = position_b * DEG_TO_RAD_MULT;

    let phi_1: f32 = rad_a.x;
    let phi_2: f32 = rad_b.x;

    let delta_phi: f32 = rad_b.x - rad_a.x;
    let delta_lambda: f32 = rad_b.y - rad_a.y;

    let a: f32 = sin(delta_phi / 2.0) * sin(delta_phi / 2.0)
        + cos(phi_1) * cos(phi_2) * sin(delta_lambda / 2.0) * sin(delta_lambda / 2.0);
    let c: f32 = 2.0 * asin(sqrt(a));

    return EARTH_RADIUS_METER * c;
}
