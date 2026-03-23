use std::{cmp::max, sync::Arc};
use tracing::{info_span, instrument};
use wgpu::{BufferUsages, Device, util::DeviceExt};
use winit::window::Window;

use crate::structs::{GpuGridCellKey, GpuGridCellVal, GpuStop, JFAConfig, ShaderConfig};

const JFA_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

// TODO: add a scale reference (like google maps has) showing how zoomed in the view is

pub struct Buffers {
    pub gpu_stops_buffer: wgpu::Buffer,
    pub minmax_buffer: wgpu::Buffer,

    pub shader_config_buffer: wgpu::Buffer,
    pub jfa_config_buffer: wgpu::Buffer,
    pub jfa_jump_values_buffer: wgpu::Buffer,

    pub timestamp_resolve_buffer: wgpu::Buffer,
    pub timestamp_readback_buffer: wgpu::Buffer,
    pub timestamp_query_set: wgpu::QuerySet,

    pub jfa_jump_count: u32,
    // TODO: stupid bad byte offset. remove and replace with something less fragile
    // byte offset of `jump_size` field inside ShaderConfig
    pub shader_config_jump_offset_bytes: u64,
}

pub struct ShaderResources {
    pub jfa_seed_pipeline: wgpu::ComputePipeline,
    pub jfa_seed_bind_group: wgpu::BindGroup,
    pub jfa_seed_bind_group_layout: wgpu::BindGroupLayout,

    pub jfa_step_pipeline: wgpu::ComputePipeline,
    pub jfa_step_bind_group_a: wgpu::BindGroup,
    pub jfa_step_bind_group_b: wgpu::BindGroup,
    pub jfa_step_bind_group_layout: wgpu::BindGroupLayout,

    pub minmax_pipeline: wgpu::ComputePipeline,
    pub minmax_bind_group_layout: wgpu::BindGroupLayout,
    pub minmax_bind_group_a: wgpu::BindGroup,
    pub minmax_bind_group_b: wgpu::BindGroup,

    pub jfa_render_pipeline: wgpu::RenderPipeline,
    pub jfa_render_bind_group_a: wgpu::BindGroup,
    pub jfa_render_bind_group_b: wgpu::BindGroup,
    pub jfa_render_bind_group_layout: wgpu::BindGroupLayout,

    pub stop_points_pipeline: wgpu::RenderPipeline,
    pub stop_points_bind_group: wgpu::BindGroup,

    pub jfa_texture_a: wgpu::Texture,
    pub jfa_texture_b: wgpu::Texture,
    pub jfa_texture_a_view: wgpu::TextureView,
    pub jfa_texture_b_view: wgpu::TextureView,
}

// Holds all the wgpu state needed to render
pub struct RenderState {
    pub buffers: Buffers,
    pub shader_resources: ShaderResources,

    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,

    pub num_stops: u32,

    pub shader_config: ShaderConfig, // CPU-side copy
    pub jfa_config: JFAConfig,       // CPU-side copy
}

impl RenderState {
    #[instrument(skip(self))]
    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let _span = info_span!("frame").entered();

        let mut q = 0u32; // query index for gpu timers

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let output = self.surface.get_current_texture()?;

