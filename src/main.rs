use csv::Reader;
use image::{ImageBuffer, Rgb};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::fs::File;
use std::time::Instant;
use std::u32;

const MAX_DIM: u32 = 500;
const WALKING_SPEED: f64 = 5.0; // walking speed in kilometers per hour
const MAX_WALK_TRANSFER_DISTANCE: f64 = 5000.0; // maximum distance to walk between stops (used for culling) (this option can be too greedy, it can cull optimal paths) (distance in meters)

// bounding box for the heatmap output (Amsterdam area)
const BBOX_MIN_LAT: f64 = 52.032003;
const BBOX_MAX_LAT: f64 = 52.505422;
const BBOX_MIN_LON: f64 = 4.407175;
const BBOX_MAX_LON: f64 = 5.247558;

const DEPART_INSTANT: DepartInstant = DepartInstant {
    position: Position {
        lat: 52.368262,
        lon: 4.904503,
    },
    time: 32400,
    date: Date {
        year: 2026,
        month: 03,
        day: 13,
    },
};

struct SpatialGrid {
    map: HashMap<(i32, i32), Vec<u32>>, // <(lat_index, lon_index), list of stop_ids>
    cell_size: f64,
}

impl SpatialGrid {
    fn new(cell_size_meters: f64) -> Self {
        Self {
            map: HashMap::new(),
            cell_size: cell_size_meters / 111_320.0, // convert meters to degrees (further handling is needed for latitude)
        }
    }

    fn insert(&mut self, position: Position, stop_id: u32) {
        // TODO: switch index calculations to be consistend (currently cells near the earth's poles are much smaller than ones near the equator)
        let lat_index: i32 = (position.lat / self.cell_size).floor() as i32;
        let lon_index: i32 = (position.lon / self.cell_size).floor() as i32;

        self.map
            .entry((lat_index, lon_index))
            .or_insert_with(Vec::new)
            .push(stop_id);
    }

    fn get_nearby(&self, position: Position) -> Vec<u32> {
        let mut nearby: Vec<u32> = vec![];
        let lat_index: i32 = (position.lat / self.cell_size).floor() as i32;
        let lon_index: i32 = (position.lon / self.cell_size).floor() as i32;

        // At a given latitude, longitude degrees are smaller in physical size.
        // We need to search more longitude cells to cover the same physical distance.
        // cos(lat) gives the ratio of longitude degree size to latitude degree size.
        let lon_scale = position.lat.to_radians().cos().max(1e-10); // clamp to avoid divide by zero
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
                        .unwrap_or(&vec![]),
                );
            }
        }
        return nearby;
    }
}

struct DepartInstant {
    position: Position,
    time: u32, // seconds since midnight
    date: Date,
}

// represents a position
#[derive(Clone, Copy)]
struct Position {
    lat: f64,
    lon: f64,
}

#[derive(Clone, Copy)]
struct Stop {
    stop_id: u32,
    // name: String,
    position: Position,
}

