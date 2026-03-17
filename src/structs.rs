// This file contains all of the data structures used throughout the code

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub struct DepartInstant {
    pub position: Position,
    /// seconds since midnight
    pub time: u32,
    pub date: Date,
}

#[derive(Serialize, Deserialize)]
pub struct SpatialGrid {
    /// <(lat_index, lon_index), list of stop_ids>
    pub map: HashMap<(i32, i32), Vec<u32>>,
    /// side length of each cell (in radians)
    pub cell_size: f64,
}

impl SpatialGrid {
    pub fn new(cell_size_meters: f64) -> Self {
        Self {
            map: HashMap::new(),
            cell_size: cell_size_meters / 6_371_000.0, // convert radians to meters (further handling is needed for latitude)
        }
    }

    pub fn insert(&mut self, position: Position, stop_id: u32) {
        let lat_index: i32 = (position.lat / self.cell_size).floor() as i32;
        let lon_index: i32 = (position.lon / self.cell_size).floor() as i32;

        self.map
            .entry((lat_index, lon_index))
            .or_insert_with(Vec::new)
            .push(stop_id);
    }

    pub fn get_nearby(&self, position: Position) -> Vec<u32> {
        let mut nearby: Vec<u32> = vec![];
        let lat_index: i32 = (position.lat / self.cell_size).floor() as i32;
        let lon_index: i32 = (position.lon / self.cell_size).floor() as i32;

        // At a given latitude, longitude degrees are smaller in physical size.
        // We need to search more longitude cells to cover the same physical distance.
        // cos(lat) gives the ratio of longitude degree size to latitude degree size.
        let lon_scale = position.lat.cos().max(1e-10); // clamp to avoid divide by zero
        // How many longitude cells fit in one latitude cell's worth of physical distance.
        // Add 1 to be safe at cell boundaries.
        // Also clamp to half the total longitude cells in a full 360° circle at this latitude,
        // since searching more than that would wrap around and be redundant.
        let total_lon_cells = (360.0 / self.cell_size) as i32; // total cells in a full circle
        let lon_range = ((1.0 / lon_scale).ceil() as i32 + 1).min((total_lon_cells + 1) / 2);

        // Check 3x3 grid around stop
        for d_lat in -1..=1 {
            for d_lon in -lon_range..=lon_range {
                nearby.extend(
                    self.map
                        .get(&(lat_index + d_lat, lon_index + d_lon))
                        .unwrap_or(&vec![]), // default to empty array if cell not initialized
                );
            }
        }
        return nearby;
    }
}