        {
            // JFA Seed Pass
            {
                encoder.clear_texture(
                    &self.shader_resources.jfa_texture_a,
                    &wgpu::ImageSubresourceRange::default(),
                );
                // encoder.clear_texture(&self.shader_resources.jfa_texture_b, &wgpu::ImageSubresourceRange::default());

                let mut jfa_seed_compute_pass =
                    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Seed Compute Pass"),
                        timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                            query_set: &self.buffers.timestamp_query_set,
                            beginning_of_pass_write_index: Some(q),
                            end_of_pass_write_index: Some(q + 1),
                        }),
                    });

                q += 2; // increment q for seed enter and close

                jfa_seed_compute_pass.set_pipeline(&self.shader_resources.jfa_seed_pipeline);
                jfa_seed_compute_pass.set_bind_group(
                    0,
                    &self.shader_resources.jfa_seed_bind_group,
                    &[],
                );

                let wg = 256u32;
                let n = self.num_stops; // store this somewhere
                let dispatch_x = (n + wg - 1) / wg;
                jfa_seed_compute_pass.dispatch_workgroups(dispatch_x, 1, 1);
            }

            // JFA Step Passes
            // flips read/write between texture_a and texture_b
            // false -> read texture_a, write texture_b
            // true -> read texture_b, write texture_a
            let mut flip = false;
            {
                for jump_size_index in 0..self.buffers.jfa_jump_count {
                    // Copy one f32 jump value into ShaderConfig.jump_size
                    let src_offset = (jump_size_index as u64) * (std::mem::size_of::<f32>() as u64);
                    encoder.copy_buffer_to_buffer(
                        &self.buffers.jfa_jump_values_buffer,
                        src_offset,
                        &self.buffers.jfa_config_buffer,
                        self.buffers.shader_config_jump_offset_bytes,
                        std::mem::size_of::<f32>() as u64,
                    );

                    let mut jfa_step_compute_pass =
                        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("Seed Compute Pass"),
                            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                                query_set: &self.buffers.timestamp_query_set,
                                beginning_of_pass_write_index: Some(q),
                                end_of_pass_write_index: Some(q + 1),
                            }),
                        });

                    q += 2; // increment q for step enter and close

                    jfa_step_compute_pass.set_pipeline(&self.shader_resources.jfa_step_pipeline);
                    if flip {
                        jfa_step_compute_pass.set_bind_group(
                            0,
                            &self.shader_resources.jfa_step_bind_group_b,
                            &[],
                        );
                    } else {
                        jfa_step_compute_pass.set_bind_group(
                            0,
                            &self.shader_resources.jfa_step_bind_group_a,
                            &[],
                        );
                    }

                    let wg_size_x = 16u32;
                    let wg_size_y = 16u32;

                    let dispatch_x = (self.jfa_config.jfa_width as u32 + wg_size_x - 1) / wg_size_x;
                    let dispatch_y =
                        (self.jfa_config.jfa_height as u32 + wg_size_y - 1) / wg_size_y;

                    jfa_step_compute_pass.dispatch_workgroups(dispatch_x, dispatch_y, 1);

                    flip = !flip;
                }
            }

            // Minmax Pass (finding lowest and greatest pixel arrival_time)
            {
                // reset minmax buffer each frame
                let minmax_init: [u32; 2] = [u32::MAX, 0];
                self.queue.write_buffer(
                    &self.buffers.minmax_buffer,
                    0,
                    bytemuck::cast_slice(&minmax_init),
                );

                let minmax_bind_group = if flip {
                    &self.shader_resources.minmax_bind_group_a
                } else {
                    &self.shader_resources.minmax_bind_group_b
                };

                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Arrival MinMax Pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.shader_resources.minmax_pipeline);
                    pass.set_bind_group(0, minmax_bind_group, &[]);

                    let wg_size_x = 16u32;
                    let wg_size_y = 16u32;
                    let dispatch_x = (self.jfa_config.jfa_width as u32 + wg_size_x - 1) / wg_size_x;
                    let dispatch_y =
                        (self.jfa_config.jfa_height as u32 + wg_size_y - 1) / wg_size_y;
                    pass.dispatch_workgroups(dispatch_x, dispatch_y, 1);
                }
            }

            // Render Pass (turning best stop index texture into an actual image with colors)
            {
                let view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                // note: after loop, if flip is true -> then final texture is in texture_a (and its in texture_b otherwise)
                let final_texture_render_bind_group = if flip {
                    &self.shader_resources.jfa_render_bind_group_a
                } else {
                    &self.shader_resources.jfa_render_bind_group_b
                };

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
                        query_set: &self.buffers.timestamp_query_set,
                        beginning_of_pass_write_index: Some(q),
                        end_of_pass_write_index: Some(q + 1),
                    }),
                    multiview_mask: None,
                });

                q += 2; // increment q for render enter and close

                render_pass.set_pipeline(&self.shader_resources.jfa_render_pipeline);
                render_pass.set_bind_group(0, final_texture_render_bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            // Stop points overlay pass (draw points on top of heatmap)
            {
                let view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                let mut overlay_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Stop Points Overlay Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load, // keep heatmap already drawn
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                        query_set: &self.buffers.timestamp_query_set,
                        beginning_of_pass_write_index: Some(q),
                        end_of_pass_write_index: Some(q + 1),
                    }), // or add timestamps if you want
                    multiview_mask: None,
                });

                q += 2; // increment q for stop points enter and close

                overlay_pass.set_pipeline(&self.shader_resources.stop_points_pipeline);
                overlay_pass.set_bind_group(0, &self.shader_resources.stop_points_bind_group, &[]);
                overlay_pass.draw(0..6, 0..self.num_stops); // 6 verts per stop instance
            }
        }

        encoder.resolve_query_set(
            &self.buffers.timestamp_query_set,
            0..q,
            &self.buffers.timestamp_resolve_buffer,
            0,
        );
        encoder.copy_buffer_to_buffer(
            &self.buffers.timestamp_resolve_buffer,
            0,
            &self.buffers.timestamp_readback_buffer,
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
        let slice = self
            .buffers
            .timestamp_readback_buffer
            .slice(..(q as u64 * 8));
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
        self.buffers.timestamp_readback_buffer.unmap();

        Ok(())
    }

    pub async fn new(
        window: Arc<Window>,
        gpu_grid_cell_keys: &Vec<GpuGridCellKey>,
        gpu_grid_cell_vals: &Vec<GpuGridCellVal>,
        gpu_stops: &Vec<GpuStop>,
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

        let buffers = initialize_buffers(
            &device,
            gpu_grid_cell_keys,
            gpu_grid_cell_vals,
            gpu_stops,
            shader_config,
            jfa_config,
        );

        let shader_resources =
            initialize_shader_resources(&device, &buffers, &jfa_config, surface_format);

        Self {
            buffers,
            shader_resources,

            surface,
            device,
            queue,
            config,

            num_stops: gpu_stops.len() as u32,

            shader_config,

            jfa_config,
        }
    }

    /// Update the shader config buffer (used to give real time input to the shader, like window resizes and such)
    pub fn upload_shader_config(&self) {
        self.queue.write_buffer(
            &self.buffers.shader_config_buffer,
            0,
            bytemuck::cast_slice(&[self.shader_config]),
        );
    }

    /// Update the jfa config buffer (used to give real time input to the shader, like window resizes and such)
    pub fn upload_jfa_config(&self) {
        self.queue.write_buffer(
            &self.buffers.jfa_config_buffer,
            0,
            bytemuck::cast_slice(&[self.jfa_config]),
        );
    }

    /// recreates textures and bind groups for the jump flood algorithm. Needed when resizing, because texture changes size.
    pub fn recreate_jfa_textures_and_bind_groups(&mut self) {
        self.shader_resources.recreate_jfa_textures_and_bind_groups(
            &self.device,
            &self.buffers,
            &self.jfa_config,
        );
    }
}