/*
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
 - 12: Monorail. Railway in which the track consists of a single rail or a beam.
*/
enum RouteType {
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
struct Route {
    route_id: u32,
    route_type: RouteType,
    name: String,
}

struct Trip {
    trip_id: u32,
    route_id: u32,
    service_id: u32,
    stop_times: Vec<StopTime>, // TODO: fix the duplication of stop_times (its A LOT of data)
}

struct StopTime {
    trip_id: u32,
    stop_sequence: u16,
    stop_id: u32,
    arrival_time: u32, // seconds since midnight (note, can sometimes be greater than 24 hours worth)
    departure_time: u32, // seconds since midnight (note, can sometimes be greater than 24 hours worth)
}

struct Date {
    year: u32,
    month: u8,
    day: u8,
}
// struct Service {
//     service_id: u32,
//     dates_active: Vec<Date>, // list of dates this service runs for
// }

struct Transfer {
    from_stop_id: u32,
    to_stop_id: u32,
    min_transfer_time: u32, // seconds
}

#[derive(Clone, Copy)]
struct Connection {
    from_stop_id: u32,
    to_stop_id: u32,
    trip_id: u32,        // id of parent trip
    arrival_time: u32, // time when arriving at destination (neighbor) stop (in seconds since midnight)
    departure_time: u32, // time when departing towards (neighbor) stop (in seconds since midnight)
}

struct GTFSData {
    stops: HashMap<u32, Stop>,
    grid: SpatialGrid,
    routes: HashMap<u32, Route>,
    trips: HashMap<u32, Trip>,
    services: HashMap<u32, Vec<Date>>, // <service_id, dates that service runs for>
    transfers: HashMap<u32, Vec<Transfer>>, // <from_stop_id, list of transfers from stop>
    connections: HashMap<u32, Vec<Connection>>, // <from_stop_id, list of connections>
}

fn main() {
    let now = Instant::now();
    let gtfs_data = initialize_data().unwrap();
    println!("Initializing: {}ms", now.elapsed().as_millis());

    let now = Instant::now();
    let travel_times = initialize_dijkstra(&gtfs_data).unwrap();
    println!("Dijkstra: {}ms", now.elapsed().as_millis());
    println!("{}", travel_times.len());
    println!("initialized dijkstra");

    println!(
        "depart instant time: {}",
        seconds_to_str_time(&DEPART_INSTANT.time)
    );
    generate_heatmap(&gtfs_data, &travel_times, "heatmap.png");
    println!("Heatmap saved to heatmap.png");
}

fn initialize_data() -> Result<GTFSData, Box<dyn std::error::Error>> {
    let mut gtfs_data = GTFSData {
        stops: HashMap::new(),
        grid: SpatialGrid::new(MAX_WALK_TRANSFER_DISTANCE),
        routes: HashMap::new(),
        trips: HashMap::new(),
        services: HashMap::new(),
        transfers: HashMap::new(),
        connections: HashMap::new(),
    };

    // stops
    let file = File::open("./src/lib/gtfs-nl/stops.txt")?;
    let mut reader = Reader::from_reader(file);
    for result in reader.records() {
        let record = result?;
        let stop_id = parse_stop_id(&record[0]);
        let position = Position {
            lat: record[3].parse().unwrap(),
            lon: record[4].parse().unwrap(),
        };

        gtfs_data.stops.insert(
            stop_id,
            Stop {
                stop_id: stop_id,
                // name: record[2].to_string(),
                position: position,
            },
        );

        gtfs_data.grid.insert(position, stop_id);
    }
    println!("Loaded {} stops", gtfs_data.stops.len());

    // routes
    let file = File::open("./src/lib/gtfs-nl/routes.txt")?;
    let mut reader = Reader::from_reader(file);
    for result in reader.records() {
        let record = result?;
        let route_id = record[0].parse().unwrap();

        gtfs_data.routes.insert(
            route_id,
            Route {
                route_id: route_id,
                route_type: parse_route_type(record[5].parse().unwrap()),
                name: record[3].to_string(),
            },
        );
    }
    println!("Loaded {} routes", gtfs_data.routes.len());

    // trips
    let file = File::open("./src/lib/gtfs-nl/trips.txt")?;
    let mut reader = Reader::from_reader(file);
    for result in reader.records() {
        let record = result?;
        let trip_id = record[2].parse().unwrap();

        gtfs_data.trips.insert(
            trip_id,
            Trip {
                trip_id: trip_id,
                route_id: record[0].parse().unwrap(),
                service_id: record[1].parse().unwrap(),
                stop_times: vec![],
            },
        );
    }
    println!("Loaded {} trips", gtfs_data.trips.len());

    // stop_times
    let file = File::open("./src/lib/gtfs-nl/stop_times.txt")?;
    let mut reader = Reader::from_reader(file);
    let mut stop_times_count: u32 = 0;
    for result in reader.records() {
        let record = result?;
        let trip_id = record[0].parse().unwrap();

        let trip = gtfs_data
            .trips
            .get_mut(&trip_id)
            .ok_or("stop time trip didn't exist")?;
        trip.stop_times.push(StopTime {
            trip_id: trip_id,
            stop_sequence: record[1].parse().unwrap(),
            stop_id: parse_stop_id(&record[2]),
            arrival_time: str_time_to_seconds(&record[4]),
            departure_time: str_time_to_seconds(&record[5]),
        });
        stop_times_count += 1;
    }
    println!("Loaded {} stop_times", stop_times_count);

    // services (from calendar_dates)
    let file = File::open("./src/lib/gtfs-nl/calendar_dates.txt")?;
    let mut reader = Reader::from_reader(file);
    // let mut service_map: HashMap<u32, Vec<Date>> = HashMap::new();
    for result in reader.records() {
        let record = result?;
        let service_id: u32 = record[0].parse().unwrap();
        let date = parse_date(&record[1]);

        gtfs_data
            .services
            .entry(service_id)
            .or_insert_with(Vec::new)
            .push(date);
    }
    println!("Loaded {} services", gtfs_data.services.len());

    // transfers
    let file = File::open("./src/lib/gtfs-nl/transfers.txt")?;
    let mut reader = Reader::from_reader(file);
    for result in reader.records() {
        let record = result?;
        let from_stop_id: u32 = parse_stop_id(&record[0]);

        gtfs_data
            .transfers
            .entry(from_stop_id)
            .or_insert_with(Vec::new)
            .push(Transfer {
                from_stop_id: from_stop_id,
                to_stop_id: parse_stop_id(&record[1]),
                min_transfer_time: record[7].parse().unwrap_or(0),
            });
    }
    for from_stop in gtfs_data.stops.values() {
        let culled_stops = gtfs_data.grid.get_nearby(from_stop.position);

        for to_stop_id in culled_stops {
            if from_stop.stop_id == to_stop_id {
                continue;
            }
            let to_stop = gtfs_data
                .stops
                .get(&to_stop_id)
                .ok_or("to stop not in stops")?;
            gtfs_data
                .transfers
                .entry(from_stop.stop_id)
                .or_insert_with(Vec::new)
                .push(Transfer {
                    from_stop_id: from_stop.stop_id,
                    to_stop_id: to_stop.stop_id,
                    min_transfer_time: get_walk_time(from_stop.position, to_stop.position),
                });
        }
    }
    println!("Loaded {} transfers", gtfs_data.transfers.len());

    // connections
    for (_, trip) in gtfs_data.trips.iter_mut() {
        // sort stop times to be in order
        trip.stop_times
            .sort_by(|a, b| a.departure_time.cmp(&b.departure_time));

        // skipping last index since we are looking at pairs of stops
        for i in 0..trip.stop_times.len() - 1 {
            let from_stop_id = trip.stop_times[i].stop_id;

            gtfs_data
                .connections
                .entry(from_stop_id)
                .or_insert_with(Vec::new)
                .push(Connection {
                    from_stop_id: from_stop_id,
                    to_stop_id: trip.stop_times[i + 1].stop_id,
                    trip_id: trip.trip_id,
                    arrival_time: trip.stop_times[i + 1].arrival_time,
                    departure_time: trip.stop_times[i].departure_time,
                });
        }
    }

    println!("Loaded {} connections", gtfs_data.connections.len());

    Ok(gtfs_data)
}

// TODO: is there a way to reuse the data from other dijkstra runs, rather than having to totally recalculate for each different starting position?
// TODO: (maybe) optimize by finding "hub nodes", and precomputing the travel times between them. then using that hub-to-hub time as an offset to prevent the need to calculate paths across hubs
// runs a multi-souce dijkstra, running once with each stop as the starting position
// returns HashMap<to_stop_id: u32, arrival_time: u32> (arrival time in secons since midnight)
fn initialize_dijkstra(
    gtfs_data: &GTFSData,
) -> Result<HashMap<u32, u32>, Box<dyn std::error::Error>> {
    let mut arrival_times: HashMap<u32, u32> = HashMap::new(); // <to_stop_id, arrival_time>

    // get culled connections list, removing any entries that occured before the depart instant
    let culled_connections =
        get_culled_connections(DEPART_INSTANT.time, u32::MAX, &gtfs_data.connections);

    let mut priority_queue: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new(); // Min-heap (priority queue) storing pairs of (arrival_time, stop_id)

    // add the time taken to walk to any stop
    for stop in gtfs_data.stops.values() {
        let arrival_time =
            DEPART_INSTANT.time + get_walk_time(DEPART_INSTANT.position, stop.position);
        arrival_times.insert(stop.stop_id, arrival_time);
        priority_queue.push(Reverse((arrival_time, stop.stop_id))); // push the starting stop onto the priority queue
    }

    // process the queue until all reachable stops are finalized
    while !priority_queue.is_empty() {
        let Reverse((current_stop_arrival_time, current_stop_id)) =
            priority_queue.pop().ok_or("priority queue empty")?;

        // if this distance not the latest shortest one, skip it
        if current_stop_arrival_time
            > arrival_times
                .get(&current_stop_id)
                .copied()
                .ok_or("stop id not in distances")?
        {
            continue;
        }

        // TODO: (maybe) fix code repetition between trip neighbor exploration and transfer neighbor exploration

        // explore all trip connections of the current stop
        for connection in culled_connections.get(&current_stop_id).unwrap_or(&vec![]) {
            // TODO: if this connection is not in service, skip it

            // TODO: switch to using binary search instead of iterating through until it's found

            // if departure_time already passed, skip it
            if connection.departure_time < current_stop_arrival_time {
                continue;
            }

            // if new faster path found, update that travel time and add that node onto the priority queue
            if connection.arrival_time
                < *arrival_times
                    .get(&connection.to_stop_id)
                    .unwrap_or(&u32::MAX)
            {
                arrival_times.insert(connection.to_stop_id, connection.arrival_time);
                priority_queue.push(Reverse((connection.arrival_time, connection.to_stop_id)));
            }
        }

        // explore all transfer connections of the current stop
        for transfer in gtfs_data.transfers.get(&current_stop_id).unwrap_or(&vec![]) {
            // if new faster path found, update that travel time and add that node onto the priority queue
            if transfer.min_transfer_time + current_stop_arrival_time
                < *arrival_times.get(&transfer.to_stop_id).unwrap_or(&u32::MAX)
            {
                arrival_times.insert(
                    transfer.to_stop_id,
                    transfer.min_transfer_time + current_stop_arrival_time,
                );
                priority_queue.push(Reverse((
                    transfer.min_transfer_time + current_stop_arrival_time,
                    transfer.to_stop_id,
                )));
            }
        }

        // TODO: does implementing walking connections prevent the need for handling transfer connections?
        // explore all walking connections of the current stop
        // for other_stop in gtfs_data.stops.values() {
        //     // calculate walk time
        //     let walk_arrival_time = current_stop_arrival_time
        //         + get_walk_time(
        //             gtfs_data
        //                 .stops
        //                 .get(&current_stop_id)
        //                 .ok_or("stop id not in stops")?
        //                 .position,
        //             other_stop.position,
        //         );

        //     // if walk time is faster than saved time to stop, update it
        //     if walk_arrival_time < *arrival_times.get(&other_stop.stop_id).unwrap_or(&u32::MAX) {
        //         arrival_times.insert(other_stop.stop_id, walk_arrival_time);
        //         priority_queue.push(Reverse((walk_arrival_time, other_stop.stop_id))); // push the starting stop onto the priority queue
        //     }
        // }
    }

    Ok(arrival_times)
}

fn generate_heatmap(gtfs_data: &GTFSData, travel_times: &HashMap<u32, u32>, output_path: &str) {
    let (min_lat, max_lat, min_lon, max_lon) =
        (BBOX_MIN_LAT, BBOX_MAX_LAT, BBOX_MIN_LON, BBOX_MAX_LON);

    // derive image dimensions from the bounding box aspect ratio
    // longitude degrees are physically shorter at higher latitudes, scale by cos(mid_lat)
    let mid_lat = (min_lat + max_lat) / 2.0;
    let physical_width = (max_lon - min_lon) * mid_lat.to_radians().cos();
    let physical_height = max_lat - min_lat;
    let aspect_ratio = physical_width / physical_height;
    let (width, height) = if aspect_ratio >= 1.0 {
        (MAX_DIM, (MAX_DIM as f64 / aspect_ratio) as u32)
    } else {
        ((MAX_DIM as f64 * aspect_ratio) as u32, MAX_DIM)
    };

    println!("width : {}", width);
    println!("height: {}", height);
    println!("aspect: {}\n", aspect_ratio);

    let mut pixel_arrival_time_map: HashMap<(u32, u32), u32> = HashMap::new(); // <(px, py), arrival_time>

    // find min/max travel times for color scaling
    // let min_time = DEPART_INSTANT.time;
    // let max_time = travel_times
    //     .values()
    //     .copied()
    //     .filter(|&t| t < u32::MAX)
    //     .max()
    //     .unwrap_or(min_time + 1);
    let mut max_time: u32 = 0; // stores the latest pixel arrival_time found

    for py in 0..height {
        for px in 0..width {
            // map pixel to lat/lon
            let lat = max_lat - (py as f64 / height as f64) * (max_lat - min_lat); // flip y axis
            let lon = min_lon + (px as f64 / width as f64) * (max_lon - min_lon);

            let pixel_pos = Position { lat, lon };

            // find nearest stop
            let nearby_stops = gtfs_data.grid.get_nearby(pixel_pos);
            let nearest_stop_id = nearby_stops.iter().min_by(|a, b| {
                let da = haversine_distance(pixel_pos, gtfs_data.stops.get(a).unwrap().position);
                let db = haversine_distance(pixel_pos, gtfs_data.stops.get(b).unwrap().position);
                da.partial_cmp(&db).unwrap()
            });

            match nearest_stop_id.and_then(|stop_id| travel_times.get(stop_id)) {
                Some(&arrival_time) => {
                    max_time = max_time.max(arrival_time);
                    pixel_arrival_time_map.insert((px, py), arrival_time);
                }
                None => (),
            };
        }
    }

    let mut img = ImageBuffer::new(width, height);

    for ((px, py), arrival_time) in pixel_arrival_time_map {
        let t = (arrival_time.saturating_sub(DEPART_INSTANT.time) as f64)
            / (max_time - DEPART_INSTANT.time) as f64;
        let color = travel_time_to_color(t.clamp(0.0, 1.0));

        // if haversine_distance(DEPART_INSTANT.position, pixel_pos) < 500.0 {
        //     color = Rgb([0, 0, 255]);
        // }

        img.put_pixel(px, py, color);
    }

    img.save(output_path).unwrap();
}

/// Maps a normalized travel time (0.0 = fastest, 1.0 = slowest) to a color.
/// green -> yellow -> red
fn travel_time_to_color(t: f64) -> Rgb<u8> {
    if t < 0.5 {
        // green to yellow
        let s = t * 2.0;
        Rgb([(255.0 * s) as u8, 255, 0])
    } else {
        // yellow to red
        let s = (t - 0.5) * 2.0;
        Rgb([255, (255.0 * (1.0 - s)) as u8, 0])
    }
}

// TODO: if this connection is not in service, skip it
// TODO: switch to using binary search instead of iterating through until it's found
// TODO: (maybe) add a `max_time` option to cull entries that are too late as well
// returns a connections hash map with any entries that depart before `min_time` culled
fn get_culled_connections(
    min_time: u32,
    max_time: u32,
    connections_map: &HashMap<u32, Vec<Connection>>,
) -> HashMap<u32, Vec<Connection>> {
    println!("min_time: {}", min_time);
    println!("max_time: {}", max_time);

    let mut culled_connections_map: HashMap<u32, Vec<Connection>> = HashMap::new();

    for (_, connections) in connections_map {
        for connection in connections {
            // if departure_time already passed, skip it
            // or if arrival_time is too late, skip it
            if connection.departure_time < min_time || connection.arrival_time > max_time {
                continue;
            }

            culled_connections_map
                .entry(connection.from_stop_id)
                .or_insert_with(Vec::new)
                .push(*connection);
        }
    }

    println!("Loaded {} culled connections", culled_connections_map.len());
    return culled_connections_map;
}

// turns time in hh:mm:ss format into number of seconds since midnight
fn str_time_to_seconds(time_str: &str) -> u32 {
    let parts: Vec<&str> = time_str.split(":").collect();
    assert_eq!(parts.len(), 3);

    let hours: u32 = parts[0].parse().unwrap();
    let minutes: u32 = parts[1].parse().unwrap();
    let seconds: u32 = parts[2].parse().unwrap();

    hours * 3600 + minutes * 60 + seconds
}

// parses YYYYMMDD date string into Date struct
fn parse_date(date_str: &str) -> Date {
    Date {
        year: date_str[0..4].parse().unwrap(),
        month: date_str[4..6].parse().unwrap(),
        day: date_str[6..8].parse().unwrap(),
    }
}

// inverse of `str_time_to_seconds()`
fn seconds_to_str_time(time: &u32) -> String {
    let hours = time / 3600;
    let minutes = (time % 3600) / 60;
    let seconds = time % 60;
    return format!("{:02}:{:02}:{:02}", hours, minutes, seconds);
}

// converts route_type integer to RouteType enum
fn parse_route_type(route_type: u32) -> RouteType {
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
        _ => RouteType::BUS, // default to bus for unknown types
    }
}

