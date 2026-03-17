// This file contains all of the implementations related to shaders and rendering

use std::cmp::max;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{DEPART_INSTANT, GTFSData, GpuGridCell, JFA_SCALE, MAX_DIM, Position, WALKING_SPEED};

use tracing::{info_span, instrument};
use wgpu::{BufferUsages, util::DeviceExt};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

// TODO: add a scale reference (like google maps has) showing how zoomed in the view is

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuGridCellKey {
    lat: i32,
    lon: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuGridCellVal {
    start: u32,
    count: u32,
}

// TODO: switch width, height, begin_time, and max_time to be u32
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
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

// TODO: switch width, height, and jump_size to be u32
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct JFAConfig {
    jfa_width: f32,  // how many pixels wide the image is
    jfa_height: f32, // how many pixels high the image is
    jump_size: f32,  // jump size for JFA
}

// Holds all the wgpu state needed to render
struct RenderState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    num_stops: u32,

    gpu_grid_stops_buffer: wgpu::Buffer,

    jfa_seed_pipeline: wgpu::ComputePipeline,
    jfa_seed_bind_group: wgpu::BindGroup,
    jfa_seed_bind_group_layout: wgpu::BindGroupLayout,

    jfa_step_pipeline: wgpu::ComputePipeline,
    jfa_step_bind_group_a: wgpu::BindGroup,
    jfa_step_bind_group_b: wgpu::BindGroup,
    jfa_step_bind_group_layout: wgpu::BindGroupLayout,

    jfa_render_pipeline: wgpu::RenderPipeline,
    jfa_render_bind_group_a: wgpu::BindGroup,
    jfa_render_bind_group_b: wgpu::BindGroup,
    jfa_render_bind_group_layout: wgpu::BindGroupLayout,

    shader_config: ShaderConfig,        // CPU-side copy
    shader_config_buffer: wgpu::Buffer, // GPU-side uniform buffer

    jfa_config: JFAConfig,           // CPU-side copy
    jfa_config_buffer: wgpu::Buffer, // GPU-side uniform buffer

    jfa_texture_a: wgpu::Texture,
    jfa_texture_b: wgpu::Texture,
    jfa_texture_a_view: wgpu::TextureView,
    jfa_texture_b_view: wgpu::TextureView,

    jfa_jump_values_buffer: wgpu::Buffer,
    jfa_jump_count: u32,
    // byte offset of `jump_size` field inside ShaderConfig
    shader_config_jump_offset_bytes: u64,

    timestamp_query_set: wgpu::QuerySet,
    timestamp_resolve_buffer: wgpu::Buffer,
    timestamp_readback_buffer: wgpu::Buffer,
}

