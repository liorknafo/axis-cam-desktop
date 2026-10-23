"""A test part for AXIS CAM threading: a 70 x 40 x 12 mm plate with modeled ISO threads.
- 2x M6x1 and 1x M8x1.25 tapped through, 1x M5x0.8 blind (thread 8 deep in an 11 deep Ø4.2 hole),
- 2x plain Ø6.6 clearance holes (no thread),
written as STEP, STL and 3MF to the folder given (needs build123d and bd_warehouse)."""
import os
import sys

from build123d import Align, Box, Cylinder, Mesher, Pos, export_step, export_stl
from bd_warehouse.thread import IsoThread

out = sys.argv[1]
MIN = (Align.CENTER, Align.CENTER, Align.MIN)
T = 12.0
# x, y, major diameter, pitch, thread depth, tap-drill depth (from the top face)
HOLES = [(-25, -10, 6, 1.0, T, T), (-25, 10, 6, 1.0, T, T), (0, 0, 8, 1.25, T, T), (25, 10, 5, 0.8, 8, 11)]

part = Box(70, 40, T, align=MIN)
# Every hole first, then the thread ridges: nothing more is cut once a ridge is in.
for x, y, d, pitch, depth, drill_depth in HOLES:
    # The tap-drill hole, opened to the major diameter where the thread runs.
    part = part - Pos(x, y, T - drill_depth) * Cylinder((d - pitch) / 2, drill_depth, align=MIN)
    part = part - Pos(x, y, T - depth) * Cylinder(d / 2, depth, align=MIN)
for x, y in ((25, -10), (10, -12)):
    part = part - Pos(x, y, 0) * Cylinder(3.3, T, align=MIN)
for x, y, d, pitch, depth, _ in HOLES:
    ridge = IsoThread(major_diameter=d, pitch=pitch, length=depth, external=False, end_finishes=("fade", "fade"))
    # One turn at a time, each moved on its own; a piece whose union fails (bd_warehouse's odd partial turn) is left
    # out rather than losing the whole part.
    for piece in ridge.solids():
        fused = part + Pos(x, y, T - depth) * piece
        if fused.volume > 0.5 * part.volume:
            part = fused
# The faded thread ends reach a little past their length, out of the plate: trim them off.
trimmed = part & Box(70, 40, T, align=MIN)
if trimmed.volume > 0.5 * part.volume:
    part = trimmed
print(type(part).__name__, len(part.solids()), part.bounding_box().size, round(part.volume, 1))
export_step(part, os.path.join(out, "threaded-plate.step"))
export_stl(part, os.path.join(out, "threaded-plate.stl"), tolerance=0.02, angular_tolerance=0.3)
m = Mesher()
m.add_shape(part, linear_deflection=0.02, angular_deflection=0.3)
m.write(os.path.join(out, "threaded-plate.3mf"))
print("ok")
