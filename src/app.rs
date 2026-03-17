use std::cmp::max;
use std::{collections::HashMap, sync::Arc};

use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::window::WindowId;
use winit::{application::ApplicationHandler, event_loop::ActiveEventLoop, window::Window};

use crate::MAX_WALK_TRANSFER_DISTANCE;
use crate::utils::meters_per_pixel;
use crate::{
    DEPART_INSTANT, GpuGridCell, INITIAL_HALF_LAT_SPAN, JFA_SCALE, MAX_DIM, WALKING_SPEED,
    shader::{RenderState, build_gpu_hash},
    structs::{GTFSData, GpuGridCellKey, GpuGridCellVal, JFAConfig, ShaderConfig},
    utils::bbox_from_center,
};

// Holds data needed before the window is created, and the render state after
pub struct App {
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
    pub fn new(
        gtfs_data: &GTFSData,
        arrival_times: &HashMap<u32, u32>,
        gpu_grid_cells: Vec<GpuGridCell>,
        gpu_grid_stops: Vec<[f32; 4]>,
    ) -> Self {
        // derive image dimensions from the bounding box aspect ratio
        // longitude degrees are physically shorter at higher latitudes, scale by cos(mid_lat)
        // let mid_lat = (bbox_min_position.lat + bbox_max_position.lat) / 2.0;
        // let physical_width = (bbox_max_position.lon - bbox_min_position.lon) * mid_lat.cos();
        // let physical_height = bbox_max_position.lat - bbox_min_position.lat;
        // let aspect_ratio = physical_width / physical_height;
        // let (pixels_width, pixels_height) = if aspect_ratio >= 1.0 {
        //     (MAX_DIM, (MAX_DIM as f64 / aspect_ratio) as u32)
        // } else {
        //     ((MAX_DIM as f64 * aspect_ratio) as u32, MAX_DIM)
        // };

        // TODO: replace max_time with actual processing stage to calculate it
        let begin_time = DEPART_INSTANT.time;

        let shader_config = ShaderConfig {
            width: MAX_DIM as f32,  // dummy init value (will be overwritten)
            height: MAX_DIM as f32, // dummy init value (will be overwritten)
            bbox_min_lat: 0.0,      // dummy init value (will be overwritten)
            bbox_min_lon: 1.0,      // dummy init value (will be overwritten)
            bbox_max_lat: 0.0,      // dummy init value (will be overwritten)
            bbox_max_lon: 1.0,      // dummy init value (will be overwritten)
            gpu_grid_cell_size: gtfs_data.grid.cell_size as f32,
            begin_time: begin_time as f32,
            max_walk_transfer_distance: MAX_WALK_TRANSFER_DISTANCE as f32,
            inverse_walk_speed_mps: 1.0 / ((WALKING_SPEED * 1000.0) / 3600.0) as f32,
        };

        // let jfa_width = max(1, pixels_width / JFA_SCALE);
        // let jfa_height = max(1, pixels_height / JFA_SCALE);

        // let meters_per_pixel =
        //     meters_per_pixel(bbox_min_position, bbox_max_position, jfa_width, jfa_height);

        let jfa_config = JFAConfig {
            jfa_width: 0.0,       // dummy init value (will be overwritten)
            jfa_height: 0.0,      // dummy init value (will be overwritten)
            jump_size: 0.0,       // dummy init value (will be overwritten)
            meters_per_px_x: 0.0, // dummy init value (will be overwritten)
            meters_per_px_y: 0.0, // dummy init value (will be overwritten)
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
                .create_window(Window::default_attributes().with_title("HeatMapping"))
                .unwrap(),
        );

        let size = window.inner_size();
        let center = DEPART_INSTANT.position;
        let (bbox_min, bbox_max) =
            bbox_from_center(center, INITIAL_HALF_LAT_SPAN, size.width, size.height);

        // update configs to match real startup window
        self.shader_config.width = size.width as f32;
        self.shader_config.height = size.height as f32;
        self.shader_config.bbox_min_lat = bbox_min.lat as f32;
        self.shader_config.bbox_min_lon = bbox_min.lon as f32;
        self.shader_config.bbox_max_lat = bbox_max.lat as f32;
        self.shader_config.bbox_max_lon = bbox_max.lon as f32;

        let jfa_w = max(1, size.width / JFA_SCALE);
        let jfa_h = max(1, size.height / JFA_SCALE);
        self.jfa_config.jfa_width = jfa_w as f32;
        self.jfa_config.jfa_height = jfa_h as f32;

        let mpp = meters_per_pixel(bbox_min, bbox_max, jfa_w, jfa_h);
        self.jfa_config.meters_per_px_x = mpp.0;
        self.jfa_config.meters_per_px_y = mpp.1;

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