impl RenderState {
    async fn new(
        window: Arc<Window>,
        gpu_grid_cell_keys: &Vec<GpuGridCellKey>,
        gpu_grid_cell_vals: &Vec<GpuGridCellVal>,
        gpu_grid_stops: &Vec<[f32; 4]>,
        shader_config: ShaderConfig,
        jfa_config: JFAConfig,
    ) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // SAFETY: the surface must not outlive the window it was created from.
        // We keep both alive together in App, so this is safe.
        let surface = instance.create_surface(window).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::CLEAR_TEXTURE | wgpu::Features::TIMESTAMP_QUERY,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::default(),
            })
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Initializing Buffers
        let gpu_grid_cell_keys_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("GPU Grid Cell Keys Buffer"),
                contents: bytemuck::cast_slice(&gpu_grid_cell_keys),
                usage: BufferUsages::STORAGE,
            });

        let gpu_grid_cell_vals_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("GPU Grid Cell Values Buffer"),
                contents: bytemuck::cast_slice(&gpu_grid_cell_vals),
                usage: BufferUsages::STORAGE,
            });

        let gpu_grid_stops_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Grid Stops Buffer"),
            contents: bytemuck::cast_slice(&gpu_grid_stops),
            usage: BufferUsages::STORAGE,
        });

        let shader_config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Config Buffer"),
            contents: bytemuck::cast_slice(&[shader_config]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let jfa_config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Config Buffer"),
            contents: bytemuck::cast_slice(&[jfa_config]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // TODO: this needs to dynamicall change with screen size
        // Build jump sequence: 8192, 4096, ..., 1
        let mut jumps: Vec<f32> = Vec::new();
        let mut j = 1024u32;
        while j >= 1 {
            jumps.push(j as f32);
            j /= 2;
        }

        let jfa_jump_values_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("JFA Jump Values Buffer"),
            contents: bytemuck::cast_slice(&jumps),
            usage: wgpu::BufferUsages::COPY_SRC,
        });

        // TODO: make this less horrible (push constants would be really cool if they exist (they might))
        // Offset of jump_size in JFAConfig (11th f32 field, zero-based index 10)
        let shader_config_jump_offset_bytes = (2 * std::mem::size_of::<f32>()) as u64;

        // We record 2 timestamps per pass: begin/end.
        // Passes: seed + each jfa step + final render.
        let timestamp_pass_count = 1 + (jumps.len() as u32) + 1; // seed + steps + render
        let timestamp_query_count = timestamp_pass_count * 2;

        let timestamp_query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame Timestamp Query Set"),
            ty: wgpu::QueryType::Timestamp,
            count: timestamp_query_count,
        });

        let timestamp_buffer_size =
            (timestamp_query_count as u64) * std::mem::size_of::<u64>() as u64;

        let timestamp_resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Timestamp Resolve Buffer"),
            size: timestamp_buffer_size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let timestamp_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Timestamp Readback Buffer"),
            size: timestamp_buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // TODO: make it obvious what each component of the color at a pixel is (x, y, valid, None) and such
        // TODO: fix duplication of texture format and of texture usage
        // texture buffers
        let jfa_texture_desc = wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: max(1, config.width / JFA_SCALE),
                height: max(1, config.height / JFA_SCALE),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            label: None,
            view_formats: &[],
        };
        // Ping/pong textures:
        let jfa_texture_a = device.create_texture(&jfa_texture_desc);
        let jfa_texture_b = device.create_texture(&jfa_texture_desc);
        let jfa_texture_a_view = jfa_texture_a.create_view(&Default::default());
        let jfa_texture_b_view = jfa_texture_b.create_view(&Default::default());

        // Setting Bind Group Layout (buffer layout)
        //
        // A bind group layout describes the types of resources that a bind group can contain. Think
        // of this like a C-style header declaration, ensuring both the pipeline and bind group agree
        // on the types of resources.
        let jfa_seed_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("JFA Seed Bind Group Layout"),
                entries: &[
                    // grid_stops @binding(0)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // shader_config @binding(1)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // seed_out @binding(2)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::R32Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    // jfa_config @binding(3)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let jfa_step_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("JFA Step Bind Group Layout"),
                entries: &[
                    // prev_texture @binding(0)
                    // (read only)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::ReadOnly,
                            format: wgpu::TextureFormat::R32Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    // next_texture @binding(1)
                    // (write only)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::R32Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    // shader_config @binding(2)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // jfa_config @binding(3)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let jfa_render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("JFA Render Bind Group Layout"),
                entries: &[
                    // jfa texture @binding(0)
                    // (read only)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // shader_config @binding(1)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // jfa_config @binding(2)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // Putting Buffers into Bind Layout
        //
        // The bind group contains the actual resources to bind to the pipeline.
        //
        // Even when the buffers are individually dropped, wgpu will keep the bind group and buffers
        // alive until the bind group itself is dropped.
        let jfa_seed_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Seed Bind Group"),
            layout: &jfa_seed_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: gpu_grid_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        // Bind group for reading from texture_a and writing to texture_b
        let jfa_step_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Step Bind Group A"),
            layout: &jfa_step_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        // Bind group for reading from texture_b and writing to texture_a
        let jfa_step_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Step Bind Group B"),
            layout: &jfa_step_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        let jfa_render_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group A"),
            layout: &jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        let jfa_render_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group B"),
            layout: &jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        // JFA Seed Pipeline
        let jfa_seed_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/seed_scatter.wgsl"));

        let jfa_seed_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("JFA Seed Pipeline Layout"),
                bind_group_layouts: &[&jfa_seed_bind_group_layout],
                immediate_size: 0,
            });

        let jfa_seed_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("JFA Seed Pipeline"),
            layout: Some(&jfa_seed_pipeline_layout),
            module: &jfa_seed_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // JFA Step Pipeline
        // Compute Pipeline
        let jfa_step_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/jfa_step.wgsl"));

        let jfa_step_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("JFA Step Pipeline Layout"),
                bind_group_layouts: &[&jfa_step_bind_group_layout],
                immediate_size: 0,
            });

        let jfa_step_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("JFA Step Pipeline"),
            layout: Some(&jfa_step_pipeline_layout),
            module: &jfa_step_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // JFA Render Pipeline
        let jfa_render_shader =
            device.create_shader_module(wgpu::include_wgsl!("shaders/render.wgsl"));

        let jfa_render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("JFA Render Pipeline Layout"),
                bind_group_layouts: &[&jfa_render_bind_group_layout],
                immediate_size: 0,
            });

        let jfa_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("JFA Render Pipeline"),
            layout: Some(&jfa_render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &jfa_render_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &jfa_render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            surface,
            device,
            queue,
            config,

            num_stops: gpu_grid_stops.len() as u32,

            gpu_grid_stops_buffer,

            jfa_seed_pipeline,
            jfa_seed_bind_group,
            jfa_seed_bind_group_layout,

            jfa_step_pipeline,
            jfa_step_bind_group_a,
            jfa_step_bind_group_b,
            jfa_step_bind_group_layout,

            jfa_render_pipeline,
            jfa_render_bind_group_a,
            jfa_render_bind_group_b,
            jfa_render_bind_group_layout,

            shader_config,
            shader_config_buffer,

            jfa_config,
            jfa_config_buffer,

            jfa_texture_a,
            jfa_texture_b,
            jfa_texture_a_view,
            jfa_texture_b_view,

            jfa_jump_values_buffer,
            jfa_jump_count: jumps.len() as u32,
            shader_config_jump_offset_bytes,

            timestamp_query_set,
            timestamp_resolve_buffer,
            timestamp_readback_buffer,
        }
    }

    /// Update the shader config buffer (used to give real time input to the shader, like window resizes and such)
    fn upload_shader_config(&self) {
        self.queue.write_buffer(
            &self.shader_config_buffer,
            0,
            bytemuck::cast_slice(&[self.shader_config]),
        );
    }

    /// Update the jfa config buffer (used to give real time input to the shader, like window resizes and such)
    fn upload_jfa_config(&self) {
        self.queue.write_buffer(
            &self.jfa_config_buffer,
            0,
            bytemuck::cast_slice(&[self.jfa_config]),
        );
    }

    fn zoom(&mut self, zoom_steps: f32, cursor_px: winit::dpi::PhysicalPosition<f64>) {
        if zoom_steps == 0.0 || self.config.width == 0 || self.config.height == 0 {
            return;
        }

        let w = self.config.width as f32;
        let h = self.config.height as f32;

        let x = cursor_px.x as f32;
        let y = cursor_px.y as f32;

        // clamp to window
        let x = x.clamp(0.0, w);
        let y = y.clamp(0.0, h);

        // current bbox
        let min_lon = self.shader_config.bbox_min_lon;
        let max_lon = self.shader_config.bbox_max_lon;
        let min_lat = self.shader_config.bbox_min_lat;
        let max_lat = self.shader_config.bbox_max_lat;

        let lon_span = max_lon - min_lon;
        let lat_span = max_lat - min_lat;

        // pixel -> normalized
        let u = x / w; // left..right
        let v = y / h; // bottom..top (since map has north as up)

        // world under cursor before zoom
        let world_lon = min_lon + u * lon_span;
        let world_lat = max_lat - v * lat_span; // y-down screen, lat-up world

        // zoom factor
        let zoom_per_step = 1.1_f32;
        let factor = zoom_per_step.powf(zoom_steps);

        // shrink spans when zooming in
        let new_lon_span = lon_span / factor;
        let new_lat_span = lat_span / factor;

        // solve new min/max so cursor anchors same world point
        let new_min_lon = world_lon - u * new_lon_span;
        let new_max_lon = new_min_lon + new_lon_span;

        let new_max_lat = world_lat + v * new_lat_span;
        let new_min_lat = new_max_lat - new_lat_span;

        self.shader_config.bbox_min_lon = new_min_lon;
        self.shader_config.bbox_max_lon = new_max_lon;
        self.shader_config.bbox_min_lat = new_min_lat;
        self.shader_config.bbox_max_lat = new_max_lat;

        self.upload_shader_config();
    }

    fn pan(&mut self, dx_px: f32, dy_px: f32) {
        if self.config.width == 0 || self.config.height == 0 {
            return;
        }

        let w = self.config.width as f32;
        let h = self.config.height as f32;

        let lon_span = self.shader_config.bbox_max_lon - self.shader_config.bbox_min_lon;
        let lat_span = self.shader_config.bbox_max_lat - self.shader_config.bbox_min_lat;

        // pixel drag -> world delta
        let dlon = (dx_px / w) * lon_span;
        let dlat = (dy_px / h) * lat_span;

        // drag right should move map right (content follows cursor):
        // so bbox moves opposite in lon
        self.shader_config.bbox_min_lon -= dlon;
        self.shader_config.bbox_max_lon -= dlon;

        // drag down should move map down on screen:
        // world lat decreases when moving down
        self.shader_config.bbox_min_lat += dlat;
        self.shader_config.bbox_max_lat += dlat;

        self.upload_shader_config();
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            // recalculate bounding box to adjust to new size
            // pin the bbox min corner, and just shrink/grow bounding box to keep constant effective zoom level
            let width_mult: f32 = new_size.width as f32 / self.config.width as f32;
            let height_mult: f32 = new_size.height as f32 / self.config.height as f32;

            self.shader_config.bbox_max_lat = self.shader_config.bbox_min_lat
                + (height_mult
                    * (self.shader_config.bbox_max_lat - self.shader_config.bbox_min_lat) as f32);
            self.shader_config.bbox_max_lon = self.shader_config.bbox_min_lon
                + (width_mult
                    * (self.shader_config.bbox_max_lon - self.shader_config.bbox_min_lon) as f32);

            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);

            // update shader_config
            self.shader_config.width = new_size.width as f32;
            self.shader_config.height = new_size.height as f32;
            self.upload_shader_config();

            // update jfa_config
            self.jfa_config.jfa_width = max(1, new_size.width / JFA_SCALE) as f32;
            self.jfa_config.jfa_height = max(1, new_size.height / JFA_SCALE) as f32;
            self.upload_jfa_config();

            self.recreate_jfa_textures_and_bind_groups();
        }
    }

    // TODO: this functions is bad and ugly. i would like to swap it for a much simpler solution if possible
    /// recreates textures and bind groups for the jump flood algorithm. Needed when resizing, because texture changes size.
    fn recreate_jfa_textures_and_bind_groups(&mut self) {
        let jfa_texture_desc = wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width: self.jfa_config.jfa_width as u32,
                height: self.jfa_config.jfa_height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            label: None,
            view_formats: &[],
        };

        self.jfa_texture_a = self.device.create_texture(&jfa_texture_desc);
        self.jfa_texture_b = self.device.create_texture(&jfa_texture_desc);
        self.jfa_texture_a_view = self.jfa_texture_a.create_view(&Default::default());
        self.jfa_texture_b_view = self.jfa_texture_b.create_view(&Default::default());

        // Seed bind group (writes into texture_a)
        self.jfa_seed_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Seed Bind Group"),
            layout: &self.jfa_seed_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.gpu_grid_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        // Step bind group A -> reads texture_a, writes texture_b
        self.jfa_step_bind_group_a = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Step Bind Group A"),
            layout: &self.jfa_step_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        // Step bind group B -> read texture_b, writes texture_a
        self.jfa_step_bind_group_b = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Step Bind Group B"),
            layout: &self.jfa_step_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        // Render bind groups (sampling)
        self.jfa_render_bind_group_a = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group A"),
            layout: &self.jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        self.jfa_render_bind_group_b = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group B"),
            layout: &self.jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.jfa_config_buffer.as_entire_binding(),
                },
            ],
        });
    }

    #[instrument(skip(self))]
    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let _span = info_span!("frame").entered();

        let mut q = 0u32; // query index for gpu timers

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let output = self.surface.get_current_texture()?;

        {
            // JFA Seed Pass
            {
                encoder.clear_texture(&self.jfa_texture_a, &wgpu::ImageSubresourceRange::default());
                // encoder.clear_texture(&self.jfa_texture_b, &wgpu::ImageSubresourceRange::default());

                let mut jfa_seed_compute_pass =
                    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Seed Compute Pass"),
                        timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                            query_set: &self.timestamp_query_set,
                            beginning_of_pass_write_index: Some(q),
                            end_of_pass_write_index: Some(q + 1),
                        }),
                    });

                q += 2; // increment q for seed enter and close

                jfa_seed_compute_pass.set_pipeline(&self.jfa_seed_pipeline);
                jfa_seed_compute_pass.set_bind_group(0, &self.jfa_seed_bind_group, &[]);

                let wg = 256u32;
                let n = self.num_stops; // store this somewhere
                let dispatch_x = (n + wg - 1) / wg;
                jfa_seed_compute_pass.dispatch_workgroups(dispatch_x, 1, 1);
            }

            // JFA Step Passes
            let final_texture_render_bind_group: &wgpu::BindGroup;
            {
                // flips read/write between texture_a and texture_b
                // false -> read texture_a, write texture_b
                // true -> read texture_b, write texture_a
                let mut flip = false;

                for jump_size_index in 0..self.jfa_jump_count {
                    // Copy one f32 jump value into ShaderConfig.jump_size
                    let src_offset = (jump_size_index as u64) * (std::mem::size_of::<f32>() as u64);
                    encoder.copy_buffer_to_buffer(
                        &self.jfa_jump_values_buffer,
                        src_offset,
                        &self.jfa_config_buffer,
                        self.shader_config_jump_offset_bytes,
                        std::mem::size_of::<f32>() as u64,
                    );

                    let mut jfa_step_compute_pass =
                        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("Seed Compute Pass"),
                            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                                query_set: &self.timestamp_query_set,
                                beginning_of_pass_write_index: Some(q),
                                end_of_pass_write_index: Some(q + 1),
                            }),
                        });

                    q += 2; // increment q for step enter and close

                    jfa_step_compute_pass.set_pipeline(&self.jfa_step_pipeline);
                    if flip {
                        jfa_step_compute_pass.set_bind_group(0, &self.jfa_step_bind_group_b, &[]);
                    } else {
                        jfa_step_compute_pass.set_bind_group(0, &self.jfa_step_bind_group_a, &[]);
                    }

                    let wg_size_x = 16u32;
                    let wg_size_y = 16u32;

                    let dispatch_x = (self.config.width + wg_size_x - 1) / wg_size_x;
                    let dispatch_y = (self.config.height + wg_size_y - 1) / wg_size_y;

                    jfa_step_compute_pass.dispatch_workgroups(dispatch_x, dispatch_y, 1);

                    flip = !flip;
                }

                // note: after loop, if flip is true -> then final texture is in texture_a (and its in texture_b otherwise)
                final_texture_render_bind_group = if flip {
                    &self.jfa_render_bind_group_a
                } else {
                    &self.jfa_render_bind_group_b
                };
            }

            // final_texture_render_bind_group = &self.jfa_render_bind_group_a;

            // Render Pass
            {
                let view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("JFA Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.1,
                                g: 0.2,
                                b: 0.3,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                        query_set: &self.timestamp_query_set,
                        beginning_of_pass_write_index: Some(q),
                        end_of_pass_write_index: Some(q + 1),
                    }),
                    multiview_mask: None,
                });

                q += 2; // increment q for render enter and close

                render_pass.set_pipeline(&self.jfa_render_pipeline);
                render_pass.set_bind_group(0, final_texture_render_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }
        }

        encoder.resolve_query_set(
            &self.timestamp_query_set,
            0..q,
            &self.timestamp_resolve_buffer,
            0,
        );
        encoder.copy_buffer_to_buffer(
            &self.timestamp_resolve_buffer,
            0,
            &self.timestamp_readback_buffer,
            0,
            (q as u64) * std::mem::size_of::<u64>() as u64,
        );

        self.queue.submit(Some(encoder.finish()));
        output.present();

        // Wait for GPU to complete this submission (simple/minimal approach)
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        // Map and read timestamps
        let slice = self.timestamp_readback_buffer.slice(..(q as u64 * 8));
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });

        let data = slice.get_mapped_range();
        let timestamps: &[u64] = bytemuck::cast_slice(&data);

        // Convert ticks -> ns
        let period_ns = self.queue.get_timestamp_period() as f64;

        // Example logging:
        // pair 0 = seed, next pairs = steps, last pair = render
        for i in 0..(q / 2) as usize {
            let t0 = timestamps[2 * i] as f64;
            let t1 = timestamps[2 * i + 1] as f64;
            let dt_ns = (t1 - t0) * period_ns;
            println!("gpu pass[{i}] = {:.3} ms", dt_ns / 1_000_000.0);
        }

        drop(data);
        self.timestamp_readback_buffer.unmap();

        Ok(())
    }
}