// parses stop_id, handling both "600737" and "stoparea:600737" formats
fn parse_stop_id(stop_id_str: &str) -> u32 {
    if let Some(pos) = stop_id_str.rfind(':') {
        stop_id_str[pos + 1..].parse().unwrap()
    } else {
        stop_id_str.parse().unwrap()
    }
}

// gets the maximum travel time based on walkign speed and maximum distance consts
fn get_walk_time(from_position: Position, to_position: Position) -> u32 {
    let speed_mps = (WALKING_SPEED * 1000.0) / 3600.0;
    return ((haversine_distance(from_position, to_position)) / speed_mps) as u32;
}

const EARTH_RADIUS_METER: f64 = 6371000.0;
fn haversine_distance(position_a: Position, position_b: Position) -> f64 {
    let φ1: f64 = position_a.lat.to_radians();
    let φ2: f64 = position_b.lat.to_radians();
    let δφ: f64 = (position_b.lat - position_a.lat).to_radians();
    let δλ: f64 = (position_b.lon - position_a.lon).to_radians();

    let a: f64 = (δφ / 2.0).sin() * (δφ / 2.0).sin()
        + φ1.cos() * φ2.cos() * (δλ / 2.0).sin() * (δλ / 2.0).sin();
    let c: f64 = 2.0 * (a.sqrt()).asin();

    return EARTH_RADIUS_METER * c;
}