/// represents a position on the earth (latitude, longitude)
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Position {
    /// latitude in radians
    pub lat: f64,
    /// longitude in radians
    pub lon: f64,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Stop {
    pub stop_id: u32,
    // name: String,
    pub position: Position,
}

/**
Transit type (route_type in routes.txt) values:
 - 0 : Tram, Streetcar, Light rail. Any light rail or street level system within a metropolitan area.
 - 1 : Subway, Metro. Any underground rail system within a metropolitan area.
 - 2 : Rail. Used for intercity or long-distance travel.
 - 3 : Bus. Used for short- and long-distance bus routes.
 - 4 : Ferry. Used for short- and long-distance boat service.
 - 5 : Cable tram. Used for street-level rail cars where the cable runs beneath the vehicle (e.g., cable car in San Francisco).
 - 6 : Aerial lift, suspended cable car (e.g., gondola lift, aerial tramway). Cable transport where cabins, cars, gondolas or open chairs are suspended by means of one or more cables.
 - 7 : Funicular. Any rail system designed for steep inclines.
 - 11: Trolleybus. Electric buses that draw power from overhead wires using poles.
 - 12: Monorail. Railway in which the track consists of a single rail or a beam.*/
#[derive(Serialize, Deserialize)]
pub enum RouteType {
    TRAM,
    SUBWAY,
    RAIL,
    BUS,
    FERRY,
    CABLETRAM,
    AERIALLIFT,
    FUNICULAR,
    TROLLEYBUS,
    MONORAIL,
}

impl RouteType {
    /// converts route_type integer to RouteType enum
    pub fn parse_route_type(route_type: u32) -> RouteType {
        match route_type {
            0 => RouteType::TRAM,
            1 => RouteType::SUBWAY,
            2 => RouteType::RAIL,
            3 => RouteType::BUS,
            4 => RouteType::FERRY,
            5 => RouteType::CABLETRAM,
            6 => RouteType::AERIALLIFT,
            7 => RouteType::FUNICULAR,
            11 => RouteType::TROLLEYBUS,
            12 => RouteType::MONORAIL,
            _ => RouteType::BUS, // TODO: switch to some other default to specify that it's unknown
                                 // TODO: maybe add support for more route types, if needed
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Route {
    pub route_id: u32,
    pub route_type: RouteType,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
pub struct Trip {
    pub trip_id: u32,
    pub route_id: u32,
    pub service_id: u32,
    pub stop_times: Vec<StopTime>, // TODO: fix the duplication of stop_times (its A LOT of data)
}

#[derive(Serialize, Deserialize)]
pub struct StopTime {
    pub trip_id: u32,
    pub stop_sequence: u16,
    pub stop_id: u32,
    /// seconds since midnight (note, can sometimes be greater than 24 hours worth)
    pub arrival_time: u32,
    /// seconds since midnight (note, can sometimes be greater than 24 hours worth)
    pub departure_time: u32,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Date {
    pub year: u32,
    pub month: u8,
    pub day: u8,
}

impl Date {
    /// parses YYYYMMDD date string into Date struct
    pub fn parse_date_string(date_str: &str) -> Result<Date, Box<dyn std::error::Error>> {
        let date = Date {
            year: date_str[0..4].parse()?,
            month: date_str[4..6].parse()?,
            day: date_str[6..8].parse()?,
        };

        Ok(date)
    }
}

#[derive(Serialize, Deserialize)]
pub struct Transfer {
    pub from_stop_id: u32,
    pub to_stop_id: u32,
    /// (in seconds)
    pub min_transfer_time: u32,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Connection {
    pub from_stop_id: u32,
    pub to_stop_id: u32,
    /// id of parent trip
    pub trip_id: u32,
    /// time when arriving at destination (neighbor) stop (in seconds since midnight)
    pub arrival_time: u32,
    /// time when departing towards (neighbor) stop (in seconds since midnight)
    pub departure_time: u32,
}

#[derive(PartialEq, Serialize, Deserialize)]
pub enum ServiceExceptionType {
    ServiceAdded,
    ServiceRemoved,
}

impl ServiceExceptionType {
    /// converts exception_type integer to ServiceExceptionType enum
    pub fn parse_exception_type(exception_type: u32) -> ServiceExceptionType {
        match exception_type {
            1 => ServiceExceptionType::ServiceAdded,
            2 => ServiceExceptionType::ServiceRemoved,
            _ => ServiceExceptionType::ServiceRemoved,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct GTFSData {
    /// <stop_id, Stop>
    pub stops: HashMap<u32, Stop>,
    /// SpacialGrid of stop_ids
    pub grid: SpatialGrid,
    /// <route_id, Route>
    pub routes: HashMap<u32, Route>,
    /// <trip_id, Trip>
    pub trips: HashMap<u32, Trip>,
    /// <(service_id, Date), exception_type>
    pub services: HashMap<(u32, Date), ServiceExceptionType>,
    /// <from_stop_id, list of Transfers from stop>
    pub transfers: HashMap<u32, Vec<Transfer>>,
    /// <from_stop_id, list of Connections>
    pub connections: HashMap<u32, Vec<Connection>>,
}

// --- Shader Structs ---
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuGridCellKey {
    pub lat: i32,
    pub lon: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuGridCellVal {
    pub start: u32,
    pub count: u32,
}

// TODO: switch width, height, begin_time, and max_time to be u32
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShaderConfig {
    pub width: f32,  // how many pixels wide the image is
    pub height: f32, // how many pixels high the image is
    pub bbox_min_lat: f32,
    pub bbox_min_lon: f32,
    pub bbox_max_lat: f32,
    pub bbox_max_lon: f32,
    pub gpu_grid_cell_size: f32, // size of each cell (in radians)
    pub begin_time: f32,         // departure time in seconds since midnight
    // TODO: fix max time
    pub max_time: f32, // latest arrival time in seconds since midnight
    pub inverse_walk_speed_mps: f32, // walking speed in seconds per meter
}

// TODO: switch width, height, and jump_size to be u32
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct JFAConfig {
    pub jfa_width: f32,       // how many pixels wide the image is
    pub jfa_height: f32,      // how many pixels high the image is
    pub jump_size: f32,       // jump size for JFA
    pub meters_per_px_x: f32, // approximate number of meters per x pixel
    pub meters_per_px_y: f32, // approximate number of meters per y pixel
}

/// parses stop_id, handling both "600737" and "stoparea:600737" formats
pub fn parse_stop_id(stop_id_str: &str) -> Result<u32, Box<dyn std::error::Error>> {
    let stop_id: u32;

    if let Some(pos) = stop_id_str.rfind(':') {
        stop_id = stop_id_str[pos + 1..].parse()?;
    } else {
        stop_id = stop_id_str.parse()?;
    }

    Ok(stop_id)
}

/// parses route_id, handling both "600737" and "stoparea:600737" formats
pub fn parse_route_id(route_id_str: &str) -> Result<u32, Box<dyn std::error::Error>> {
    let route_id: u32;

    if route_id_str.rfind('_').is_some() {
        route_id = route_id_str.replace('_', "").parse()?;
    } else {
        route_id = route_id_str.parse()?;
    }

    Ok(route_id)
}
