use wgpu::{BufferUsages, util::DeviceExt};

use crate::{BBOX_MAX, BBOX_MIN};

/// stop_positions: (latitude, longitude)
pub async fn run(
    stop_positions: &Vec<[f32; 2]>,
    pixels_width: u32,
    pixels_height: u32,
    output_path: &str,
) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();

    // TODO: is this right?
    // let stop_position_pixels_bytes: Vec<u8> = stop_position_pixels
    //     .iter()
    //     .flat_map(|(x, y)| [x.to_le_bytes(), y.to_le_bytes()])
    //     .flatten()
    //     .collect::<Vec<u8>>();
    let stops_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Stops Buffer"),
        contents: bytemuck::cast_slice(&stop_positions),
        usage: BufferUsages::STORAGE,
    });

    let dimensions_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Config Buffer"),
        contents: bytemuck::cast_slice(&[pixels_width as f32, pixels_height as f32]),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let bounding_box_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Bounding Box Buffer"),
        contents: bytemuck::cast_slice(&[
            BBOX_MIN.lat as f32,
            BBOX_MIN.lon as f32,
            BBOX_MAX.lat as f32,
            BBOX_MAX.lon as f32,
        ]),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let texture_desc = wgpu::TextureDescriptor {
        size: wgpu::Extent3d {
            width: pixels_width,
            height: pixels_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
        label: None,
        view_formats: &[],
    };
    let texture = device.create_texture(&texture_desc);
    let texture_view = texture.create_view(&Default::default());

    // we need to store this for later
    let u32_size = std::mem::size_of::<u32>() as u32;

    // Calculate bytes per row and align it to 256
    let unpadded_bytes_per_row = pixels_width * u32_size;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padding = (align - unpadded_bytes_per_row % align) % align;
    let padded_bytes_per_row = unpadded_bytes_per_row + padding;

    let output_buffer_size = (padded_bytes_per_row * pixels_height) as wgpu::BufferAddress;
    let output_buffer_desc = wgpu::BufferDescriptor {
        size: output_buffer_size,
        usage: wgpu::BufferUsages::COPY_DST
            // this tells wpgu that we want to read this buffer from the cpu
            | wgpu::BufferUsages::MAP_READ,
        label: None,
        mapped_at_creation: false,
    };
    let output_buffer = device.create_buffer(&output_buffer_desc);

    // A bind group layout describes the types of resources that a bind group can contain. Think
    // of this like a C-style header declaration, ensuring both the pipeline and bind group agree
    // on the types of resources.
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            // Input buffer
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
                resource: stops_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: dimensions_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: bounding_box_buffer.as_entire_binding(),
            },
        ],
    });

    let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/shader.wgsl"));

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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
                format: texture_desc.format,
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

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

    {
        let render_pass_desc = wgpu::RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture_view,
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
        };
        let mut render_pass = encoder.begin_render_pass(&render_pass_desc);

        render_pass.set_pipeline(&render_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            aspect: wgpu::TextureAspect::All,
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &output_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(pixels_height),
            },
        },
        texture_desc.size,
    );

    queue.submit(Some(encoder.finish()));

    // We need to scope the mapping variables so that we can
    // unmap the buffer
    {
        let buffer_slice = output_buffer.slice(..);

        // NOTE: We have to create the mapping THEN device.poll() before await
        // the future. Otherwise the application will freeze.
        let (tx, rx) = futures_intrusive::channel::shared::oneshot_channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.receive().await.unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();

        use image::{ImageBuffer, Rgba};

        // Extract actual pixel data by removing padding from each row
        let mut unpadded_data = Vec::with_capacity((pixels_width * pixels_height * 4) as usize);
        for chunk in data.chunks_exact(padded_bytes_per_row as usize) {
            unpadded_data.extend_from_slice(&chunk[..unpadded_bytes_per_row as usize]);
        }

        let buffer =
            ImageBuffer::<Rgba<u8>, _>::from_raw(pixels_width, pixels_height, unpadded_data)
                .unwrap();
        buffer.save(output_path).unwrap();
    }
    output_buffer.unmap();
}
