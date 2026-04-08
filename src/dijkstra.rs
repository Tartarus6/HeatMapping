//! This file contains all of the implementations related to the dijkstra algorithm

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
};

use crate::{
    structs::{DepartInstant, GTFSData},
    utils::get_walk_time,
};

// TODO: is there a way to reuse the data from other dijkstra runs, rather than having to totally recalculate for each different starting position?
// TODO: (maybe) optimize by finding "hub nodes", and precomputing the travel times between them. then using that hub-to-hub time as an offset to prevent the need to calculate paths across hubs
// TODO: move dijkstra calculations into a shader
/// runs the dijkstra algorithm with each stop as a node, with "connections" and "transfers" as the edges
/// returns HashMap<to_stop_id: u32, arrival_time: u32> (arrival time in secons since midnight)
pub fn initialize_dijkstra(
    gtfs_data: &GTFSData,
    depart_instant: &DepartInstant,
) -> Result<HashMap<u32, u32>, Box<dyn std::error::Error>> {
    let mut arrival_times: HashMap<u32, u32> = HashMap::new(); // <to_stop_id, arrival_time>

    let mut priority_queue: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new(); // Min-heap (priority queue) storing pairs of (arrival_time, stop_id)

    // initialize priority queue and arrival times with the time it would take to walk there from the starting position
    for stop_id in gtfs_data.grid.get_nearby(depart_instant.position) {
        let stop = gtfs_data
            .stops
            .get(&stop_id)
            .ok_or("walking to stop didnt exist")?;
        let arrival_time =
            depart_instant.time + get_walk_time(depart_instant.position, stop.position);
        arrival_times.insert(stop_id, arrival_time);
        priority_queue.push(Reverse((arrival_time, stop_id))); // push the starting stop onto the priority queue
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
        // default to empty array if no connections
        for connection in gtfs_data
            .connections
            .get(&current_stop_id)
            .unwrap_or(&vec![])
        {
            // TODO: switch to using binary search instead of iterating through until it's found

            // if departure_time already passed, skip it
            if connection.departure_time < current_stop_arrival_time {
                continue;
            }

            // if new faster path found, update that travel time and add that node onto the priority queue
            if connection.arrival_time
                < *arrival_times
                    .get(&connection.to_stop_id)
                    // default to high value if arrival time not yet initialized (so that it can be overridden)
                    .unwrap_or(&u32::MAX)
            {
                arrival_times.insert(connection.to_stop_id, connection.arrival_time);
                priority_queue.push(Reverse((connection.arrival_time, connection.to_stop_id)));
            }
        }

        // explore all transfer connections of the current stop
        // default to empty array if no transfers
        for transfer in gtfs_data.transfers.get(&current_stop_id).unwrap_or(&vec![]) {
            // if new faster path found, update that travel time and add that node onto the priority queue
            if transfer.min_transfer_time + current_stop_arrival_time
                < *arrival_times
                    .get(&transfer.to_stop_id)
                    // default to high value if arrival time not yet initialized (so that it can be overridden)
                    .unwrap_or(&u32::MAX)
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
