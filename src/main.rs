use pollster::FutureExt;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;
use std::{f64, u32};

use crate::dijkstra::initialize_dijkstra;
use crate::parse::initialize_data;
use crate::structs::{Date, DepartInstant, GTFSData, Position};

mod dijkstra;
mod parse;
mod shader;
mod structs;
mod utils;

/// controls the size of the heatmap output, the aspect ratio changes based on bounding box, but this controls the longest side
const MAX_DIM: u32 = 512;
/// walking speed in kilometers per hour
const WALKING_SPEED: f64 = 5.0;
/// maximum distance to walk between stops (used for culling) (this option can be too greedy, it can cull optimal paths) (distance in meters)
const MAX_WALK_TRANSFER_DISTANCE: f64 = 20000.0;

// TODO: remove bounding box consts, since it's all handled in render.rs anyways (could instead have either const starting center point, or compute the average of all stops maybe)
/// bounding box for the heatmap output (Amsterdam-ish area)
const BBOX_MIN: Position = Position {
    lat: 0.87,
    lon: 0.04,
};
/// bounding box for the heatmap output (Amsterdam-ish area)
const BBOX_MAX: Position = Position {
    lat: 0.94,
    lon: 0.16,
};

/// constants for where/when we are starting from
const DEPART_INSTANT: DepartInstant = DepartInstant {
    position: Position {
        // Amsterdam
        lat: 0.913998595445,
        lon: 0.085599725524,
        // Copenhagen (i think)
        // lat: 0.972092,
        // lon: 0.218484,
    },
    time: 32400, // 09:00:00
    date: Date {
        year: 2026,
        month: 03,
        day: 13,
    },
};

// const OUTPUT_DIRECTORY: &str = "./output/";
const CACHE_DIRECTORY: &str = "./cache/";
const GTFS_DIRECTORY: &str = "./src/lib/GTFS/";

fn main() {
    let now = Instant::now();

    // Gtfs data initialization
    let gtfs_data = match load_gtfs_data("cache/gtfs_data") {
        // try loading from cache if possible
        Ok(data) => data,
        Err(_) => {
            println!("Cache not found - parsing GTFS data...");

            // if couldnt load gtfs data from cache, parse from gtfs files
            let data = match initialize_data() {
                Ok(data) => data,
                Err(err) => panic!("error parsing gtfs data: {:?}", err),
            };

            // save that parsed data into the cache
            match save_gtfs_data(&data, format!("{CACHE_DIRECTORY}{}", "gtfs_data").as_str()) {
                Ok(()) => (),
                Err(err) => panic!("error saving gtfs data: {:?}", err),
            };

            data // return that data
        }
    };
    println!("Initializing: {}ms\n", now.elapsed().as_millis());

    // Dijkstra
    let now = Instant::now();
    let arrival_times = match initialize_dijkstra(&gtfs_data) {
        Ok(out) => out,
        Err(err) => panic!("error running dijkstra: {:?}", err),
    };
    println!("Dijkstra: {}ms\n", now.elapsed().as_millis());

    // Gpu spatial grid initialization
    let now = Instant::now();
    let (gpu_grid_cells, gpu_grid_stops) = match initialize_gpu_grid(&gtfs_data, &arrival_times) {
        Ok(data) => data,
        Err(err) => {
            panic!("gpu grid initializing error: {:?}", err);
        }
    };
    println!("Gpu grid intiializing: {}ms\n", now.elapsed().as_millis());

    // Shader
    let now = Instant::now();
    shader::run(
        &gtfs_data,
        &arrival_times,
        gpu_grid_cells,
        gpu_grid_stops,
        BBOX_MIN,
        BBOX_MAX,
    )
    .block_on();
    println!("Heatmap: {}ms\n", now.elapsed().as_millis());
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuGridCell {
    lat_index: i32,
    lon_index: i32,
    start: u32,
    count: u32,
}

// TODO: remove this function. purpose is superceeded by ones in shader.rs (will need to refactor those supeceeding functions to use SpatialGrid instead of the output of this function)
fn initialize_gpu_grid(
    gtfs_data: &GTFSData,
    arrival_times: &HashMap<u32, u32>,
) -> Result<(Vec<GpuGridCell>, Vec<[f32; 4]>), Box<dyn std::error::Error>> {
    let mut gpu_grid_cells: Vec<GpuGridCell> = Vec::new();
    let mut gpu_grid_stops: Vec<[f32; 4]> = Vec::new(); // entries are (stop_lat, stop_lon, arrival_time)

    for (&(lat_index, lon_index), stop_ids) in &gtfs_data.grid.map {
        let start = gpu_grid_stops.len() as u32;
        let count = stop_ids.len() as u32;

        // add the stop_ids from the cell into the array
        for stop_id in stop_ids {
            let stop = gtfs_data
                .stops
                .get(&stop_id)
                .ok_or(format!("stop id not found -> {}", stop_id))?;
            let arrival_time: &u32 = arrival_times.get(&stop_id).unwrap_or(&u32::MAX); // default to high arrival time if not found

            gpu_grid_stops.push([
                stop.position.lat as f32,
                stop.position.lon as f32,
                *arrival_time as f32,
                0.0,
            ]);
        }

        // add cell as entry into grid cells
        gpu_grid_cells.push(GpuGridCell {
            lat_index,
            lon_index,
            start,
            count,
        });
    }

    println!("gpu grid cell count: {}", gpu_grid_stops.len());

    Ok((gpu_grid_cells, gpu_grid_stops))
}

fn save_gtfs_data(data: &GTFSData, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = postcard::to_allocvec(data)?;
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

fn load_gtfs_data(path: &str) -> Result<GTFSData, Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    let data = postcard::from_bytes(&buffer)?;

    Ok(data)
}