// Holds data needed before the window is created, and the render state after
struct App {
    // Pre-init data
    gpu_grid_cell_keys: Vec<GpuGridCellKey>,
    gpu_grid_cell_vals: Vec<GpuGridCellVal>,
    gpu_grid_stops: Vec<[f32; 4]>,
    shader_config: ShaderConfig,
    jfa_config: JFAConfig,

    // Post-init data
    window: Option<Arc<Window>>,
    render_state: Option<RenderState>,
    // input state
    cursor_pos_px: Option<winit::dpi::PhysicalPosition<f64>>,
    dragging: bool,
    last_drag_pos_px: Option<winit::dpi::PhysicalPosition<f64>>,
}

impl App {
    fn new(
        gtfs_data: &GTFSData,
        arrival_times: &HashMap<u32, u32>,
        gpu_grid_cells: Vec<GpuGridCell>,
        gpu_grid_stops: Vec<[f32; 4]>,
        bbox_min_position: Position,
        bbox_max_position: Position,
    ) -> Self {
        // derive image dimensions from the bounding box aspect ratio
        // longitude degrees are physically shorter at higher latitudes, scale by cos(mid_lat)
        let mid_lat = (bbox_min_position.lat + bbox_max_position.lat) / 2.0;
        let physical_width = (bbox_max_position.lon - bbox_min_position.lon) * mid_lat.cos();
        let physical_height = bbox_max_position.lat - bbox_min_position.lat;
        let aspect_ratio = physical_width / physical_height;
        let (pixels_width, pixels_height) = if aspect_ratio >= 1.0 {
            (MAX_DIM, (MAX_DIM as f64 / aspect_ratio) as u32)
        } else {
            ((MAX_DIM as f64 * aspect_ratio) as u32, MAX_DIM)
        };

        // TODO: replace max_time with actual processing stage to calculate it
        let begin_time = DEPART_INSTANT.time;
        let max_time = DEPART_INSTANT.time + 36000; // shitty hack to make it display SOMETHING

        let shader_config = ShaderConfig {
            width: pixels_width as f32,
            height: pixels_height as f32,
            bbox_min_lat: bbox_min_position.lat as f32,
            bbox_min_lon: bbox_min_position.lon as f32,
            bbox_max_lat: bbox_max_position.lat as f32,
            bbox_max_lon: bbox_max_position.lon as f32,
            gpu_grid_cell_size: gtfs_data.grid.cell_size as f32,
            begin_time: begin_time as f32,
            max_time: max_time as f32,
            inverse_walk_speed_mps: 1.0 / ((WALKING_SPEED * 1000.0) / 3600.0) as f32,
        };

        let jfa_config = JFAConfig {
            jfa_width: max(1, pixels_width / 2) as f32,
            jfa_height: max(1, pixels_height / 2) as f32,
            jump_size: 0.0,
        };

        let (gpu_grid_cell_keys, gpu_grid_cell_vals) = build_gpu_hash(&gpu_grid_cells);

        Self {
            gpu_grid_cell_keys,
            gpu_grid_cell_vals,
            gpu_grid_stops,
            shader_config,
            jfa_config,
            window: None,
            render_state: None,
            cursor_pos_px: None,
            dragging: false,
            last_drag_pos_px: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("HeatMapping")
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            self.shader_config.width,
                            self.shader_config.height,
                        )),
                )
                .unwrap(),
        );

        // resumed() is not async, so we block on the async init here
        let render_state = pollster::block_on(RenderState::new(
            window.clone(),
            &self.gpu_grid_cell_keys,
            &self.gpu_grid_cell_vals,
            &self.gpu_grid_stops,
            self.shader_config,
            self.jfa_config,
        ));

        self.window = Some(window);
        self.render_state = Some(render_state);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                // unset window stuff, so that the program knows the window was closed rather than just being confused
                self.render_state = None;
                self.window = None;

                event_loop.exit(); // and also stop the loop
            }
            WindowEvent::Resized(new_size) => {
                if let Some(state) = &mut self.render_state {
                    state.resize(new_size);
                }
            }
            WindowEvent::MouseWheel {
                device_id,
                delta,
                phase,
            } => {
                // TODO: make helper function to calculate scroll steps so that it can be used elsewhere if needed
                // TODO: implement pinch zoom
                if let (Some(state), Some(window), Some(cursor_pos_px)) =
                    (&mut self.render_state, &self.window, self.cursor_pos_px)
                {
                    let scroll_steps: f32 = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(pos) => (pos.y as f32) / -40.0, // TODO: tune magic number
                    };

                    state.zoom(scroll_steps, cursor_pos_px);

                    // request redraw after zooming
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // if dragging, pan by delta from previous cursor position
                if self.dragging {
                    if let (Some(prev), Some(state)) =
                        (self.last_drag_pos_px, &mut self.render_state)
                    {
                        let dx = position.x as f32 - prev.x as f32;
                        let dy = position.y as f32 - prev.y as f32;
                        state.pan(dx, dy);

                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    self.last_drag_pos_px = Some(position);
                }

                self.cursor_pos_px = Some(position);
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    match state {
                        ElementState::Pressed => {
                            self.dragging = true;
                            self.last_drag_pos_px = self.cursor_pos_px;
                        }
                        ElementState::Released => {
                            self.dragging = false;
                            self.last_drag_pos_px = None;
                        }
                    }
                }

                if button == MouseButton::Right && state == ElementState::Pressed {
                    if let (Some(state), Some(pos)) =
                        (self.render_state.as_ref(), self.cursor_pos_px)
                    {
                        let w = state.config.width as f32;
                        let h = state.config.height as f32;
                        if w > 0.0 && h > 0.0 {
                            // clamp cursor to window
                            let x = (pos.x as f32).clamp(0.0, w);
                            let y = (pos.y as f32).clamp(0.0, h);

                            // normalized coords
                            let u = x / w; // 0..1 left->right
                            let v = 1.0 - (y / h); // 0..1 bottom->top

                            // bbox -> world
                            let min_lon = state.shader_config.bbox_min_lon;
                            let max_lon = state.shader_config.bbox_max_lon;
                            let min_lat = state.shader_config.bbox_min_lat;
                            let max_lat = state.shader_config.bbox_max_lat;

                            let lon = min_lon + u * (max_lon - min_lon);
                            let lat = max_lat - v * (max_lat - min_lat);

                            println!("clicked lat/lon: {:.6}, {:.6}", lat, lon);
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(state) = &mut self.render_state {
                    match state.render() {
                        Ok(()) => {}
                        // Reconfigure if the surface is lost
                        Err(wgpu::SurfaceError::Lost) => {
                            if let Some(state) = &mut self.render_state {
                                let size = winit::dpi::PhysicalSize::new(
                                    state.config.width,
                                    state.config.height,
                                );
                                state.resize(size);
                            }
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                        Err(e) => eprintln!("Render error: {e:?}"),
                    }
                }

                // TODO: figure out when to redraw window and how to make it do that just when it needs to (if thats not what it's already doing)
                // Request another frame immediately for continuous rendering
                // if let Some(window) = &self.window {
                //     window.request_redraw();
                // }
            }
            _ => {}
        }
    }
}

// TODO: switch to giving shader some kinda spatial grid rather than having it iterate through all stops for every pixel
// TODO: switch to a multi-stage aproach that first calculates the arrival time to each pixel, then turns that into a heatmap
/// stop_positions: (latitude, longitude)
pub async fn run(
    gtfs_data: &GTFSData,
    arrival_times: &HashMap<u32, u32>,
    gpu_grid_cells: Vec<GpuGridCell>,
    gpu_grid_stops: Vec<[f32; 4]>,
    bbox_min_position: Position,
    bbox_max_position: Position,
) {
    let event_loop = EventLoop::new().unwrap();

    let mut app = App::new(
        gtfs_data,
        arrival_times,
        gpu_grid_cells,
        gpu_grid_stops,
        bbox_min_position,
        bbox_max_position,
    );

    event_loop.run_app(&mut app).unwrap();
}

fn build_gpu_hash(cells: &[GpuGridCell]) -> (Vec<GpuGridCellKey>, Vec<GpuGridCellVal>) {
    // TODO: what's the times 2 for?
    let cap = (cells.len() * 2).next_power_of_two(); // calculate power of 2 size for hash map (to make gpu happy)
    let empty = GpuGridCellKey {
        lat: i32::MIN,
        lon: i32::MIN,
    }; // TODO: huh?

    let mut keys = vec![empty; cap];
    let mut vals = vec![GpuGridCellVal { start: 0, count: 0 }; cap];

    for cell in cells {
        let key = GpuGridCellKey {
            lat: cell.lat_index,
            lon: cell.lon_index,
        };
        let val = GpuGridCellVal {
            start: cell.start,
            count: cell.count,
        };

        let mut idx = (hash2_i32(key.lat, key.lon) as usize) & (cap - 1); // TODO: huh?

        // TODO: huh?
        loop {
            if keys[idx].lat == i32::MIN {
                keys[idx] = key;
                vals[idx] = val;
                break;
            }
            // if duplicate key possible, overwrite/merge here
            idx = (idx + 1) & (cap - 1);
        }
    }

    return (keys, vals);
}

/// hash function for gpu compatibility (used to compute hashes for a hashmap that can be used within shaders)
fn hash2_i32(a: i32, b: i32) -> u32 {
    let mut x = a as u32;
    let mut y = b as u32;

    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;

    y ^= y >> 16;
    y = y.wrapping_mul(0x7feb352d);
    y ^= y >> 15;
    y = y.wrapping_mul(0x846ca68b);
    y ^= y >> 16;

    x ^ y.rotate_left(16)
}
