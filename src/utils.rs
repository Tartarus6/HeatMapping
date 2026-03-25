// This file contains all of the implementations for functions with general utility used in multiple files

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::{WALKING_SPEED, structs::Position};

/// gets the number of seconds taken to walk between 2 positions based on set walking speed
pub fn get_walk_time(from_position: Position, to_position: Position) -> u32 {
    let speed_mps = (WALKING_SPEED * 1000.0) / 3600.0;
    return ((haversine_distance(from_position, to_position)) / speed_mps) as u32;
}

/// gets distance in meters between 2 positions
const EARTH_RADIUS_METER: f32 = 6371000.0;
pub fn haversine_distance(position_a: Position, position_b: Position) -> f32 {
    let φ1: f32 = position_a.lat;
    let φ2: f32 = position_b.lat;
    let δφ: f32 = position_b.lat - position_a.lat;
    let δλ: f32 = position_b.lon - position_a.lon;

    let a: f32 = (δφ / 2.0).sin() * (δφ / 2.0).sin()
        + φ1.cos() * φ2.cos() * (δλ / 2.0).sin() * (δλ / 2.0).sin();
    let c: f32 = 2.0 * (a.sqrt()).asin();

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
    let mpp_y = EARTH_RADIUS_METER * dlat / height as f32;

    // meters per pixel horizontally (scaled by cos(latitude))
    let mpp_x = EARTH_RADIUS_METER * lat_center.cos() * dlon / width as f32;

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

/// get bbox from center (lat, lon) position and given zoom level that fits the aspect ratio of the given width and height
pub fn bbox_from_center(
    center: Position,
    half_lat_span: f32,
    width_px: u32,
    height_px: u32,
) -> (Position, Position) {
    let w = width_px.max(1) as f32;
    let h = height_px.max(1) as f32;
    let aspect = w / h;

    let cos_lat = center.lat.cos().abs().max(1e-6);

    let lat_span = 2.0 * half_lat_span;
    let lon_span = lat_span * aspect / cos_lat;

    let min = Position {
        lat: center.lat - 0.5 * lat_span,
        lon: center.lon - 0.5 * lon_span,
    };
    let max = Position {
        lat: center.lat + 0.5 * lat_span,
        lon: center.lon + 0.5 * lon_span,
    };

    (min, max)
}

/// hash function for gpu compatibility (used to compute hashes for a hashmap that can be used within shaders)
pub fn hash2_i32(a: i32, b: i32) -> u32 {
    let mut x = a as u32;
    let mut y = b as u32;

    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;

    y ^= y >> 16;
    y = y.wrapping_mul(0x7feb352d);
    y ^= y >> 15;
    y = y.wrapping_mul(0x846ca68b);
    y ^= y >> 16;

    x ^ y.rotate_left(16)
}

/// hash function to convert a string to u32 (used for string values, usually IDs, from gtfs data)
pub fn str_to_u32_hash(s: &str) -> u32 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);

    (hasher.finish() & 0xFFFF_FFFF) as u32
}

/// finds the log base 2 of an unsigned integer
pub fn log2(x: u32) -> u32 {
    debug_assert!(x > 0, "can't compute log of zero");
    31 - x.leading_zeros()
}
