# HeatMapping Ideas - Jump Fill


### The Idea
One huge optimization that could be done with the generation of the heatmap images would be using a modified jump fill algorithm.

A traditional jump fill is a very optimal way of approximating voronoi diagrams. Given a set of "seeds" (colored pixels placed at known points within the image), the algorithm will fill the image with the colors of the seeds, where each pixel is colored the same as its nearest seed.

We can use the jump fill algorithm by treating each stop as a seed, computing the region around each stop that's fastest path uses that stop.


### My Explanation of how Traditional Jump Fill Works
**Setup**

- You start with an N by N image (pixels, obviously).
- You have some datastructure that stores the position and color of each seed.
- The position of each seed should correlate exactly to some pixel in the image.


**Algorithm**

The algorithm described below doesn't handle conflicts, and thus is not really GPU compatible, but it describes the idea of how the fill works.
```
for each step size (k) in [N/2, N/4, ..., 1]:
    iterate over each pixel (p) at position (x, y):
        for each neighbor (q) of p at position (x + i, y + j), where (i, j) is any pair choice in [-k, 0, k]:
            if p is not colored, and q is colored:
                color p the same as q
            if p is colored and q is not colored:
                color q the same as p
```

**Visualization**

· · · · · · · ·      · · · · · · · ·      · · · · · · · ·      & & & & & & & &
· · · · · · · ·      · · · · · · · ·      & · & · & · & ·      & & & & & & & &
· · · · · · · ·      · · · · · · · ·      · · · · · · · ·      & & & & & & & &
· · · · · · · ·      & · · · & · · ·      & · & · & · & ·      & & & & & & & &
· · · · · · · ·      · · · · · · · ·      · · · · · · · ·      & & & & & & & &
· · · · · · · ·      · · · · · · · ·      & · & · & · & ·      & & & & & & & &
· · · · · · · ·      · · · · · · · ·      · · · · · · · ·      & & & & & & & &
& · · · · · · ·      & · · · & · · ·      & · & · & · & ·      & & & & & & & &


### The Challenges
- How will conflicts be handled such that the algorithm can be done in massive parallel on the GPU?
- The traditional jump fill algorithm simply finds the nearest seed to any given pixel. This is not the correct behavior for the heat map.
    - The heat map needs to acount for the arrival time at each stop. So rather than simply comparing distances from pixels to seeds, it will need to compare the distance plus the arrival time at that seed.
- Stops that are not within the viewport still exist, and should be acounted for.
    - The traditional jump fill algorithm assumes that each seed is within the image, but it's totally possible that the fastest path to a pixel in the viewport is from a stop that's just outside the viewport.
- The viewport won't be square, jump fill assumes a square image.
