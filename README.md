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
    - Ability to set starting position
    - Ability to set departure time
    - Ability to set destination position (in *Trip Mode*)
    - Display of heat map
- Engine
    - Calculating heat map data
    - Calculating trip mode data
    - Methods of transit dealt with are walking, trains, metro, tram, and busses
    - Ability to toggle transportation modes (i.e. disabling busses)
    - Walking is handles "as the crow flies" (constant walking speed, walking does not care about obstacles such as buildings or rivers)
    - Transit timetables need to be accounted for (wait times and such)
- UX/Responsiveness
    - Heat map updating in real time (target 60fps)
    - (maybe) *Trip Mode* path also updating in real time
