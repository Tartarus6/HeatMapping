//! This is a mapping engine, kind of like the bakend of google maps.
//!
//! The engine is provided GTFS data of a region (such as Amsterdam), and will do all of the math behind mapping throughout that region.

use pollster::FutureExt;
use std::time::Instant;
use std::u32;

use crate::dijkstra::initialize_dijkstra;
use crate::gtfs::get_gtfs_data;
use crate::structs::{Date, DepartInstant, GTFSData, Position};

mod app;
mod dijkstra;
mod gtfs;
mod render_state;
mod shader;
mod structs;
mod utils;

/// controls the size of the heatmap output, the aspect ratio changes based on bounding box, but this controls the longest side
const MAX_DIM: u32 = 512;
/// walking speed in kilometers per hour
const WALKING_SPEED: f32 = 5.0;
/// maximum distance to walk between stops (used for culling) (this option can be too greedy, it can cull optimal paths) (distance in meters)
const MAX_WALK_TRANSFER_DISTANCE: f32 = 5000.0;

/// initial zoom control (half of latitude span in radians)
/// bigger value means zoomed further out.
const INITIAL_HALF_LAT_SPAN: f32 = 0.03;

// TODO: switch to automatically setting and updating jfa scale based on window dimensions (or maybe measure performance and increase if too slow)
/// integer scale of jfa render
/// 2 would mean jfa width and height are half of output
const JFA_SCALE: u32 = 8;

/// constants for where/when we are starting from
const DEPART_INSTANT: DepartInstant = DepartInstant {
    position: Position {
        // Amsterdam
        // lat: 0.913998595445,
        // lon: 0.085599725524,
        // Copenhagen (i think)
        lat: 0.972092,
        lon: 0.218484,
    },
    time: 32400, // 09:00:00
    date: Date {
        year: 2026,
        month: 03,
        day: 13,
    },
};

const CACHE_DIRECTORY: &str = "./cache/";
const GTFS_DIRECTORY: &str = "./GTFS/";

fn main() {
    tracing_subscriber::fmt()
        .with_span_events(
            tracing_subscriber::fmt::format::FmtSpan::ENTER
                | tracing_subscriber::fmt::format::FmtSpan::CLOSE,
        )
        .init();

    let now = Instant::now();

    // Gtfs data initialization
    let gtfs_data = get_gtfs_data();
    println!("Initializing: {}ms\n", now.elapsed().as_millis());

    // Dijkstra
    let now = Instant::now();
    let arrival_times = match initialize_dijkstra(&gtfs_data) {
        Ok(out) => out,
        Err(err) => panic!("error running dijkstra: {:?}", err),
    };
    println!("Dijkstra: {}ms\n", now.elapsed().as_millis());

    // Shader
    shader::run(&gtfs_data, &arrival_times).block_on();
}
