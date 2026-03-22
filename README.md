# HeatMapping

This is a mapping engine, kind of like the bakend of google maps.

The engine is provided GTFS data of a region (such as Amsterdam), and will do all of the math behind mapping throughout that region.


## The Application
The end goal is to make a UI (probably a website) in which the starting position and departure time are given, then travel times can be shown.

The application will have 2 modes:
1. **Heatmap Mode** (the reason this project is named as it is): Only the starting position and departure time are given, then a "heat map" of travel times is shown. The time it takes to get to any point is calculated, and displayed as a color gradiet on the map.
2. **Trip Mode**: In addition to the starting position and departure time, a target destination is also chosen. In this mode, the time it takes to get to the target destination is shown as well as the path taken to get there.


## Goals/Constraints
- UI
    - Display a map of the GTFS data (as google maps would, ish)
    - Ability to set departure position
    - Ability to set departure time and date
    - Ability to set destination position (in *Trip Mode*)
    - Display of heat map
    - Display of best path (in *Trip Mode*)
- Engine
    - Calculating heat map data
    - Calculating trip mode data
    - Methods of transit dealt with are at least walking, trains, metro, tram, and busses (more might be dealt with)
    - Ability to toggle transportation modes (i.e. disabling busses)
    - Walking is handles "as the crow flies" (constant walking speed, walking does not care about obstacles such as buildings or rivers)
    - Transit timetables need to be accounted for (wait times and such)
- UX/Responsiveness
    - Heat map updating in real time (target 60fps)
    - (maybe) *Trip Mode* path also updating in real time
    - The departure position and time should be able to be changed in real time, and it should update the result in real time as well


## How to Run
In order to run it, you'll first need some GTFS data for the heatmap to be based on.

As some example data, click [this](https://gtfs.ovapi.nl/nl/gtfs-nl.zip) link to download the GTFS files for the Netherlands. Put that zip file in the `GTFS` directory of this project, and unzip it.

*Note: "GTFS data" is just a folder of specifically formatted files. Each file has the `.txt` extension, but they're all actually CSV formatted.*

The project structure should look something like this:
```
HeatMapping/
├── cache/
│   └── ...
├── GTFS/
│   ├── gtfs-nl/
│   │   ├── agency.txt
│   │   ├── calendar_dates.txt
│   │   └── ...
│   ├── example_other_gtfs_folder/
│   │   ├── agency.txt
│   │   ├── calendar_dates.txt
│   │   └── ...
│   └── ...
├── output/
│   └── ...
└── ...
```

You can put as many different GTFS data folders in the `GTFS/` directory as you'd like. They will all be parsed.

*Note: only the child directories of `GTFS/` are parsed, not the direct contents of `GTFS/`. So any GTFS files put directly in `GTFS/` will be ignored*

Once you've got the data in there, run the `cargo run --release` command to compile and execute the program. The release flag is reccomended because this is a rather intensive program, and it can take a very long time to finish without the compiler optimizations.

Once the GTFS data is parsed for the first time, a cache file called `gtfs_data` is created and placed in `cache/`. This contains the raw parsed data. In order to speed up the program, if a cache file is present, it is used instead of re-parsing the GTFS data.

*Note: If changes are made to the way that GTFS data is parsed or if the GTFS data being parsed has changed: In order for the difference to apply you must delete the `gtfs_data` cache file.*


## Main Program Pipeline
1. **Parse GTFS Data**
  - Or load cache if present
2. **Run Dijkstra Algorithm**
  - Takes departure position, time, and date
  - Finds the earliest arrival time to each stop in the GTFS data
3. **Run Heatmap Algorithm**
  - Uses arrival time at each station
  - Runs a modified jump flood algorithm (JFA)
  - Detailed explanation [here](docs/JUMP_FLOOD_HEATMAP)