pub fn initialize_buffers(
    device: &Device,
    gpu_grid_cell_keys: &Vec<GpuGridCellKey>,
    gpu_grid_cell_vals: &Vec<GpuGridCellVal>,
    gpu_stops: &Vec<GpuStop>,
    shader_config: ShaderConfig,
    jfa_config: JFAConfig,
) -> Buffers {
    // Initializing Buffers

    // TODO: use gpu_grid_cell_keys_buffer and gpu_grid_cell_vals_buffer in planned dijkstra shader
    let gpu_grid_cell_keys_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("GPU Grid Cell Keys Buffer"),
        contents: bytemuck::cast_slice(&gpu_grid_cell_keys),
        usage: BufferUsages::STORAGE,
    });

    let gpu_grid_cell_vals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("GPU Grid Cell Values Buffer"),
        contents: bytemuck::cast_slice(&gpu_grid_cell_vals),
        usage: BufferUsages::STORAGE,
    });

    let gpu_stops_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("GPU Grid Stops Buffer"),
        contents: bytemuck::cast_slice(&gpu_stops),
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
    // jumps.push(1.0); // add an extra jump of distance 1 in order to improve output stability

    let jfa_jump_values_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("JFA Jump Values Buffer"),
        contents: bytemuck::cast_slice(&jumps),
        usage: wgpu::BufferUsages::COPY_SRC,
    });

    // TODO: make this less horrible (push constants would be really cool if they exist (they might))
    // Offset of jump_size in JFAConfig (11th f32 field, zero-based index 10)
    let jfa_config_jump_offset_bytes = (2 * std::mem::size_of::<f32>()) as u64;

    // We record 2 timestamps per pass: begin/end.
    // Passes: seed + each jfa step + final render.
    let timestamp_pass_count = 1 + (jumps.len() as u32) + 1 + 1; // seed + steps + render + stop_points
    let timestamp_query_count = timestamp_pass_count * 2;

    let timestamp_query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("Frame Timestamp Query Set"),
        ty: wgpu::QueryType::Timestamp,
        count: timestamp_query_count,
    });

    let timestamp_buffer_size = (timestamp_query_count as u64) * std::mem::size_of::<u64>() as u64;

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

    let minmax_init: [u32; 2] = [u32::MAX, 0];
    let minmax_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("MinMax Buffer"),
        contents: bytemuck::cast_slice(&minmax_init),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    return Buffers {
        gpu_stops_buffer,
        minmax_buffer,

        shader_config_buffer,
        jfa_config_buffer,
        jfa_jump_values_buffer,

        timestamp_query_set,
        timestamp_resolve_buffer,
        timestamp_readback_buffer,

        jfa_jump_count: jumps.len() as u32,
        shader_config_jump_offset_bytes: jfa_config_jump_offset_bytes,
    };
}

