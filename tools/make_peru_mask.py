"""Generates static/geo/peru_departments.json and static/geo/peru_mask.bin: a mask of
Peru's 25 departments over the CHIRPS v3.0 0.05° grid window (0°–18.5°S, 81.5°–68.5°W).

Source: Natural Earth 10m admin-1 (public domain), ne_10m_admin_1_states_provinces.
Requires pyshp (pip install pyshp). Downloads the NE zip once into the system temp dir.

Output:
  peru_departments.json  metadata: window geometry, department code + names, north→south order
  peru_mask.bin          260 x 370 bytes; byte = department index (0..24) or 255 = outside Peru

Run: python3 tools/make_peru_mask.py
"""
import json, os, tempfile, urllib.request, zipfile, io, math, sys

NE_URL = "https://naciscdn.org/naturalearth/10m/cultural/ne_10m_admin_1_states_provinces.zip"

# CHIRPS v3.0 grid window: cell centres at multiples of 0.05°.
CELL = 0.05
LON0, LON1 = -81.475, -68.525   # westernmost / easternmost cell centre
LAT_TOP, LAT_BOTTOM = -0.025, -18.475  # northernmost / southernmost cell centre
NCOLS = int(round((LON1 - LON0) / CELL)) + 1          # 260
NROWS = int(round((LAT_TOP - LAT_BOTTOM) / CELL)) + 1  # 370

# NE "name" -> (code, es, en), ordered north→south (Callao grouped with Lima).
# Natural Earth splits the capital out as "Lima Province": merged into Lima.
DEPTS = [
    ("Tumbes", "tumbes", "Tumbes", "Tumbes"),
    ("Piura", "piura", "Piura", "Piura"),
    ("Amazonas", "amazonas", "Amazonas", "Amazonas"),
    ("Loreto", "loreto", "Loreto", "Loreto"),
    ("Cajamarca", "cajamarca", "Cajamarca", "Cajamarca"),
    ("San Martín", "san_martin", "San Martín", "San Martín"),
    ("Lambayeque", "lambayeque", "Lambayeque", "Lambayeque"),
    ("La Libertad", "la_libertad", "La Libertad", "La Libertad"),
    ("Áncash", "ancash", "Áncash", "Áncash"),
    ("Huánuco", "huanuco", "Huánuco", "Huánuco"),
    ("Ucayali", "ucayali", "Ucayali", "Ucayali"),
    ("Pasco", "pasco", "Pasco", "Pasco"),
    ("Callao", "callao", "Callao", "Callao"),
    ("Lima", "lima", "Lima", "Lima"),
    ("Lima Province", "lima", "Lima", "Lima"),
    ("Junín", "junin", "Junín", "Junín"),
    ("Madre de Dios", "madre_de_dios", "Madre de Dios", "Madre de Dios"),
    ("Huancavelica", "huancavelica", "Huancavelica", "Huancavelica"),
    ("Ica", "ica", "Ica", "Ica"),
    ("Ayacucho", "ayacucho", "Ayacucho", "Ayacucho"),
    ("Apurímac", "apurimac", "Apurímac", "Apurímac"),
    ("Cusco", "cusco", "Cusco", "Cusco"),
    ("Puno", "puno", "Puno", "Puno"),
    ("Arequipa", "arequipa", "Arequipa", "Arequipa"),
    ("Moquegua", "moquegua", "Moquegua", "Moquegua"),
    ("Tacna", "tacna", "Tacna", "Tacna"),
]
# Distinct departments (Lima Province merges into Lima); NE names map onto them.
CODES = []
for d in DEPTS:
    if d[1] not in CODES:
        CODES.append(d[1])
BY_NAME = {d[0]: CODES.index(d[1]) for d in DEPTS}
DEPTS_UNIQUE = [next(x for x in DEPTS if x[1] == c) for c in CODES]

root = os.path.join(os.path.dirname(__file__), "..")


def download_ne() -> bytes:
    cache = os.path.join(tempfile.gettempdir(), "ne_10m_admin_1_states_provinces.zip")
    if not os.path.exists(cache):
        print(f"downloading {NE_URL}")
        with urllib.request.urlopen(NE_URL, timeout=120) as r, open(cache, "wb") as f:
            f.write(r.read())
    return open(cache, "rb").read()


def rings_of(shape) -> list:
    """Splits a pyshp polygon shape into rings (parts are rings; holes are rings too)."""
    pts = shape.points
    parts = list(shape.parts) + [len(pts)]
    return [pts[parts[i]:parts[i + 1]] for i in range(len(parts) - 1)]


