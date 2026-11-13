"""A test part for AXIS CAM threading (LIO-448): a 30 x 30 x 18.5 mm block with one M8x1.25 thread through it,
modelled as the spindle adapter plate's four are: the hole opened 0.05 mm past the thread's major radius and the
ISO thread ridge joined into it one turn at a time, so each root is relieved to Ø8.1 in a notch P/8 wide.
Written as STEP to the folder given (needs build123d and bd_warehouse)."""
import os
import sys

from build123d import Align, Box, Cylinder, Pos, export_step
from bd_warehouse.thread import IsoThread

out = sys.argv[1]
MIN = (Align.CENTER, Align.CENTER, Align.MIN)
T, MAJOR, PITCH = 18.5, 8.0, 1.25

part = Box(30, 30, T, align=MIN) - Cylinder(MAJOR / 2 + 0.05, T, align=MIN)
# Faded ends run out just inside both faces, so each mouth is round.
ridge = IsoThread(major_diameter=MAJOR, pitch=PITCH, length=T, external=False, end_finishes=("fade", "fade"))
for piece in ridge.solids():
    fused = part + piece
    assert fused.volume > part.volume + 0.5 and len(fused.solids()) == 1, "a thread turn failed to join"
    part = fused
print(type(part).__name__, part.bounding_box().size, round(part.volume, 1))
export_step(part, os.path.join(out, "m8-tapped-block.step"))
print("ok")
