# HeatMapping Docs - Jump Flood

## The Traditional Jump Flood Algorithm
A traditional jump fill is a very optimal way of approximating voronoi diagrams. Given a set of "seeds" (colored pixels placed at known points within the image), the algorithm will fill the image with the colors of the seeds, where each pixel is colored the same as its nearest seed.

We can use the jump fill algorithm by treating each stop as a seed, computing the region around each stop that's fastest path uses that stop.


## My Explanation of how the HeatMapping Jump Flood Works
The goal with the jump flood algorithm in HeatMapping is to calculate the arrival time of each pixel on the screen.

In the case of HeatMapping, each seed is a GTFS stop.

A shader buffer (let's call it `stops_buffer`) is created to hold all of the information that the JFA will need about all of the stops. The buffer is an array, where each element correspond to a stop. Each element in the array stores the *latitude* and *longitude* of that stop as well as the *arrival time* to that stop calculated by the dijkstra algorithm.

You have 2 textures: lets call them `texture_a` and `texture_b`. `texture_a` is initialized with the seed pixels, and `texture_b` is initially empty.

Both textures are in the *R32Uint* format. This means that the texture has only 1 color chanel, and that one color chanel stores a 32 bit unsigned integer for each pixel.

**The algorith has 3 main parts:**
1. Seed Scattering (initializing)
2. JFA Steps
3. Finalizing


**Seed Scattering**:

A shader is run, with a thread assigned to each element in `stops_buffer`.

For each stop:
1. The pixel correlating to that stop's latitude and longitude is calculated
2. If that pixel isn't within the viewport, we ignore that stop
3. The stop's pixel is "colored", assigning it the number coresponding to that stop's index in `stops_buffer`.

So this means that after the seed scattering, `texture_a` has little dots on it, that each corespond to a stop.


**JFA Steps**:

The "JFA Steps" stage consists of multiple shader passes that will fill in the texture. By the end of the steps, each pixel will be "colored" to the index of the best stop to get there from.

Each JFA step reads from one texture and writes to the other. So the first step reads from `texture_a`, and writes to `texture_b`, then the second step switches it to be reading from `texture_b` and writing to `texture_a`.
This makes sure that each pixel is reading the same data, and that nothing is getting overwritten.

```
for each step size (k) in [N/2, N/4, ..., 1]:
    iterate over each pixel (p) at position (x, y):        
        for dx in [-k, 0, k]:
            for dy in [-k, 0, k]:
                let q be the pixel at position (x+dx, y+dy)
                
                if q has a "color" and p doesnt:
                    color p the same as q
                if q has a "color" and p does as well:
                    if the stop that q is assigned to is a faster way to get to pixel p than the stop that p is already assigned to:
                        then color p the same as q
```

After running that, every pixel should be colored.


**Finalizing**:

Now that we have the best stop to take to get to each pixel, we need to calculate the resulting arrival time of each pixel.

For each pixel:
1. look up the element in `stops_buffer` coresponding to the value of that pixel
2. calculate the distance between that stop and the pixel
3. calculate how long it would take to walk that distance
4. add that walking time to the arrival time to that stop, and write that new value into a texture

Now we have a texture that stores the arrival time at each pixel.


## Further Improvements
What's described above isn't exactly how it's implemented in the code, several improvements were made.

- In **Seed Scattering**, instead of coloring just 1 pixel for each stop, a 3x3 of pixels is colored around each stop. This helps to reduce noise and make the output more consistent
- In order to make a texture that's actually visually useful, several changes have been put in place
  - An additional shader, **Minmax**, was added. It runs after **JFA Steps** and before **Finalizing**. It finds the earliest and latest arrival times within the texture. This helps make bounds that are used in the gradient described below.
  - *A gradient* is used to turn the pixel's arrival times into an actual color. The arrival times are normalized to be between 0.0 and 1.0, and used to lookup a color within a gradient. This is how the final output is actually made
