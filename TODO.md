# TODO

- [x] Make shder.rs not horrible
- [ ] Greatly reduce memory usage with large datasets
- [x] Fix the issue where seeds overwrite each other in the seed_scatter stage, leading to race conditions
- [ ] improve position to pixel mapping, so that it adjusts to fit best to the bounding box rather than stretching at the poles
- [ ] limit bounding box to either stop at poles or wrap
- [ ] (maybe) switch to using integers for lat and lon, deciding some formula to convert between the actual numbers and the integers. make the range bound to the possible latitudes and longitudes. this should help improve accuracy (since all values map to some possible position rather than being focused around zero)
- [ ] Fix weird artifact when zooming (zoom way out, at a specific zoom level the heat map stretches horizontally, then goes back to normal)
- [ ] Fix aspect ratio breaking due to floating point precision issues when zooming way way in (zoom way way in, and the whole view will be squished or stretched even after zooming back out)
- [ ] Make dijkstra into a shader
  - [ ] Dijkstra updates in real-time
- [ ] Ability to toggle transit types (like disabling busses)