pub fn initialize_shader_resources(
    device: &Device,
    buffers: &Buffers,
    jfa_config: &JFAConfig,
    surface_format: wgpu::TextureFormat,
) -> ShaderResources {
    // --- JFA Seed ---
    let jfa_seed_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("JFA Seed Bind Group Layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: JFA_TEXTURE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
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

    let jfa_seed_shader =
        device.create_shader_module(wgpu::include_wgsl!("shaders/seed_scatter.wgsl"));

    let jfa_seed_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

    // --- JFA Step ---
    let jfa_step_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("JFA Step Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: JFA_TEXTURE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: JFA_TEXTURE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    let jfa_step_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/jfa_step.wgsl"));

    let jfa_step_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

    // --- Minmax ---
    let minmax_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("MinMax Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    let minmax_shader =
        device.create_shader_module(wgpu::include_wgsl!("shaders/arrival_minmax.wgsl"));

    let minmax_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("MinMax Pipeline Layout"),
        bind_group_layouts: &[&minmax_bind_group_layout],
        immediate_size: 0,
    });

    let minmax_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("MinMax Pipeline"),
        layout: Some(&minmax_pipeline_layout),
        module: &minmax_shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    // --- JFA Render ---
    let jfa_render_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("JFA Render Bind Group Layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    let jfa_render_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/render.wgsl"));

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

    // --- Stop Points ---
    let stop_points_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Stop Points Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    let stop_points_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Stop Point Bind Group"),
        layout: &stop_points_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.gpu_stops_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.shader_config_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.jfa_config_buffer.as_entire_binding(),
            },
        ],
    });

    let stop_points_shader =
        device.create_shader_module(wgpu::include_wgsl!("shaders/stop_points.wgsl"));

    let stop_points_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Stop Points Pipeline Layout"),
            bind_group_layouts: &[&stop_points_bind_group_layout],
            immediate_size: 0,
        });

    let stop_points_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Stop Points Pipeline"),
        layout: Some(&stop_points_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &stop_points_shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &stop_points_shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

    let (jfa_texture_a, jfa_texture_b, jfa_texture_a_view, jfa_texture_b_view) =
        create_jfa_texture_pair(
            device,
            jfa_config.jfa_width as u32,
            jfa_config.jfa_height as u32,
        );

    ShaderResources {
        jfa_seed_pipeline,
        jfa_seed_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Seed Bind Group"),
            layout: &jfa_seed_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
            ],
        }),
        jfa_seed_bind_group_layout,

        jfa_step_pipeline,
        jfa_step_bind_group_a: device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
            ],
        }),
        jfa_step_bind_group_b: device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
            ],
        }),
        jfa_step_bind_group_layout,

        minmax_pipeline,
        minmax_bind_group_a: device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MinMax Bind Group A"),
            layout: &minmax_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        }),
        minmax_bind_group_b: device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MinMax Bind Group B"),
            layout: &minmax_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        }),
        minmax_bind_group_layout,

        jfa_render_pipeline,
        jfa_render_bind_group_a: device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group A"),
            layout: &jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        }),
        jfa_render_bind_group_b: device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group B"),
            layout: &jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        }),
        jfa_render_bind_group_layout,

        stop_points_pipeline,
        stop_points_bind_group,

        jfa_texture_a,
        jfa_texture_b,
        jfa_texture_a_view,
        jfa_texture_b_view,
    }
}

