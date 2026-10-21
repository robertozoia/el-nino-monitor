"""Generates static/learn/world.json: a light land outline for the isometric map plate
on the Learn page. Rings from static/geo/countries.geojson (Natural Earth 50m), simplified
with Douglas-Peucker, tiny islands dropped, Antarctica removed. Longitudes are shifted
to a Pacific-centred range [-30, 330) so the Pacific is never split.
Run: python3 tools/make_learn_world.py"""
import json, os

TOLERANCE = 0.35      # degrees
MIN_AREA = 0.6        # square degrees
LON0 = -30

root = os.path.join(os.path.dirname(__file__), "..")


def dp(pts, tol):
    if len(pts) < 3:
        return pts
    keep = [False] * len(pts)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    while stack:
        a, b = stack.pop()
        (x1, y1), (x2, y2) = pts[a], pts[b]
        dx, dy = x2 - x1, y2 - y1
        norm = (dx * dx + dy * dy) ** 0.5 or 1e-9
        best, idx = 0.0, -1
        for i in range(a + 1, b):
            x, y = pts[i]
            d = abs(dy * (x - x1) - dx * (y - y1)) / norm
            if d > best:
                best, idx = d, i
        if best > tol:
            keep[idx] = True
            stack += [(a, idx), (idx, b)]
    return [p for p, k in zip(pts, keep) if k]


def area(r):
    return abs(sum(r[i][0] * r[i + 1][1] - r[i + 1][0] * r[i][1] for i in range(len(r) - 1))) / 2


def shift(lon):
    return lon + 360 if lon < LON0 else lon


src = json.load(open(os.path.join(root, "static", "geo", "countries.geojson")))
rings = []
for f in src["features"]:
    g = f["geometry"]
    polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
    for poly in polys:
        outer = poly[0]
        if max(p[1] for p in outer) < -58 or area(outer) < MIN_AREA:
            continue
        # Shift the whole ring by its first point so it stays contiguous; the client
        # draws a copy 360° away and clips to the plate.
        off = shift(outer[0][0]) - outer[0][0]
        ring = [(round(x + off, 2), round(y, 2)) for x, y in outer]
        # A closed ring has identical endpoints, so simplify its two halves separately.
        far = max(range(len(ring)), key=lambda i: (ring[i][0] - ring[0][0]) ** 2 + (ring[i][1] - ring[0][1]) ** 2)
        ring = dp(ring[: far + 1], TOLERANCE)[:-1] + dp(ring[far:], TOLERANCE)
        if len(ring) >= 4:
            rings.append([v for p in ring for v in (round(p[0], 1), round(p[1], 1))])

out = os.path.join(root, "static", "learn", "world.json")
os.makedirs(os.path.dirname(out), exist_ok=True)
with open(out, "w") as fh:
    json.dump({"lon0": LON0, "rings": rings}, fh, separators=(",", ":"))
print(f"{len(rings)} rings, {os.path.getsize(out) // 1024} KB -> {out}")
