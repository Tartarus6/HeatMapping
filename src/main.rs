use csv::Reader;
// use image::{ImageBuffer, Rgb};
use pollster::FutureExt;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;
use std::u32;

mod shader;

/// controls the size of the heatmap output, the aspect ratio changes based on bounding box, but this controls the longest side
const MAX_DIM: u32 = 500;
/// walking speed in kilometers per hour
const WALKING_SPEED: f64 = 5.0;
/// maximum distance to walk between stops (used for culling) (this option can be too greedy, it can cull optimal paths) (distance in meters)
const MAX_WALK_TRANSFER_DISTANCE: f64 = 500.0;

// bounding box for the heatmap output (Amsterdam-ish area)
const BBOX_MIN: Position = Position {
    lat: 0.909875098007,
    lon: 0.080410372965,
};
const BBOX_MAX: Position = Position {
    lat: 0.914647159795,
    lon: 0.088096506285,
};

/// constants for where/when we are starting from
const DEPART_INSTANT: DepartInstant = DepartInstant {
    position: Position {
        lat: 0.913998595445,
        lon: 0.085599725524,
    },
    time: 32400, // 09:00:00
    date: Date {
        year: 2026,
        month: 03,
        day: 13,
    },
};

const OUTPUT_DIRECTORY: &str = "./output/";
const CACHE_DIRECTORY: &str = "./cache/";
const GTFS_DIRECTORY: &str = "./src/lib/gtfs-nl/";

struct DepartInstant {
    position: Position,
    /// seconds since midnight
    time: u32,
    date: Date,
}

#[derive(Serialize, Deserialize)]
struct SpatialGrid {
    /// <(lat_index, lon_index), list of stop_ids>
    map: HashMap<(i32, i32), Vec<u32>>,
    cell_size: f64,
}

impl SpatialGrid {
    fn new(cell_size_meters: f64) -> Self {
        Self {
            map: HashMap::new(),
            cell_size: cell_size_meters / 6_371_000.0, // convert radians to meters (further handling is needed for latitude)
        }
    }

    fn insert(&mut self, position: Position, stop_id: u32) {
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
                        .unwrap_or(&vec![]),
                );
            }
        }
        return nearby;
    }
}

