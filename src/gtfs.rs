//! This file contains all of the implementations related to parsing the gtfs files

use csv::{Reader, StringRecord};
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::{collections::HashMap, path::PathBuf};

use crate::CACHE_DIRECTORY;
use crate::utils::str_to_u32_hash;
use crate::{
    GTFS_DIRECTORY, MAX_WALK_TRANSFER_DISTANCE,
    structs::{
        Connection, Date, GTFSData, Position, Route, RouteType, ServiceExceptionType, SpatialGrid,
        Stop, StopTime, Transfer, Trip, parse_route_id, parse_stop_id,
    },
    utils::{get_walk_time, str_time_to_seconds},
};

/// Gets the gtfs data for use by the program.
/// If a cache is present, it'll just use the cache,
/// otherwise, the GTFS files are parsed.
pub fn get_gtfs_data() -> GTFSData {
    let gtfs_data = match load_gtfs_data("cache/gtfs_data") {
        // try loading from cache if possible
        Ok(data) => data,
        Err(_) => {
            println!("Cache not found - parsing GTFS data...");

            // if couldnt load gtfs data from cache, parse from gtfs files
            let data = match parse_data() {
                Ok(data) => data,
                Err(err) => panic!("error parsing gtfs data: {:?}", err),
            };

            // save that parsed data into the cache
            match save_gtfs_data(
                &data,
                format!("{}{}", CACHE_DIRECTORY, "gtfs_data").as_str(),
            ) {
                Ok(()) => (),
                Err(err) => panic!("error saving gtfs data: {:?}", err),
            };

            data // return that data
        }
    };

    return gtfs_data;
}

/// Save the parsed GTFS data into a file.
/// This uses the postcard format.
fn save_gtfs_data(data: &GTFSData, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = postcard::to_allocvec(data)?;
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

/// Load previously parsed GTFS data from a file.
/// This uses the postcard format.
fn load_gtfs_data(path: &str) -> Result<GTFSData, Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    let data = postcard::from_bytes(&buffer)?;

    Ok(data)
}

