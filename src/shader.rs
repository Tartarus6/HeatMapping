use std::collections::HashMap;
use std::sync::Arc;

use crate::{DEPART_INSTANT, GTFSData, GpuGridCell, MAX_DIM, Position, str_time_to_seconds};

use wgpu::{BufferUsages, util::DeviceExt};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

// TODO: switch width, height, begin_time, and max_time to be u32
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
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

// Holds all the wgpu state needed to render
struct RenderState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl RenderState {
    async fn new(
        window: Arc<Window>,
        gpu_grid_cells: &Vec<GpuGridCell>,
        gpu_grid_stops: &Vec<[f32; 4]>,
        shader_config: ShaderConfig,
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
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();

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

        let gpu_grid_cells_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Grid Cells Buffer"),
            contents: bytemuck::cast_slice(&gpu_grid_cells),
            usage: BufferUsages::STORAGE,
        });

        let gpu_grid_stops_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GPU Grid Stops Buffer"),
            contents: bytemuck::cast_slice(&gpu_grid_stops),
            usage: BufferUsages::STORAGE,
        });

        let config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Config Buffer"),
            contents: bytemuck::cast_slice(&[shader_config]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // A bind group layout describes the types of resources that a bind group can contain. Think
        // of this like a C-style header declaration, ensuring both the pipeline and bind group agree
        // on the types of resources.
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // The bind group contains the actual resources to bind to the pipeline.
        //
        // Even when the buffers are individually dropped, wgpu will keep the bind group and buffers
        // alive until the bind group itself is dropped.
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: gpu_grid_cells_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: gpu_grid_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: config_buffer.as_entire_binding(),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/shader.wgsl"));

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                immediate_size: 0,
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: None,
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: None,
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState {
                        alpha: wgpu::BlendComponent::REPLACE,
                        color: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                // Setting this to anything other than Fill requires Features::NON_FILL_POLYGON_MODE
                polygon_mode: wgpu::PolygonMode::Fill,
                // Requires Features::DEPTH_CLIP_CONTROL
                unclipped_depth: false,
                // Requires Features::CONSERVATIVE_RASTERIZATION
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            // If the pipeline will be used with a multiview render pass, this
            // tells wgpu to render to just specific texture layers.
            multiview_mask: None,
            cache: None,
        });

        Self {
            surface,
            device,
            queue,
            config,
            render_pipeline,
            bind_group,
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);

            println!("resized!");
        }
    }

    fn render(&self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
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
                timestamp_writes: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();

        Ok(())
    }
}

// Holds data needed before the window is created, and the render state after
struct App {
    // Pre-init data
    gpu_grid_cells: Vec<GpuGridCell>,
    gpu_grid_stops: Vec<[f32; 4]>,
    shader_config: ShaderConfig,
    // Post-init state
    window: Option<Arc<Window>>,
    render_state: Option<RenderState>,
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
            &self.gpu_grid_cells,
            &self.gpu_grid_stops,
            self.shader_config,
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
                // TODO: this doesn't seem to work
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if let Some(state) = &mut self.render_state {
                    state.resize(new_size);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(state) = &self.render_state {
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

    println!("pixels_width : {}", pixels_width);
    println!("pixels_height: {}", pixels_height);
    println!("aspect: {}", aspect_ratio);

    let mut stop_positions: Vec<[f32; 3]> = Vec::new();
    for stop in gtfs_data.stops.values() {
        if let Some(&arrival_time) = arrival_times.get(&stop.stop_id) {
            stop_positions.push([
                stop.position.lat as f32,
                stop.position.lon as f32,
                arrival_time as f32,
            ]);
        }
    }

    // TODO: replace max_time with actual processing stage to calculate it
    let begin_time = DEPART_INSTANT.time;
    let max_time = DEPART_INSTANT.time + str_time_to_seconds("05:00:00").unwrap(); // shitty hack to make it display SOMETHING

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
    };

    let event_loop = EventLoop::new().unwrap();
    let mut app = App {
        gpu_grid_cells,
        gpu_grid_stops,
        shader_config,
        window: None,
        render_state: None,
    };

    event_loop.run_app(&mut app).unwrap();
}