def bbox(rings):
    xs = [p[0] for r in rings for p in r]
    ys = [p[1] for r in rings for p in r]
    return min(xs), min(ys), max(xs), max(ys)


def in_ring(x, y, ring) -> bool:
    """Ray casting; even-odd rule across all rings of a feature handles holes."""
    inside = False
    j = len(ring) - 1
    for i in range(len(ring)):
        xi, yi = ring[i]
        xj, yj = ring[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi:
            inside = not inside
        j = i
    return inside


def main() -> None:
    import shapefile

    data = download_ne()
    with zipfile.ZipFile(io.BytesIO(data)) as zf:
        names = {n.split("/")[-1]: n for n in zf.namelist()}
        shp = zf.read(names["ne_10m_admin_1_states_provinces.shp"])
        dbf = zf.read(names["ne_10m_admin_1_states_provinces.dbf"])
    sf = shapefile.Reader(shp=io.BytesIO(shp), dbf=io.BytesIO(dbf))
    fields = [f[0] for f in sf.fields[1:]]
    name_i, adm0_i = fields.index("name"), fields.index("adm0_a3")

    features = []  # (dept_index_in_unique, rings, bbox)
    for rec, shape in zip(sf.records(), sf.shapes()):
        if rec[adm0_i] != "PER" or rec[name_i] not in BY_NAME:
            continue
        rings = rings_of(shape)
        features.append((BY_NAME[rec[name_i]], rings, bbox(rings)))
    found = sorted({i for i, _, _ in features})
    missing = [d[0] for i, d in enumerate(DEPTS_UNIQUE) if i not in found]
    if missing or len(features) < len(DEPTS_UNIQUE):
        sys.exit(f"expected {len(DEPTS_UNIQUE)} departments, missing: {missing}, found {len(features)}")

    # Assign each cell centre to the first department whose polygon contains it.
    mask = bytearray([255]) * (NCOLS * NROWS)
    for idx, rings, (x0, y0, x1, y1) in features:
        n = 0
        for r in range(NROWS):
            y = LAT_TOP - r * CELL
            if y < y0 - 1e-9 or y > y1 + 1e-9:
                continue
            for c in range(NCOLS):
                x = LON0 + c * CELL
                if x < x0 - 1e-9 or x > x1 + 1e-9:
                    continue
                if mask[r * NCOLS + c] != 255:
                    continue
                if sum(in_ring(x, y, ring) for ring in rings) % 2 == 1:
                    mask[r * NCOLS + c] = idx
                    n += 1
        print(f"{DEPTS_UNIQUE[idx][1]:<14} {n:>5} cells")

    # North→south order: by mean latitude of assigned cells.
    cells = [[] for _ in DEPTS_UNIQUE]
    for r in range(NROWS):
        for c in range(NCOLS):
            v = mask[r * NCOLS + c]
            if v != 255:
                cells[v].append((LON0 + c * CELL, LAT_TOP - r * CELL))
    order = sorted(range(len(DEPTS_UNIQUE)), key=lambda i: -sum(y for _, y in cells[i]) / len(cells[i]))
    order = {old: new for new, old in enumerate(order)}
    remapped = bytearray(255 for _ in range(NCOLS * NROWS))
    for i, v in enumerate(mask):
        if v != 255:
            remapped[i] = order[v]

    depts = [
        {"code": d[1], "name_es": d[2], "name_en": d[3]}
        for i, d in sorted((order[i], d) for i, d in enumerate(DEPTS_UNIQUE))
    ]
    meta = {
        "window": {"cell": CELL, "ncols": NCOLS, "nrows": NROWS, "lon0": LON0, "lat_top": LAT_TOP},
        "departments": depts,
    }
    out_dir = os.path.join(root, "static", "geo")
    os.makedirs(out_dir, exist_ok=True)
    with open(os.path.join(out_dir, "peru_departments.json"), "w") as f:
        json.dump(meta, f, ensure_ascii=False, separators=(",", ":"))
    with open(os.path.join(out_dir, "peru_mask.bin"), "wb") as f:
        f.write(remapped)
    print(f"mask {NCOLS}x{NROWS} -> static/geo/peru_mask.bin ({len(remapped)} bytes)")
    print("order:", " → ".join(d["code"] for d in depts))


if __name__ == "__main__":
    main()
