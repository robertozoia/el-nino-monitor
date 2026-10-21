import {
  t, getJSON, mountChart, showError, base, valueAxis, timeAxis, catAxis, tipRow, fmtSigned, fmtDate,
  tokens, sstColor, sstRamp,
} from "./common.js";

// Niño regions (°E, negative = west). Niño 4 straddles the antimeridian, so boxes are
// drawn as open line strings split at ±180° rather than polygons.
const BOXES = [
  { name: "Niño 4", w: 160, e: -150, s: -5, n: 5, label: -175 },
  { name: "Niño 3.4", w: -170, e: -120, s: -5, n: 5 },
  { name: "Niño 3", w: -150, e: -90, s: -5, n: 5 },
  { name: "Niño 1+2", w: -90, e: -80, s: -10, n: 0 },
];

function boxLines(b) {
  if (b.w < b.e) {
    return [[[b.w, b.s], [b.e, b.s], [b.e, b.n], [b.w, b.n], [b.w, b.s]]];
  }
  return [
    [[180, b.s], [b.w, b.s], [b.w, b.n], [180, b.n]],
    [[-180, b.n], [b.e, b.n], [b.e, b.s], [-180, b.s]],
  ];
}

function boxesGeoJSON() {
  return {
    type: "FeatureCollection",
    features: BOXES.map((b) => ({
      type: "Feature",
      properties: { name: b.name },
      geometry: { type: "MultiLineString", coordinates: boxLines(b) },
    })),
  };
}

function mapStyle(k, tiles, maxzoom) {
  return {
    version: 8,
    sources: {
      sst: { type: "raster", tiles: [tiles], tileSize: 256, maxzoom, attribution: "NOAA OISST" },
      countries: { type: "geojson", data: "/static/geo/countries.geojson" },
      boxes: { type: "geojson", data: boxesGeoJSON() },
    },
    layers: [
      { id: "bg", type: "background", paint: { "background-color": k.ocean } },
      { id: "sst", type: "raster", source: "sst", paint: { "raster-opacity": 1, "raster-resampling": "linear", "raster-fade-duration": 0 } },
      { id: "land", type: "fill", source: "countries", paint: { "fill-color": k.land } },
      { id: "borders", type: "line", source: "countries", paint: { "line-color": k.landLine, "line-width": 0.5 } },
      { id: "boxes", type: "line", source: "boxes", paint: { "line-color": k.text, "line-width": 1.25, "line-dasharray": [3, 2] } },
    ],
  };
}

async function sstMap() {
  const el = document.getElementById("sst-map");
  let frames;
  try {
    frames = await getJSON("/api/sst/frames");
  } catch (e) {
    frames = { dates: [] };
  }
  if (!frames.dates.length || !window.maplibregl) {
    el.innerHTML = `<div class="empty">${t("js_no_frames")}</div>`;
    return;
  }
  const urlFor = (d) => frames.tiles.replace("{date}", d);
  const slider = document.getElementById("frame");
  const dateLabel = document.getElementById("frame-date");
  const readout = document.getElementById("hover-readout");
  slider.max = String(frames.dates.length - 1);
  slider.value = slider.max;
  let current = frames.dates.length - 1;
  dateLabel.textContent = fmtDate(frames.dates[current]);

  document.getElementById("ramp").style.background =
    `linear-gradient(90deg, ${sstRamp(-5, 5, 21).join(",")})`;

  const map = new window.maplibregl.Map({
    container: el,
    style: mapStyle(tokens(), urlFor(frames.dates[current]), frames.maxzoom),
    center: [-155, 0],
    zoom: window.innerWidth < 640 ? 0.3 : 1.15,
    minZoom: 0,
    maxZoom: 6,
    renderWorldCopies: true,
    attributionControl: { compact: true, customAttribution: "Natural Earth" },
  });
  map.addControl(new window.maplibregl.NavigationControl({ showCompass: false }), "top-right");

  // Region labels as HTML markers (no glyph server needed).
  const markers = BOXES.map((b) => {
    const m = document.createElement("div");
    m.textContent = b.name;
    m.style.cssText = "font:700 11px var(--font);color:var(--text-primary);text-shadow:0 0 3px var(--surface-1),0 0 3px var(--surface-1);pointer-events:none";
    return new window.maplibregl.Marker({ element: m, anchor: "bottom" }).setLngLat([b.label ?? (b.w + b.e) / 2, b.n + 0.5]).addTo(map);
  });

  const show = (i) => {
    current = i;
    slider.value = String(i);
    dateLabel.textContent = fmtDate(frames.dates[i]);
    const src = map.getSource("sst");
    if (src) src.setTiles([urlFor(frames.dates[i])]);
  };
  slider.addEventListener("input", () => show(Number(slider.value)));

  let timer = null;
  const play = document.getElementById("play");
  play.addEventListener("click", () => {
    if (timer) {
      clearInterval(timer);
      timer = null;
      play.textContent = "▶";
      play.setAttribute("aria-label", t("js_play"));
      return;
    }
    play.textContent = "❚❚";
    play.setAttribute("aria-label", t("js_pause"));
    if (current >= frames.dates.length - 1) show(0);
    timer = setInterval(() => {
      const next = current + 1;
      if (next >= frames.dates.length) {
        clearInterval(timer);
        timer = null;
        play.textContent = "▶";
        return;
      }
      show(next);
    }, 450);
  });

  const toggle = document.getElementById("toggle-boxes");
  toggle.addEventListener("click", () => {
    const on = toggle.getAttribute("aria-pressed") !== "true";
    toggle.setAttribute("aria-pressed", String(on));
    map.setLayoutProperty("boxes", "visibility", on ? "visible" : "none");
    markers.forEach((m) => (m.getElement().style.display = on ? "" : "none"));
  });

  let pending = null;
  map.on("mousemove", (e) => {
    clearTimeout(pending);
    pending = setTimeout(async () => {
      const { lat, lng } = e.lngLat;
      try {
        const r = await fetch(`/api/sst/point?date=${frames.dates[current]}&lat=${lat.toFixed(2)}&lon=${lng.toFixed(2)}`).then((x) => x.json());
        const pos = `${Math.abs(lat).toFixed(1)}°${lat >= 0 ? "N" : "S"}, ${Math.abs(((lng + 540) % 360) - 180).toFixed(1)}°${((lng + 540) % 360) - 180 >= 0 ? "E" : "W"}`;
        readout.innerHTML = r.value == null
          ? `${pos}: ${t("js_land")}`
          : `${pos}: <strong style="color:var(--text-primary)">${fmtSigned(r.value, 2)} °C</strong> <span class="swatch" style="background:${sstColor(r.value)}"></span>`;
      } catch (err) { /* ignore */ }
    }, 120);
  });

  window.addEventListener("themechange", () => {
    const k = tokens();
    map.setPaintProperty("bg", "background-color", k.ocean);
    map.setPaintProperty("land", "fill-color", k.land);
    map.setPaintProperty("borders", "line-color", k.landLine);
    map.setPaintProperty("boxes", "line-color", k.text);
  });
}

