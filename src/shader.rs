// This file contains all of the implementations related to shaders and rendering

use std::cmp::max;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    GTFSData, GpuGridCell, JFA_SCALE, Position,
    app::App,
    structs::{GpuGridCellKey, GpuGridCellVal, JFAConfig, ShaderConfig},
    utils::{hash2_i32, meters_per_pixel},
};

use tracing::{info_span, instrument};
use wgpu::{BufferUsages, Device, util::DeviceExt};
use winit::{event_loop::EventLoop, window::Window};

// TODO: switch to giving shader some kinda spatial grid rather than having it iterate through all stops for every pixel
// TODO: switch to a multi-stage aproach that first calculates the arrival time to each pixel, then turns that into a heatmap
/// stop_positions: (latitude, longitude)
pub async fn run(
    gtfs_data: &GTFSData,
    arrival_times: &HashMap<u32, u32>,
    gpu_grid_cells: Vec<GpuGridCell>,
    gpu_grid_stops: Vec<[f32; 4]>,
) {
    let event_loop = EventLoop::new().unwrap();

    let mut app = App::new(gtfs_data, arrival_times, gpu_grid_cells, gpu_grid_stops);

    event_loop.run_app(&mut app).unwrap();
}

pub fn build_gpu_hash(cells: &[GpuGridCell]) -> (Vec<GpuGridCellKey>, Vec<GpuGridCellVal>) {
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
