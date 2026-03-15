struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

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

@group(0) @binding(0) var jfa_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> config: ShaderConfig;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    // Clamp UV to avoid sampling outside (because our fullscreen triangle uses UV beyond 0..1)
    let uv = clamp(in.uv, vec2f(0.0), vec2f(1.0));

    // convert UV into integer texel coordinates
    let dims: vec2u = textureDimensions(jfa_tex);
    // map uv in [0,1] to [0, dims-1]
    let xy: vec2u = min(vec2u(uv * vec2f(dims)), dims - vec2u(1u));

    let c: vec4f = textureLoad(jfa_tex, vec2i(xy), 0);

    // return vec4f(c.rg / vec2(config.width, config.height), c.b, 1.0);
    return vec4f(c.b, c.b, c.b, 1.0);
}