const lonLabel = (l) => {
  const w = l > 180 ? 360 - l : null;
  if (Math.abs(l - 180) < 0.6) return "180°";
  return w != null ? `${Math.round(w)}°W` : `${Math.round(l)}°E`;
};

async function hovmoller() {
  let h;
  try {
    h = await getJSON("/api/sst/hovmoller");
  } catch (e) {
    return showError("chart-hov");
  }
  if (!h.dates.length) return showError("chart-hov", t("js_no_frames"));
  mountChart("chart-hov", (k) => ({
    ...base(k),
    grid: { left: 52, right: 16, top: 10, bottom: 70 },
    legend: { show: false },
    tooltip: {
      trigger: "item", backgroundColor: k.surface, borderColor: k.border, textStyle: { color: k.text, fontSize: 12 },
      formatter: (p) => `<div style="font-weight:700">${fmtDate(h.dates[p.value[1]])}</div>${lonLabel(h.lons[p.value[0]])}: <strong>${fmtSigned(p.value[2], 2)} °C</strong>`,
    },
    xAxis: catAxis(k, h.lons.map(lonLabel), {
      axisLabel: { color: k.muted, fontSize: 11, interval: (i) => [0, 60, 120, 159].includes(i) || i % 20 === 0 && i < 155 },
      name: t("js_longitude"), nameLocation: "middle", nameGap: 26, nameTextStyle: { color: k.muted, fontSize: 11 },
    }),
    yAxis: catAxis(k, h.dates.map((d) => fmtDate(d).replace(/ \d{4}$/, "")), { axisLabel: { color: k.muted, fontSize: 11 } }),
    visualMap: {
      type: "continuous", min: -4, max: 4, calculable: false, orient: "horizontal", left: "center", bottom: 0,
      itemHeight: 220, itemWidth: 10, text: ["+4 °C", "−4 °C"], textStyle: { color: k.muted, fontSize: 11 },
      inRange: { color: sstRamp(-4, 4, 17) },
    },
    series: [{ type: "heatmap", data: h.values, progressive: 0 }],
  }));
}

async function dailyIndices() {
  const from = new Date(Date.now() - 400 * 864e5).toISOString().slice(0, 10);
  let a, b;
  try {
    [a, b] = await Promise.all([getJSON(`/api/series/nino34_oisst?from=${from}`), getJSON(`/api/series/nino12_oisst?from=${from}`)]);
  } catch (e) {
    return showError("chart-daily", t("js_no_frames"));
  }
  mountChart("chart-daily", (k) => ({
    ...base(k),
    grid: { left: 40, right: 16, top: 36, bottom: 30 },
    tooltip: {
      ...base(k).tooltip,
      formatter: (ps) => `<div style="font-weight:700;margin-bottom:4px">${fmtDate(new Date(ps[0].value[0]).toISOString())}</div>` +
        ps.map((p) => tipRow(p.color, p.seriesName, `${fmtSigned(p.value[1], 2)} °C`)).join(""),
    },
    xAxis: timeAxis(k),
    yAxis: valueAxis(k, { unit: "°" }),
    series: [
      { name: t("js_daily_n34"), type: "line", data: a.points, showSymbol: false, lineStyle: { width: 2, color: k.series[0] }, itemStyle: { color: k.series[0] } },
      { name: t("js_daily_n12"), type: "line", data: b.points, showSymbol: false, lineStyle: { width: 2, color: k.series[1] }, itemStyle: { color: k.series[1] } },
    ],
  }));
}

sstMap();
hovmoller();
dailyIndices();
