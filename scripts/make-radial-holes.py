"""A test part for indexed drilling on the rotary A axis: a Ø40 x 40 mm round part along X
with six blind radial Ø4 holes, 8 deep, one every 60° round X from +Y towards +Z (0°, 60°, …,
300°), each at its own X (-12.5, -7.5, …, 12.5) so that a test can tell them apart. Positive A
turns +Y towards +Z, so hole k (at 60k°) faces up at A 90 - 60k: A 90, 30, -30, -90, -150 and
-210, which are A 90, 30, 330, 270, 210 and 150.
Also radial-tube: a Ø40 x 40 mm tube along X with a Ø28 bore and six Ø4 radial holes through its
wall at X 0, 60° apart as above, so that each faces the one opposite across the bore.
And radial-tube-half: the same tube with only the holes at 0°, 60° and 120°, each facing solid wall
across the bore.
Written as STEP and 3MF to the folder given (needs cadquery)."""
import math
import os
import sys
import zipfile

import cadquery as cq

out = sys.argv[1]
R, LENGTH, HOLE, DEPTH = 20.0, 40.0, 4.0, 8.0
part = cq.Workplane("YZ").circle(R).extrude(LENGTH).translate((-LENGTH / 2, 0, 0))
for k in range(6):
    t = math.radians(60 * k)
    d = cq.Vector(0, math.cos(t), math.sin(t))
    x = -12.5 + 5 * k
    start = cq.Vector(x, 0, 0) + d * (R - DEPTH)
    part = part.cut(cq.Workplane().add(cq.Solid.makeCylinder(HOLE / 2, DEPTH + 1, start, d)))


def write(part, name):
    solid = part.val()
    box = solid.BoundingBox()
    print(name, box.xlen, box.ylen, box.zlen, round(solid.Volume(), 1))
    cq.exporters.export(part, os.path.join(out, name + ".step"))
    path = os.path.join(out, name + ".3mf")
    cq.exporters.export(part, path, tolerance=0.02, angularTolerance=0.3)
    # CadQuery numbers the mesh object 0; a 3MF resource id must be 1 or more.
    with zipfile.ZipFile(path) as z:
        files = {n: z.read(n) for n in z.namelist()}
    model = "3D/3dmodel.model"
    files[model] = files[model].replace(b'object id="0"', b'object id="2"').replace(b'objectid="0"', b'objectid="2"')
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for n, data in files.items():
            z.writestr(n, data)


write(part, "radial-holes")
BORE = 28.0


def tube_with(holes):
    tube = cq.Workplane("YZ").circle(R).circle(BORE / 2).extrude(LENGTH).translate((-LENGTH / 2, 0, 0))
    for k in holes:
        t = math.radians(60 * k)
        d = cq.Vector(0, math.cos(t), math.sin(t))
        start = d * (BORE / 2 - 1)
        tube = tube.cut(cq.Workplane().add(cq.Solid.makeCylinder(HOLE / 2, R - BORE / 2 + 2, start, d)))
    return tube


write(tube_with(range(6)), "radial-tube")
write(tube_with(range(3)), "radial-tube-half")
print("ok")
