// This file contains all of the implementations related to shaders and rendering

use std::{collections::HashMap, time::Instant, u32};

use crate::{
    GTFSData,
    app::App,
    structs::{GpuGridCellKey, GpuGridCellVal, GpuStop},
};

use winit::event_loop::EventLoop;

/// Main function to run the app and the shaders
pub async fn run(gtfs_data: &GTFSData, arrival_times: &HashMap<u32, u32>) {
    let event_loop = EventLoop::new().unwrap();

    // Gpu spatial grid initialization
    let now = Instant::now();
    // gpu_grid_cell_keys, gpu_grid_cell_vals, and gpu_stops default to empty if hash build fails
    let (gpu_grid_cell_keys, gpu_grid_cell_vals, gpu_stops) =
        build_gpu_hash(gtfs_data, arrival_times).unwrap_or_default();
    println!("Gpu grid intiializing: {}ms\n", now.elapsed().as_millis());

    let mut app = App::new(gtfs_data, gpu_grid_cell_keys, gpu_grid_cell_vals, gpu_stops);

    event_loop.run_app(&mut app).unwrap();
}

/// Constructs 3 arrays for use by the shaders
///
/// Vec<GpuStop>: a simple array of the position and arrival time to each stop
/// Vec<GpuGridCellKey>: lookup keys to identify a spatial grid cell
/// Vec<GpuGridCellVal>: value tied to key that identifies where in the gpu stop array the cell correlates to
pub fn build_gpu_hash(
    gtfs_data: &GTFSData,
    arrival_times: &HashMap<u32, u32>,
) -> Result<(Vec<GpuGridCellKey>, Vec<GpuGridCellVal>, Vec<GpuStop>), Box<dyn std::error::Error>> {
    let mut keys: Vec<GpuGridCellKey> = vec![];
    let mut vals: Vec<GpuGridCellVal> = vec![];

    let mut stops: Vec<GpuStop> = vec![];

    for (&(lat_index, lon_index), stop_ids) in &gtfs_data.grid.map {
        // add key
        keys.push(GpuGridCellKey {
            lat_index,
            lon_index,
        });

        // add value
        vals.push(GpuGridCellVal {
            start: vals.len() as u32,
            count: stop_ids.len() as u32,
        });

        // add stops
        for stop_id in stop_ids {
            let stop = gtfs_data
                .stops
                .get(stop_id)
                .ok_or(format!("stop id not found -> {}", stop_id))?;

            // default to high arrival_time in case stop was not found to be reachable or something
            let arrival_time = *arrival_times.get(stop_id).unwrap_or(&u32::MAX);

            stops.push(GpuStop {
                lat: stop.position.lat,
                lon: stop.position.lon,
                arrival_time: arrival_time,
                _pad0: 0,
            })
        }
    }

    Ok((keys, vals, stops))
}
