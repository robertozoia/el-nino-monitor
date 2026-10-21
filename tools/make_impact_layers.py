"""Generates static/layers/typical_{djf,jja}.geojson: a deliberately simplified sketch
of typical El Niño impacts, adapted from NOAA CPC / IRI composite maps.
Shapes are ellipses (centre lon/lat, radii in degrees) so nobody mistakes them for
precise boundaries. Run: python3 tools/make_impact_layers.py"""
import json, math, os

AREAS = {
    "djf": [
        ("wet", -80, -5, 3.5, 5.5, "Costa norte de Perú y Ecuador", "Northern Peru and Ecuador coast"),
        ("wet", -55, -29, 9, 6, "Sur de Brasil, Uruguay y NE de Argentina", "Southern Brazil, Uruguay, NE Argentina"),
        ("wet", -97, 30, 20, 5, "Sur de EE. UU. y norte de México", "Southern US and northern Mexico"),
        ("wet", 41, -1, 6, 8, "África Oriental (oct–dic)", "East Africa (Oct–Dec)"),
        ("wet", -150, 0, 28, 6, "Pacífico ecuatorial central", "Central equatorial Pacific"),
        ("dry", 28, -20, 11, 8, "África Austral", "Southern Africa"),
        ("dry", -63, 2, 14, 8, "Norte de Sudamérica y Amazonía", "Northern South America and Amazonia"),
        ("dry", 128, -6, 24, 11, "Indonesia, Filipinas y norte de Australia", "Indonesia, Philippines, northern Australia"),
        ("dry", 155, 8, 10, 5, "Pacífico occidental (Micronesia)", "Western Pacific (Micronesia)"),
        ("dry", -68, -17, 4, 4, "Altiplano peruano-boliviano", "Peru–Bolivia Altiplano"),
        ("warm", -100, 52, 30, 7, "Canadá y norte de EE. UU.", "Canada and northern US"),
        ("warm", 30, -25, 6, 4, "Sudáfrica (calor)", "South Africa (heat)"),
    ],
    "jja": [
        ("dry", 78, 21, 9, 9, "India (monzón más débil)", "India (weaker monsoon)"),
        ("dry", 125, -10, 30, 15, "Indonesia y este de Australia", "Indonesia and eastern Australia"),
        ("dry", -82, 13, 14, 5, "Centroamérica y Caribe", "Central America and Caribbean"),
        ("dry", 39, 10, 5, 5, "Tierras altas de Etiopía", "Ethiopian highlands"),
        ("wet", -71.5, -35, 2.5, 5, "Chile central", "Central Chile"),
        ("wet", -55, -30, 8, 5, "Sur de Brasil y Uruguay", "Southern Brazil and Uruguay"),
        ("wet", -150, 0, 28, 6, "Pacífico ecuatorial central", "Central equatorial Pacific"),
        ("warm", -82, -6, 4, 7, "Mar cálido frente a Perú y Ecuador", "Warm sea off Peru and Ecuador"),
    ],
}

def ellipse(cx, cy, rx, ry, n=48):
    pts = [[round(cx + rx * math.cos(2 * math.pi * i / n), 2), round(cy + ry * math.sin(2 * math.pi * i / n), 2)] for i in range(n)]
    return pts + [pts[0]]

root = os.path.join(os.path.dirname(__file__), "..", "static", "layers")
os.makedirs(root, exist_ok=True)
for season, areas in AREAS.items():
    fc = {"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": {"kind": k, "es": es, "en": en},
         "geometry": {"type": "Polygon", "coordinates": [ellipse(cx, cy, rx, ry)]}}
        for k, cx, cy, rx, ry, es, en in areas]}
    with open(os.path.join(root, f"typical_{season}.geojson"), "w") as f:
        json.dump(fc, f, ensure_ascii=False, separators=(",", ":"))
    print(season, len(areas))
