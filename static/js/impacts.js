import { t, LANG, getJSON, mountChart, showError, base, valueAxis, timeAxis, tipRow, fmtSigned, fmtMonth, tokens } from "./common.js";

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function kindColors(k) {
  return { FL: k.series[0], DR: k.series[3], TC: k.series[6], WF: k.series[7] };
}

function typicalColor(k) {
  return ["match", ["get", "kind"], "wet", k.series[2], "dry", k.series[3], "warm", k.series[7], k.muted];
}

async function impactMap() {
  const el = document.getElementById("impact-map");
  if (!window.maplibregl) return;
  const k = tokens();
  const month = new Date().getMonth() + 1;
  let season = month >= 5 && month <= 9 ? "jja" : "djf";
  const picker = document.getElementById("season-picker");
  for (const b of picker.querySelectorAll("button")) b.setAttribute("aria-pressed", String(b.dataset.season === season));

  const colors = kindColors(k);
  const map = new window.maplibregl.Map({
    container: el,
    style: {
      version: 8,
      sources: {
        countries: { type: "geojson", data: "/static/geo/countries.geojson" },
        typical: { type: "geojson", data: `/static/layers/typical_${season}.geojson` },
        events: { type: "geojson", data: `/api/events?days=180&lang=${LANG}` },
      },
      layers: [
        { id: "bg", type: "background", paint: { "background-color": k.ocean } },
        { id: "land", type: "fill", source: "countries", paint: { "fill-color": k.land } },
        { id: "typical-fill", type: "fill", source: "typical", paint: { "fill-color": typicalColor(k), "fill-opacity": 0.28 } },
        { id: "typical-line", type: "line", source: "typical", paint: { "line-color": typicalColor(k), "line-width": 1.25, "line-dasharray": [2, 2] } },
        { id: "borders", type: "line", source: "countries", paint: { "line-color": k.landLine, "line-width": 0.5 } },
        {
          id: "events", type: "circle", source: "events",
          paint: {
            "circle-color": ["match", ["get", "kind"], "FL", colors.FL, "DR", colors.DR, "TC", colors.TC, "WF", colors.WF, k.muted],
            "circle-radius": ["case", ["==", ["get", "alert"], "Red"], 7, 5],
            "circle-stroke-color": ["case", ["==", ["get", "alert"], "Red"], k.text, k.surface],
            "circle-stroke-width": ["case", ["==", ["get", "alert"], "Red"], 2, 1.5],
            "circle-opacity": ["case", ["get", "current"], 1, 0.6],
          },
        },
      ],
    },
    center: [10, 8],
    zoom: window.innerWidth < 640 ? 0 : 0.9,
    minZoom: 0,
    maxZoom: 7,
    attributionControl: { compact: true, customAttribution: "GDACS · Natural Earth" },
  });
  map.addControl(new window.maplibregl.NavigationControl({ showCompass: false }), "top-right");

  picker.addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    for (const x of picker.querySelectorAll("button")) x.setAttribute("aria-pressed", String(x === b));
    season = b.dataset.season;
    const vis = season === "none" ? "none" : "visible";
    map.setLayoutProperty("typical-fill", "visibility", vis);
    map.setLayoutProperty("typical-line", "visibility", vis);
    if (season !== "none") map.getSource("typical").setData(`/static/layers/typical_${season}.geojson`);
  });

  const popup = new window.maplibregl.Popup({ closeButton: true, maxWidth: "300px" });
  map.on("click", "events", (e) => {
    const p = e.features[0].properties;
    popup.setLngLat(e.lngLat).setHTML(
      `<div style="font-weight:700;margin-bottom:2px">${esc(p.name)}</div>
       <div class="muted xsmall">${esc(p.kind_label)} · ${t("js_alert")} ${esc(t("alert_" + p.alert))}</div>
       <div class="xsmall" style="margin:4px 0">${esc(p.countries)}</div>
       <div class="xsmall muted">${esc(p.from)}${p.to ? " → " + esc(p.to) : ""}</div>
       ${p.severity ? `<div class="xsmall" style="margin-top:4px">${esc(p.severity)}</div>` : ""}
       ${p.url ? `<a class="xsmall" href="${esc(p.url)}" target="_blank" rel="noopener">${t("js_more")} →</a>` : ""}`,
    ).addTo(map);
  });
  map.on("click", "typical-fill", (e) => {
    if (map.queryRenderedFeatures(e.point, { layers: ["events"] }).length) return;
    const p = e.features[0].properties;
    const label = { wet: t("js_typical_wet"), dry: t("js_typical_dry"), warm: t("js_typical_warm") }[p.kind];
    popup.setLngLat(e.lngLat).setHTML(`<div style="font-weight:700">${esc(p[LANG])}</div><div class="xsmall muted">${label}</div>`).addTo(map);
  });
  for (const layer of ["events", "typical-fill"]) {
    map.on("mouseenter", layer, () => (map.getCanvas().style.cursor = "pointer"));
    map.on("mouseleave", layer, () => (map.getCanvas().style.cursor = ""));
  }

  window.addEventListener("themechange", () => {
    const n = tokens();
    const c = kindColors(n);
    map.setPaintProperty("bg", "background-color", n.ocean);
    map.setPaintProperty("land", "fill-color", n.land);
    map.setPaintProperty("borders", "line-color", n.landLine);
    map.setPaintProperty("typical-fill", "fill-color", typicalColor(n));
    map.setPaintProperty("typical-line", "line-color", typicalColor(n));
    map.setPaintProperty("events", "circle-color", ["match", ["get", "kind"], "FL", c.FL, "DR", c.DR, "TC", c.TC, "WF", c.WF, n.muted]);
    map.setPaintProperty("events", "circle-stroke-color", ["case", ["==", ["get", "alert"], "Red"], n.text, n.surface]);
  });
}

async function globalTemp() {
  let s;
  try {
    s = await getJSON("/api/series/global_temp?from=1950-01-01");
  } catch (e) {
    return showError("chart-global");
  }
  mountChart("chart-global", (k) => ({
    ...base(k),
    grid: { left: 44, right: 16, top: 24, bottom: 30 },
    legend: { show: false },
    tooltip: {
      ...base(k).tooltip,
      formatter: (ps) => `<div style="font-weight:700;margin-bottom:4px">${fmtMonth(ps[0].value[0])}</div>` +
        tipRow(ps[0].color, t("js_temp"), `${fmtSigned(ps[0].value[1], 2)} °C`),
    },
    visualMap: { show: false, type: "piecewise", dimension: 1, pieces: [{ min: -100, max: 0, color: k.cool[2] }, { min: 0, max: 100, color: k.warm[2] }] },
    xAxis: timeAxis(k),
    yAxis: valueAxis(k, { unit: "°" }),
    series: [{ type: "line", data: s.points, showSymbol: false, lineStyle: { width: 1.25 } }],
  }));
}

impactMap();
globalTemp();
