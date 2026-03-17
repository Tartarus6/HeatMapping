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
    // TODO: fix max time
    max_time: f32,               // latest arrival time in seconds since midnight
    inverse_walk_speed_mps: f32, // walking speed in seconds per meter
}

struct JFAConfig {
    jfa_width: f32,       // how many pixels wide the image is
    jfa_height: f32,      // how many pixels high the image is
    jump_size: f32,       // jump size for JFA
    meters_per_px_x: f32, // approximate number of meters per x pixel
    meters_per_px_y: f32, // approximate number of meters per y pixel
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    // Clamp UV to avoid sampling outside (because our fullscreen triangle uses UV beyond 0..1)
    let uv = clamp(in.uv, vec2f(0.0), vec2f(1.0));

    // convert UV into integer texel coordinates
    let dims: vec2u = textureDimensions(jfa_tex);
    // map uv in [0,1] to [0, dims-1]
    let xy: vec2u = min(vec2u(uv * vec2f(dims)), dims - vec2u(1u));

    // get the arrival time of pixel (just x component since texture is r32uint)
    let arrival_time: u32 = textureLoad(jfa_tex, vec2i(xy), 0).x;

    // convert arrival time into [0,1] based on max_time
    let uniform_arrival_time: f32 = (f32(arrival_time) - config.begin_time) / (config.max_time - config.begin_time);

    // return the resulting color
    return vec4f(gradient_get_color(uniform_arrival_time));
}

/// gets color on gradient based on scale [0,1]
fn gradient_get_color(scale: f32) -> vec4<f32> {
    let red = vec3(0.55, 0.2, 0.15);  //fastest
    let orange = vec3(0.65, 0.1, 0.5);
    let yellow = vec3(0.85, 0.00, 0.18);
    let green = vec3(0.72, -0.18, 0.08);
    let blue = vec3(0.55, -0.05, -0.2);
    let purple = vec3(0.4, 0.15, -0.2);   //slowest

    var oklab: vec3<f32>;
    if scale < 0.20 {
        oklab = mix(red, orange, scale * 5);
    } else if scale < 0.40 {
        oklab = mix(orange, yellow, (scale - 0.20) * 5);
    } else if scale < 0.60 {
        oklab = mix(yellow, green, (scale - 0.40) * 5);
    } else if scale < 0.80 {
        oklab = mix(green, blue, (scale - 0.60) * 5);
    } else {
        oklab = mix(blue, purple, (scale - 0.80) * 5);
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
        return 1.055 * pow(c, 0.41666666666) - 0.055;
    } else {
        return 12.92 * c;
    }
}
