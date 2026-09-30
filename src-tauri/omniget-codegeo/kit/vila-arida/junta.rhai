// Adobe buttress at the seam between two houses.
let b = wedge([0.5, 2.8, 0.6], #{ at: [0.0, 0.0, -0.3], mat: "wall", rot: 180 });
if lod() > 0 { b } else { union([b, drainpipe(0.3, -0.1)]) }
