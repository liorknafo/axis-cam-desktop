#!/usr/bin/env python3
"""Writes examples/dowel-block.3mf: a 120 x 80 x 20 mm block, centred on X0
Y0 with its bottom at Z0, with two Ø4.1 dowel holes 8 mm deep in its top
face at X-40 Y25 and X40 Y25 (64-sided), the holes its design has for
fixture pins Ø4 80 mm apart. Watertight: every face is fanned over all the
points on its edges."""
import math
import sys
import zipfile

X, Y, Z = 60.0, 40.0, 20.0
HOLES = [(-40.0, 25.0), (40.0, 25.0)]
R, DEPTH, SIDES, HALF = 2.05, 8.0, 64, 5.0
XS = [-X, -45.0, -35.0, 35.0, 45.0, X]
YS = [-Y, 20.0, 30.0, Y]

tris = []


def r6(v):
    return round(v, 6)


def fan(points, normal):
    """A convex polygon's points in any order, fanned in the order that
    faces `normal`: a quad corner to corner, more points from the centre."""
    c = [sum(p[k] for p in points) / len(points) for k in range(3)]
    # Two axes in the polygon's plane.
    u = (1.0, 0.0, 0.0) if abs(normal[0]) < 0.9 else (0.0, 1.0, 0.0)
    v = (
        normal[1] * u[2] - normal[2] * u[1],
        normal[2] * u[0] - normal[0] * u[2],
        normal[0] * u[1] - normal[1] * u[0],
    )
    u = (
        v[1] * normal[2] - v[2] * normal[1],
        v[2] * normal[0] - v[0] * normal[2],
        v[0] * normal[1] - v[1] * normal[0],
    )
    def angle(p):
        d = [p[k] - c[k] for k in range(3)]
        return math.atan2(sum(d[k] * v[k] for k in range(3)), sum(d[k] * u[k] for k in range(3)))
    ring = sorted(set(points), key=angle)
    if len(ring) == 4:
        apexes = [(ring[0], ring[i], ring[i + 1]) for i in (1, 2)]
    else:
        # Fanned from its centre, so that no edge point is left out.
        centre = tuple(r6(x) for x in c)
        apexes = [(centre, ring[i], ring[(i + 1) % len(ring)]) for i in range(len(ring))]
    for a, b, d in apexes:
        e1 = [b[k] - a[k] for k in range(3)]
        e2 = [d[k] - a[k] for k in range(3)]
        n = (
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        )
        if math.sqrt(sum(x * x for x in n)) < 1e-9:
            continue
        tris.append((a, b, d) if sum(n[k] * normal[k] for k in range(3)) > 0 else (a, d, b))


def ring(cx, cy, r, z):
    return [
        (r6(cx + r * math.cos(2 * math.pi * i / SIDES)), r6(cy + r * math.sin(2 * math.pi * i / SIDES)), z)
        for i in range(SIDES)
    ]


def square(cx, cy, z):
    """Where each ray from the hole's centre to a ring point meets its
    square cell: the cell's corners are among them (64 sides)."""
    out = []
    for i in range(SIDES):
        a = 2 * math.pi * i / SIDES
        m = max(abs(math.cos(a)), abs(math.sin(a)))
        out.append((r6(cx + HALF * math.cos(a) / m), r6(cy + HALF * math.sin(a) / m), z))
    return out


squares = [square(cx, cy, Z) for cx, cy in HOLES]
edge_points = [p for s in squares for p in s]

# The top face: grid cells, a hole's cell an annulus to its ring.
for i in range(len(XS) - 1):
    for j in range(len(YS) - 1):
        x0, x1, y0, y1 = XS[i], XS[i + 1], YS[j], YS[j + 1]
        hole = next(
            (k for k, (cx, cy) in enumerate(HOLES) if x0 < cx < x1 and y0 < cy < y1), None
        )
        if hole is not None:
            cx, cy = HOLES[hole]
            inner, outer = ring(cx, cy, R, Z), squares[hole]
            for k in range(SIDES):
                n = (k + 1) % SIDES
                fan([inner[k], outer[k], outer[n], inner[n]], (0, 0, 1))
            # The wall, down to the floor, and the floor.
            low = ring(cx, cy, R, Z - DEPTH)
            for k in range(SIDES):
                n = (k + 1) % SIDES
                a, b, c, d = inner[k], inner[n], low[n], low[k]
                tris.append((a, b, c))
                tris.append((a, c, d))
            centre = (cx, cy, Z - DEPTH)
            for k in range(SIDES):
                tris.append((centre, low[k], low[(k + 1) % SIDES]))
            continue
        points = [(x0, y0, Z), (x1, y0, Z), (x1, y1, Z), (x0, y1, Z)]
        points += [
            p
            for p in edge_points
            if (x0 - 1e-6 <= p[0] <= x1 + 1e-6 and y0 - 1e-6 <= p[1] <= y1 + 1e-6)
            and (abs(p[0] - x0) < 1e-6 or abs(p[0] - x1) < 1e-6 or abs(p[1] - y0) < 1e-6 or abs(p[1] - y1) < 1e-6)
        ]
        fan(points, (0, 0, 1))

# The bottom, and the walls with the top face's grid points on their top edges.
fan([(-X, -Y, 0.0), (X, -Y, 0.0), (X, Y, 0.0), (-X, Y, 0.0)], (0, 0, -1))
for x, n in [(-X, (-1, 0, 0)), (X, (1, 0, 0))]:
    fan([(x, -Y, 0.0), (x, Y, 0.0)] + [(x, y, Z) for y in YS], n)
for y, n in [(-Y, (0, -1, 0)), (Y, (0, 1, 0))]:
    fan([(-X, y, 0.0), (X, y, 0.0)] + [(x, y, Z) for x in XS], n)

index = {}
vertices = []
faces = []
for t in tris:
    ids = []
    for p in t:
        if p not in index:
            index[p] = len(vertices)
            vertices.append(p)
        ids.append(index[p])
    faces.append(ids)

model = ['<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><metadata name="Title">AXIS dowel-pin test block</metadata><resources><object id="1" type="model" name="Dowel block"><mesh><vertices>']
model += [f'<vertex x="{p[0]:g}" y="{p[1]:g}" z="{p[2]:g}"/>' for p in vertices]
model.append("</vertices><triangles>")
model += [f'<triangle v1="{a}" v2="{b}" v3="{c}"/>' for a, b, c in faces]
model.append('</triangles></mesh></object></resources><build><item objectid="1"/></build></model>')

out = sys.argv[1] if len(sys.argv) > 1 else "examples/dowel-block.3mf"
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr(
        "[Content_Types].xml",
        '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>',
    )
    z.writestr(
        "_rels/.rels",
        '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rel0" Target="/3D/3dmodel.model" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>',
    )
    z.writestr("3D/3dmodel.model", "".join(model))
print(f"{out}: {len(vertices)} vertices, {len(faces)} triangles")