// TODO: make the data smaller where possible (removing unimportant data, maybe making a separate database not in-memory)
/// Reads the gtfs files, parsing them into a GTFSData struct instance
fn parse_data() -> Result<GTFSData, Box<dyn std::error::Error>> {
    let mut gtfs_data = GTFSData {
        stops: HashMap::new(),
        grid: SpatialGrid::new(MAX_WALK_TRANSFER_DISTANCE),
        routes: HashMap::new(),
        trips: HashMap::new(),
        services: HashMap::new(),
        transfers: HashMap::new(),
        connections: HashMap::new(),
    };

    // for each gtfs folder
    for directory in get_child_dirs(GTFS_DIRECTORY)? {
        println!("GTFS Directory: {:?}", directory);

        // stops
        let file = File::open(directory.join("stops.txt"))?;
        let mut reader = Reader::from_reader(file);

        let headers = reader.headers()?.clone();
        let idx = header_index(&headers);

        for result in reader.records() {
            let record = result?;

            let stop_id = parse_stop_id(require(&record, &idx, "stop_id")?)?;

            let position = Position {
                lat: require(&record, &idx, "stop_lat")?
                    .parse::<f32>()?
                    .to_radians(),
                lon: require(&record, &idx, "stop_lon")?
                    .parse::<f32>()?
                    .to_radians(),
            };

            gtfs_data.stops.insert(stop_id, Stop { stop_id, position });
            gtfs_data.grid.insert(position, stop_id);
        }
        println!("Loaded {} stops", gtfs_data.stops.len());

        // routes
        let file = File::open(directory.join("routes.txt"))?;
        let mut reader = Reader::from_reader(file);

        let headers = reader.headers()?.clone();
        let idx = header_index(&headers);

        for result in reader.records() {
            let record = result?;

            let route_id: u32 = parse_route_id(require(&record, &idx, "route_id")?)?;

            gtfs_data.routes.insert(
                route_id,
                Route {
                    route_id,
                    route_type: RouteType::parse_route_type(
                        require(&record, &idx, "route_type")?.parse()?,
                    ),
                    name: get(&record, &idx, "route_long_name")
                        .or_else(|| get(&record, &idx, "route_short_name"))
                        .unwrap_or("")
                        .to_string(),
                },
            );
        }
        println!("Loaded {} routes", gtfs_data.routes.len());

        // trips
        let file = File::open(directory.join("trips.txt"))?;
        let mut reader = Reader::from_reader(file);

        let headers = reader.headers()?.clone();
        let idx = header_index(&headers);

        for result in reader.records() {
            let record = result?;

            let trip_id: u32 = str_to_u32_hash(require(&record, &idx, "trip_id")?);

            gtfs_data.trips.insert(
                trip_id,
                Trip {
                    trip_id,
                    route_id: parse_route_id(require(&record, &idx, "route_id")?)?,
                    service_id: str_to_u32_hash(require(&record, &idx, "service_id")?),
                    stop_times: vec![],
                },
            );
        }
        println!("Loaded {} trips", gtfs_data.trips.len());

        // stop_times
        let file = File::open(directory.join("stop_times.txt"))?;
        let mut reader = Reader::from_reader(file);

        let headers = reader.headers()?.clone();
        let idx = header_index(&headers);

        let mut stop_times_count: u32 = 0;
        for result in reader.records() {
            let record = result?;
            let trip_id: u32 = str_to_u32_hash(require(&record, &idx, "trip_id")?);

            let trip = gtfs_data
                .trips
                .get_mut(&trip_id)
                .ok_or("stop time trip didn't exist")?;

            trip.stop_times.push(StopTime {
                trip_id,
                stop_id: parse_stop_id(require(&record, &idx, "stop_id")?)?,
                arrival_time: str_time_to_seconds(require(&record, &idx, "arrival_time")?)?,
                departure_time: str_time_to_seconds(require(&record, &idx, "departure_time")?)?,
            });

            stop_times_count += 1;
        }
        println!("Loaded {} stop_times", stop_times_count);

        // services (from calendar_dates)
        let file = File::open(directory.join("calendar_dates.txt"))?;
        let mut reader = Reader::from_reader(file);

        let headers = reader.headers()?.clone();
        let idx = header_index(&headers);

        for result in reader.records() {
            let record = result?;

            let service_id: u32 = str_to_u32_hash(require(&record, &idx, "service_id")?);
            let date = Date::parse_date_string(require(&record, &idx, "date")?)?;
            let exception_type = ServiceExceptionType::parse_exception_type(
                require(&record, &idx, "exception_type")?
                    .parse()
                    .unwrap_or(0),
            ); // default to 0, which lets `parse_exception_type()` decide the default

            gtfs_data
                .services
                .insert((service_id, date), exception_type);
        }
        println!("Loaded {} services", gtfs_data.services.len());

        // transfers
        let file = File::open(directory.join("transfers.txt"))?;
        let mut reader = Reader::from_reader(file);

        let headers = reader.headers()?.clone();
        let idx = header_index(&headers);

        for result in reader.records() {
            let record = result?;

            let from_stop_id: u32 = parse_stop_id(require(&record, &idx, "from_stop_id")?)?;

            gtfs_data
                .transfers
                .entry(from_stop_id)
                .or_insert_with(Vec::new)
                .push(Transfer {
                    from_stop_id: from_stop_id,
                    to_stop_id: parse_stop_id(require(&record, &idx, "to_stop_id")?)?,
                    // TODO: is the GTFS standard format for min_transfer_time in seconds already, or does it need to be converted?
                    min_transfer_time: require(&record, &idx, "min_transfer_time")
                        .unwrap_or_default()
                        .parse()
                        .unwrap_or(0), // default to 0 if not declared
                });
        }

        println!();
    }

    // Transfers Post-Parse
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

    // Connections Post-Parse
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

fn header_index(headers: &StringRecord) -> HashMap<&str, usize> {
    headers.iter().enumerate().map(|(i, h)| (h, i)).collect()
}

fn get<'a>(rec: &'a StringRecord, idx: &HashMap<&str, usize>, name: &str) -> Option<&'a str> {
    idx.get(name).and_then(|&i| rec.get(i))
}

/// Like `get()`, but errors if missing
fn require<'a>(
    rec: &'a StringRecord,
    idx: &HashMap<&str, usize>,
    name: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    get(rec, idx, name).ok_or_else(|| format!("missing required column: {name}").into())
}

/// Get the child directories of a directory
fn get_child_dirs(root: impl AsRef<Path>) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut dirs = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }

    Ok(dirs)
}