/// represents a position on the earth (latitude, longitude)
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Position {
    /// latitude in radians
    lat: f64,
    /// longitude in radians
    lon: f64,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Stop {
    stop_id: u32,
    // name: String,
    position: Position,
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
#[derive(Serialize, Deserialize)]
struct Route {
    route_id: u32,
    route_type: RouteType,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct Trip {
    trip_id: u32,
    route_id: u32,
    service_id: u32,
    stop_times: Vec<StopTime>, // TODO: fix the duplication of stop_times (its A LOT of data)
}

#[derive(Serialize, Deserialize)]
struct StopTime {
    trip_id: u32,
    stop_sequence: u16,
    stop_id: u32,
    /// seconds since midnight (note, can sometimes be greater than 24 hours worth)
    arrival_time: u32,
    /// seconds since midnight (note, can sometimes be greater than 24 hours worth)
    departure_time: u32,
}

#[derive(Eq, Hash, PartialEq, Serialize, Deserialize)]
struct Date {
    year: u32,
    month: u8,
    day: u8,
}

#[derive(Serialize, Deserialize)]
struct Transfer {
    from_stop_id: u32,
    to_stop_id: u32,
    /// (in seconds)
    min_transfer_time: u32,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Connection {
    from_stop_id: u32,
    to_stop_id: u32,
    /// id of parent trip
    trip_id: u32,
    /// time when arriving at destination (neighbor) stop (in seconds since midnight)
    arrival_time: u32,
    /// time when departing towards (neighbor) stop (in seconds since midnight)
    departure_time: u32,
}

#[derive(PartialEq, Serialize, Deserialize)]
enum ServiceExceptionType {
    ServiceAdded,
    ServiceRemoved,
}

#[derive(Serialize, Deserialize)]
struct GTFSData {
    /// <stop_id, Stop>
    stops: HashMap<u32, Stop>,
    /// SpacialGrid of stop_ids
    grid: SpatialGrid,
    /// <route_id, Route>
    routes: HashMap<u32, Route>,
    /// <trip_id, Trip>
    trips: HashMap<u32, Trip>,
    /// <(service_id, Date), exception_type>
    services: HashMap<(u32, Date), ServiceExceptionType>,
    /// <from_stop_id, list of Transfers from stop>
    transfers: HashMap<u32, Vec<Transfer>>,
    /// <from_stop_id, list of Connections>
    connections: HashMap<u32, Vec<Connection>>,
}

fn main() {
    let now = Instant::now();

    // Loading gtfs data
    let gtfs_data = match load_gtfs_data("cache/gtfs_data") {
        // try loading from cache if possible
        Ok(data) => data,
        Err(_) => {
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
    println!("Initializing: {}ms", now.elapsed().as_millis());

    let now = Instant::now();
    let travel_times = initialize_dijkstra(&gtfs_data).unwrap();
    println!("Dijkstra: {}ms", now.elapsed().as_millis());

    println!(
        "depart instant time: {}",
        seconds_to_str_time(&DEPART_INSTANT.time)
    );
    let now = Instant::now();
    generate_heatmap(
        &gtfs_data,
        &travel_times,
        format!("{OUTPUT_DIRECTORY}{}", "heatmap.png").as_str(),
    );
    println!("Heatmap: {}ms", now.elapsed().as_millis());
    println!("Heatmap saved to heatmap.png");
}

// TODO: switch all latitudes and longitudes to use radians (for consistency)
// TODO: make the data smaller where possible (removing unimportant data, maybe making a separate database not in-memory)
/// reads the gtfs data from the gtfs files and puts them into a GTFSData struct instance
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
    let file = File::open(format!("{GTFS_DIRECTORY}{}", "stops.txt"))?;
    let mut reader = Reader::from_reader(file);
    for result in reader.records() {
        let record = result?;
        let stop_id = parse_stop_id(&record[0]);
        let position = Position {
            lat: record[3].parse::<f64>()?.to_radians(),
            lon: record[4].parse::<f64>()?.to_radians(),
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
    let file = File::open(format!("{GTFS_DIRECTORY}{}", "routes.txt"))?;
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
    let file = File::open(format!("{GTFS_DIRECTORY}{}", "trips.txt"))?;
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
    let file = File::open(format!("{GTFS_DIRECTORY}{}", "stop_times.txt"))?;
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
    let file = File::open(format!("{GTFS_DIRECTORY}{}", "calendar_dates.txt"))?;
    let mut reader = Reader::from_reader(file);
    for result in reader.records() {
        let record = result?;
        let service_id: u32 = record[0].parse().unwrap();
        let date = parse_date(&record[1]);
        let exception_type: u32 = record[2].parse().unwrap_or(0); // default to 0, which lets `parse_exception_type()` decide the default

        gtfs_data
            .services
            .insert((service_id, date), parse_exception_type(exception_type));
    }
    println!("Loaded {} services", gtfs_data.services.len());

    // transfers
    let file = File::open(format!("{GTFS_DIRECTORY}{}", "transfers.txt"))?;
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
    // walking transfers (just stored as transfers)
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
    let mut connection_count: u32 = 0;
    for (_, trip) in gtfs_data.trips.iter_mut() {
        // sort stop times to be in order
        trip.stop_times
            .sort_by(|a, b| a.departure_time.cmp(&b.departure_time));

        // skipping last index since we are looking at pairs of stops
        for i in 0..trip.stop_times.len() - 1 {
            let from_stop_id = trip.stop_times[i].stop_id;

            connection_count += 1;

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

    println!("Loaded {} connections", connection_count);

    Ok(gtfs_data)
}

// TODO: is there a way to reuse the data from other dijkstra runs, rather than having to totally recalculate for each different starting position?
// TODO: (maybe) optimize by finding "hub nodes", and precomputing the travel times between them. then using that hub-to-hub time as an offset to prevent the need to calculate paths across hubs
/// runs the dijkstra algorithm with each stop as a node, with "connections" and "transfers" as the edges
/// returns HashMap<to_stop_id: u32, arrival_time: u32> (arrival time in secons since midnight)
fn initialize_dijkstra(
    gtfs_data: &GTFSData,
) -> Result<HashMap<u32, u32>, Box<dyn std::error::Error>> {
    let mut arrival_times: HashMap<u32, u32> = HashMap::new(); // <to_stop_id, arrival_time>

    // get culled connections list, removing any entries that occured before the depart instant (max time not used, so set to u32::MAX)
    let culled_connections = get_culled_connections(
        DEPART_INSTANT.time,
        u32::MAX,
        &gtfs_data.connections,
        &gtfs_data,
    );

    let mut priority_queue: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new(); // Min-heap (priority queue) storing pairs of (arrival_time, stop_id)

    // initialize priority queue and arrival times with the time it would take to walk there from the starting position
    for stop_id in gtfs_data.grid.get_nearby(DEPART_INSTANT.position) {
        let stop = gtfs_data
            .stops
            .get(&stop_id)
            .ok_or("walking to stop didnt exist")?;
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
    }

    Ok(arrival_times)
}

// TODO: split the map into regions that share a common best path, within region just apply a gradient rather than having to recalculate optimal path for each pixel
// TODO: make this generate some kinda vector graphic maybe
/// generates an image displaying the time it takes to get from the starting position to any other position (within the heatmap) as a color gradient
/// uses the parsed gtfs data and the travel times from the dijkstra algorithm
fn generate_heatmap(gtfs_data: &GTFSData, arrival_times: &HashMap<u32, u32>, output_path: &str) {
    // derive image dimensions from the bounding box aspect ratio
    // longitude degrees are physically shorter at higher latitudes, scale by cos(mid_lat)
    let mid_lat = (BBOX_MIN.lat + BBOX_MAX.lat) / 2.0;
    let physical_width = (BBOX_MAX.lon - BBOX_MIN.lon) * mid_lat.cos();
    let physical_height = BBOX_MAX.lat - BBOX_MIN.lat;
    let aspect_ratio = physical_width / physical_height;
    let (width, height) = if aspect_ratio >= 1.0 {
        (MAX_DIM, (MAX_DIM as f64 / aspect_ratio) as u32)
    } else {
        ((MAX_DIM as f64 * aspect_ratio) as u32, MAX_DIM)
    };

    println!("width : {}", width);
    println!("height: {}", height);
    println!("aspect: {}\n", aspect_ratio);

    let mut stop_positions: Vec<[f32; 2]> = Vec::new();
    for stop in gtfs_data.stops.values() {
        stop_positions.push([stop.position.lat as f32, stop.position.lon as f32]);
    }

    shader::run(&stop_positions, width, height, output_path).block_on();

    // let mut pixel_arrival_time_map: HashMap<(u32, u32), u32> = HashMap::new(); // <(px, py), arrival_time>

    // // finding travel times for each pixel, storing in a map
    // let mut max_time: u32 = 0; // stores the latest pixel arrival_time found

    // for py in 0..height {
    //     for px in 0..width {
    //         // map pixel to lat/lon
    //         let lat = max_lat - (py as f64 / height as f64) * (max_lat - min_lat); // flip y axis
    //         let lon = min_lon + (px as f64 / width as f64) * (max_lon - min_lon);

    //         let pixel_pos = Position { lat, lon };

    //         // find best stop
    //         let nearby_stops = gtfs_data.grid.get_nearby(pixel_pos);
    //         let best_stop_id = nearby_stops.iter().min_by(|a, b| {
    //             // sort stops based on stop arrival time + walk time to pixel_pos from stop
    //             let stop_a = gtfs_data.stops.get(a).unwrap();
    //             let stop_b = gtfs_data.stops.get(b).unwrap();
    //             let da = get_walk_time(pixel_pos, stop_a.position);
    //             // + arrival_times.get(&stop_a.stop_id).unwrap();
    //             let db = get_walk_time(pixel_pos, stop_b.position);
    //             // + arrival_times.get(&stop_b.stop_id).unwrap();
    //             da.partial_cmp(&db).unwrap()
    //         });

    //         match best_stop_id.and_then(|stop_id| arrival_times.get(stop_id)) {
    //             Some(&arrival_time) => {
    //                 max_time = max_time.max(arrival_time);
    //                 pixel_arrival_time_map.insert(
    //                     (px, py),
    //                     arrival_time
    //                         + get_walk_time(
    //                             pixel_pos,
    //                             gtfs_data.stops.get(best_stop_id.unwrap()).unwrap().position,
    //                         ),
    //                 );
    //             }
    //             None => (),
    //         };
    //     }
    // }

    // // generating image from values stored in map
    // let mut img = ImageBuffer::new(width, height);
    // for ((px, py), arrival_time) in pixel_arrival_time_map {
    //     let t = (arrival_time.saturating_sub(DEPART_INSTANT.time) as f64)
    //         / (max_time - DEPART_INSTANT.time) as f64;
    //     let color = travel_time_to_color(t.clamp(0.0, 1.0));

    //     img.put_pixel(px, py, color);
    // }

    // img.save(output_path).unwrap(); // create the image file
}

/// Maps a normalized travel time (0.0 = fastest, 1.0 = slowest) to a color.
/// white -> black
// fn travel_time_to_color(t: f64) -> Rgb<u8> {
//     // white to black
//     let s = ((1.0 - t) * 255.0) as u8;
//     Rgb([s, s, s])
// }

// TODO: switch to using binary search instead of iterating through until it's found
// TODO: add ability to ignore certain transport types (i.e. only no-busses routes)
/// returns a connections hash map with any entries that depart before `min_time` culled
fn get_culled_connections(
    min_time: u32,
    max_time: u32,
    connections_map: &HashMap<u32, Vec<Connection>>,
    gtfs_data: &GTFSData,
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

            // TODO: fix service_exception_type definition to be safer (stop using `unwrap()`)
            // TODO: this service exception type check is really slow i think, gotta speed this up (i think it alone is adding 4 seconds of compute)
            let service_exception_type = gtfs_data
                .services
                .get(&(
                    gtfs_data.trips.get(&connection.trip_id).unwrap().service_id,
                    DEPART_INSTANT.date,
                ))
                .unwrap_or(&ServiceExceptionType::ServiceRemoved);

            // if connection not in service today, skip it
            if *service_exception_type != ServiceExceptionType::ServiceAdded {
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

/// turns time in hh:mm:ss format into number of seconds since midnight
fn str_time_to_seconds(time_str: &str) -> u32 {
    let parts: Vec<&str> = time_str.split(":").collect();
    assert_eq!(parts.len(), 3);

    let hours: u32 = parts[0].parse().unwrap();
    let minutes: u32 = parts[1].parse().unwrap();
    let seconds: u32 = parts[2].parse().unwrap();

    hours * 3600 + minutes * 60 + seconds
}

/// turns time in seconds since midnight into hh:mm:ss format
fn seconds_to_str_time(time: &u32) -> String {
    let hours = time / 3600;
    let minutes = (time % 3600) / 60;
    let seconds = time % 60;
    return format!("{:02}:{:02}:{:02}", hours, minutes, seconds);
}

/// parses YYYYMMDD date string into Date struct
fn parse_date(date_str: &str) -> Date {
    Date {
        year: date_str[0..4].parse().unwrap(),
        month: date_str[4..6].parse().unwrap(),
        day: date_str[6..8].parse().unwrap(),
    }
}

/// converts route_type integer to RouteType enum
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
        _ => RouteType::BUS, // TODO: switch to some other default to specify that it's unknown
                             // TODO: maybe add support for more route types, if needed
    }
}

/// converts exception_type integer to ServiceExceptionType enum
fn parse_exception_type(exception_type: u32) -> ServiceExceptionType {
    match exception_type {
        1 => ServiceExceptionType::ServiceAdded,
        2 => ServiceExceptionType::ServiceRemoved,
        _ => ServiceExceptionType::ServiceRemoved,
    }
}

/// parses stop_id, handling both "600737" and "stoparea:600737" formats
fn parse_stop_id(stop_id_str: &str) -> u32 {
    if let Some(pos) = stop_id_str.rfind(':') {
        stop_id_str[pos + 1..].parse().unwrap()
    } else {
        stop_id_str.parse().unwrap()
    }
}

/// gets the number of seconds taken to walk between 2 positions based on set walking speed
fn get_walk_time(from_position: Position, to_position: Position) -> u32 {
    let speed_mps = (WALKING_SPEED * 1000.0) / 3600.0;
    return ((haversine_distance(from_position, to_position)) / speed_mps) as u32;
}

/// gets distance in meters between 2 positions
const EARTH_RADIUS_METER: f64 = 6371000.0;
fn haversine_distance(position_a: Position, position_b: Position) -> f64 {
    let φ1: f64 = position_a.lat;
    let φ2: f64 = position_b.lat;
    let δφ: f64 = position_b.lat - position_a.lat;
    let δλ: f64 = position_b.lon - position_a.lon;

    let a: f64 = (δφ / 2.0).sin() * (δφ / 2.0).sin()
        + φ1.cos() * φ2.cos() * (δλ / 2.0).sin() * (δλ / 2.0).sin();
    let c: f64 = 2.0 * (a.sqrt()).asin();

    return EARTH_RADIUS_METER * c;
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
