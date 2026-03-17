// This file contains all of the implementations for functions with general utility used in multiple files

use crate::{WALKING_SPEED, structs::Position};

/// gets the number of seconds taken to walk between 2 positions based on set walking speed
pub fn get_walk_time(from_position: Position, to_position: Position) -> u32 {
    let speed_mps = (WALKING_SPEED * 1000.0) / 3600.0;
    return ((haversine_distance(from_position, to_position)) / speed_mps) as u32;
}

/// gets distance in meters between 2 positions
const EARTH_RADIUS_METER: f64 = 6371000.0;
pub fn haversine_distance(position_a: Position, position_b: Position) -> f64 {
    let φ1: f64 = position_a.lat;
    let φ2: f64 = position_b.lat;
    let δφ: f64 = position_b.lat - position_a.lat;
    let δλ: f64 = position_b.lon - position_a.lon;

    let a: f64 = (δφ / 2.0).sin() * (δφ / 2.0).sin()
        + φ1.cos() * φ2.cos() * (δλ / 2.0).sin() * (δλ / 2.0).sin();
    let c: f64 = 2.0 * (a.sqrt()).asin();

    return EARTH_RADIUS_METER * c;
}

/// approximates the size of one pixel based on texture size and bounding box
pub fn meters_per_pixel(
    bbox_min_position: Position,
    bbox_max_position: Position,
    width: u32,
    height: u32,
) -> (f32, f32) {
    let dlat = bbox_max_position.lat - bbox_min_position.lat;
    let dlon = bbox_max_position.lon - bbox_min_position.lon;
    let lat_center = 0.5 * (bbox_min_position.lat + bbox_max_position.lat);

    // meters per pixel vertically
    let mpp_y = EARTH_RADIUS_METER * dlat / height as f64;

    // meters per pixel horizontally (scaled by cos(latitude))
    let mpp_x = EARTH_RADIUS_METER * lat_center.cos() * dlon / width as f64;

    (mpp_x.abs() as f32, mpp_y.abs() as f32)
}

/// turns time in hh:mm:ss format into number of seconds since midnight
pub fn str_time_to_seconds(time_str: &str) -> Result<u32, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = time_str.split(":").collect();

    assert_eq!(parts.len(), 3); // parts should have ["hh", "mm", "ss"], otherwise panic

    let hours: u32 = parts[0].parse()?;
    let minutes: u32 = parts[1].parse()?;
    let seconds: u32 = parts[2].parse()?;

    Ok(hours * 3600 + minutes * 60 + seconds)
}

/// turns time in seconds since midnight into hh:mm:ss format
pub fn seconds_to_str_time(time: &u32) -> String {
    let hours = time / 3600;
    let minutes = (time % 3600) / 60;
    let seconds = time % 60;
    return format!("{:02}:{:02}:{:02}", hours, minutes, seconds);
}