impl ShaderResources {
    pub fn recreate_jfa_textures_and_bind_groups(
        &mut self,
        device: &Device,
        buffers: &Buffers,
        jfa_config: &JFAConfig,
    ) {
        let (jfa_texture_a, jfa_texture_b, jfa_texture_a_view, jfa_texture_b_view) =
            create_jfa_texture_pair(
                device,
                jfa_config.jfa_width as u32,
                jfa_config.jfa_height as u32,
            );

        self.jfa_texture_a = jfa_texture_a;
        self.jfa_texture_b = jfa_texture_b;
        self.jfa_texture_a_view = jfa_texture_a_view;
        self.jfa_texture_b_view = jfa_texture_b_view;

        self.jfa_seed_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Seed Bind Group"),
            layout: &self.jfa_seed_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
            ],
        });

        self.jfa_step_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
            ],
        });

        self.jfa_step_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
            ],
        });

        self.minmax_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MinMax Bind Group A"),
            layout: &self.minmax_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        });

        self.minmax_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MinMax Bind Group B"),
            layout: &self.minmax_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        });

        self.jfa_render_bind_group_a = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group A"),
            layout: &self.jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        });

        self.jfa_render_bind_group_b = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("JFA Render Bind Group B"),
            layout: &self.jfa_render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.jfa_texture_b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.shader_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.jfa_config_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.gpu_stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.minmax_buffer.as_entire_binding(),
                },
            ],
        });
    }
}

fn create_jfa_texture_pair(
    device: &Device,
    width: u32,
    height: u32,
) -> (
    wgpu::Texture,
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::TextureView,
) {
    let jfa_texture_desc = wgpu::TextureDescriptor {
        size: wgpu::Extent3d {
            width: max(1, width),
            height: max(1, height),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: JFA_TEXTURE_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST,
        label: None,
        view_formats: &[],
    };

    let jfa_texture_a = device.create_texture(&jfa_texture_desc);
    let jfa_texture_b = device.create_texture(&jfa_texture_desc);
    let jfa_texture_a_view = jfa_texture_a.create_view(&Default::default());
    let jfa_texture_b_view = jfa_texture_b.create_view(&Default::default());

    (
        jfa_texture_a,
        jfa_texture_b,
        jfa_texture_a_view,
        jfa_texture_b_view,
    )
}
