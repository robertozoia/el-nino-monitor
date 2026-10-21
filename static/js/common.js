// Shared helpers: i18n, theme, data fetching, chart scaffolding (ECharts) and formatting.

export const LANG = window.LANG || "es";
const T = window.I18N || {};
export const t = (k) => T[k] ?? k;
export const MONTHS = t("js_months").split(",");

// ------------------------------------------------------------------ theme

export function css(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function isDark() {
  const forced = document.documentElement.dataset.theme;
  if (forced) return forced === "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

const themeButton = document.getElementById("theme-toggle");
if (themeButton) {
  themeButton.addEventListener("click", () => {
    const next = isDark() ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem("theme", next); } catch (e) { /* private mode */ }
    window.dispatchEvent(new Event("themechange"));
  });
}
window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
  window.dispatchEvent(new Event("themechange"));
});

export function tokens() {
  const v = (n) => css(n);
  return {
    surface: v("--surface-1"), surface2: v("--surface-2"), border: v("--border"), grid: v("--grid"),
    text: v("--text-primary"), text2: v("--text-secondary"), muted: v("--text-muted"),
    warm: [v("--warm-1"), v("--warm-2"), v("--warm-3"), v("--warm-4")],
    cool: [v("--cool-1"), v("--cool-2"), v("--cool-3"), v("--cool-4")],
    neutral: v("--neutral-mid"),
    series: [1, 2, 3, 4, 5, 6, 7, 8].map((i) => v(`--series-${i}`)),
    ocean: v("--ocean"), land: v("--land"), landLine: v("--land-line"),
    font: v("--font"),
  };
}

// ------------------------------------------------------------------ data

const cache = new Map();
export function getJSON(url) {
  if (!cache.has(url)) {
    cache.set(url, fetch(url).then((r) => {
      if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
      return r.json();
    }));
  }
  return cache.get(url);
}

// ------------------------------------------------------------------ formatting

export function fmtSigned(v, d = 2) {
  if (v == null || Number.isNaN(v)) return "—";
  const s = (v > 0 ? "+" : "") + v.toFixed(d);
  return s.replace("-", "−");
}

export function fmtDate(iso) {
  const [y, m, d] = iso.slice(0, 10).split("-").map(Number);
  return `${d} ${MONTHS[m - 1]} ${y}`;
}

export function fmtMonth(iso) {
  const [y, m] = iso.split("-").map(Number);
  return `${MONTHS[m - 1]} ${y}`;
}

// ------------------------------------------------------------------ charts

/** Mounts a chart; `build(tokens)` returns an ECharts option. Re-renders on theme change. */
export function mountChart(el, build) {
  if (typeof el === "string") el = document.getElementById(el);
  if (!el || !window.echarts) return null;
  el.classList.remove("loading");
  el.textContent = "";
  let chart = null;
  const render = () => {
    if (chart) chart.dispose();
    chart = window.echarts.init(el, null, { renderer: "canvas" });
    const option = build(tokens());
    chart.setOption(option);
    return chart;
  };
  render();
  const observer = new ResizeObserver(() => chart && chart.resize());
  observer.observe(el);
  window.addEventListener("themechange", render);
  return {
    get instance() { return chart; },
    rerender: render,
    destroy() {
      observer.disconnect();
      window.removeEventListener("themechange", render);
      if (chart) chart.dispose();
      chart = null;
    },
  };
}

export function showError(el, msg) {
  if (typeof el === "string") el = document.getElementById(el);
  if (!el) return;
  el.classList.remove("loading");
  el.innerHTML = `<div class="empty">${msg || t("no_data_yet")}</div>`;
}

/** Common axis/tooltip styling. */
export function base(k, extra = {}) {
  return {
    animationDuration: 400,
    textStyle: { fontFamily: k.font, color: k.text2 },
    grid: { left: 48, right: 18, top: 36, bottom: 36, containLabel: false },
    tooltip: {
      trigger: "axis",
      backgroundColor: k.surface,
      borderColor: k.border,
      textStyle: { color: k.text, fontSize: 12 },
      axisPointer: { type: "line", lineStyle: { color: k.muted, width: 1 } },
      extraCssText: "box-shadow: 0 2px 10px rgba(0,0,0,.12); border-radius: 8px;",
    },
    legend: { top: 0, left: 0, icon: "roundRect", itemWidth: 14, itemHeight: 3, textStyle: { color: k.text2, fontSize: 12 } },
    ...extra,
  };
}

export function valueAxis(k, extra = {}) {
  const unit = extra.unit ?? "";
  delete extra.unit;
  return {
    type: "value",
    axisLine: { show: false },
    axisTick: { show: false },
    axisLabel: { color: k.muted, fontSize: 11, formatter: (v) => `${v}${unit}` },
    splitLine: { lineStyle: { color: k.grid } },
    ...extra,
  };
}

export function catAxis(k, data, extra = {}) {
  return {
    type: "category",
    data,
    axisLine: { lineStyle: { color: k.border } },
    axisTick: { show: false },
    axisLabel: { color: k.muted, fontSize: 11 },
    ...extra,
  };
}

/** Localised tick labels: year on Jan 1, month name on the 1st, otherwise the day. */
function timeLabel(v) {
  const d = new Date(v);
  const [y, m, day] = [d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()];
  if (m === 0 && day === 1) return `{b|${y}}`;
  if (day === 1) return MONTHS[m];
  return String(day);
}

export function timeAxis(k, extra = {}) {
  return {
    type: "time",
    axisLine: { lineStyle: { color: k.border } },
    axisTick: { show: false },
    axisLabel: { color: k.muted, fontSize: 11, hideOverlap: true, formatter: timeLabel, rich: { b: { fontWeight: 700, color: k.text2, fontSize: 11 } } },
    splitLine: { show: false },
    ...extra,
  };
}

/** Tooltip row with a colour key. Text stays in text colours; the swatch carries identity. */
export function tipRow(color, label, value) {
  return `<div style="display:flex;gap:8px;align-items:center;justify-content:space-between">
    <span><span style="display:inline-block;width:10px;height:3px;border-radius:2px;background:${color};margin-right:6px;vertical-align:middle"></span>${label}</span>
    <strong style="font-variant-numeric:tabular-nums">${value}</strong></div>`;
}

// ------------------------------------------------------------------ SST colours (mirror of grid.rs)

export const SST_STOPS = [
  [-5, [5, 48, 97]], [-3, [33, 102, 172]], [-2, [67, 147, 195]], [-1, [146, 197, 222]], [-0.5, [209, 229, 240]],
  [0, [247, 247, 247]], [0.5, [253, 219, 199]], [1, [244, 165, 130]], [2, [214, 96, 77]], [3, [178, 24, 43]], [5, [103, 0, 31]],
];

export function sstColor(v) {
  const s = SST_STOPS;
  if (v <= s[0][0]) return `rgb(${s[0][1].join(",")})`;
  for (let i = 1; i < s.length; i++) {
    if (v <= s[i][0]) {
      const [v0, c0] = s[i - 1];
      const [v1, c1] = s[i];
      const f = (v - v0) / (v1 - v0);
      return `rgb(${c0.map((c, j) => Math.round(c + (c1[j] - c) * f)).join(",")})`;
    }
  }
  return `rgb(${s[s.length - 1][1].join(",")})`;
}

/** Evenly spaced samples of the SST scale for ECharts continuous visualMaps. */
export function sstRamp(min, max, n = 21) {
  return Array.from({ length: n }, (_, i) => sstColor(min + ((max - min) * i) / (n - 1)));
}

// ------------------------------------------------------------------ RONI outlook fan chart (Now + Indices)

const SEASONS = ["DJF", "JFM", "FMA", "MAM", "AMJ", "MJJ", "JJA", "JAS", "ASO", "SON", "OND", "NDJ"];
const SEASONS_ES = { DJF: "DEF", JFM: "EFM", NDJ: "NDE" };
export const seasonLabel = (code) => (LANG === "es" && SEASONS_ES[code]) || code;

export async function outlookChart(id) {
  let obs, fc;
  try {
    [obs, fc] = await Promise.all([getJSON("/api/series/roni?from=2025-01-01"), getJSON("/api/forecast/cpc_roni_outlook")]);
  } catch (e) {
    return showError(id);
  }
  // Seasons are dated at their centre month; forecast rows are consecutive months.
  const [iy, im] = fc.issued.split("-").map(Number);
  const first = SEASONS.indexOf(fc.rows[0].season) + 1;
  let y = iy, m = first;
  if (m - im > 6) y -= 1;
  if (im - m > 6) y += 1;
  const fcDates = fc.rows.map((_, i) => {
    const mm = ((m - 1 + i) % 12) + 1;
    const yy = y + Math.floor((m - 1 + i) / 12);
    return `${yy}-${String(mm).padStart(2, "0")}-01`;
  });
  const obsMap = new Map(obs.points.map(([d, v]) => [d, v]));
  const dates = [...new Set([...obs.points.map((p) => p[0]), ...fcDates])].sort();
  const label = (d) => {
    const mm = Number(d.slice(5, 7));
    return `${seasonLabel(SEASONS[mm - 1])}\n${d.slice(0, 4)}`;
  };
  const fcIdx = new Map(fcDates.map((d, i) => [d, fc.rows[i].values]));
  const col = (p) => dates.map((d) => (fcIdx.has(d) ? fcIdx.get(d)[p] : null));
  const diff = (a, b) => dates.map((d) => (fcIdx.has(d) ? +(fcIdx.get(d)[b] - fcIdx.get(d)[a]).toFixed(2) : null));
  // columns: 5,15,25,50,75,85,95
  mountChart(id, (k) => {
    const band = (name, lo, a, b, color) => [
      { name: `${name}-base`, type: "line", stack: name, data: col(lo), lineStyle: { opacity: 0 }, symbol: "none", silent: true, tooltip: { show: false } },
      { name, type: "line", stack: name, data: diff(a, b), color, lineStyle: { opacity: 0 }, symbol: "none", areaStyle: { color, opacity: 1 }, silent: true },
    ];
    return {
      ...base(k),
      grid: { left: 48, right: 18, top: 64, bottom: 40 },
      legend: { ...base(k).legend, data: [t("js_observed"), t("js_median"), t("js_range50"), t("js_range70"), t("js_range90")] },
      tooltip: {
        ...base(k).tooltip,
        formatter: (ps) => {
          const d = dates[ps[0].dataIndex];
          const o = obsMap.get(d);
          const f = fcIdx.get(d);
          let html = `<div style="font-weight:700;margin-bottom:4px">${label(d).replace("\n", " ")}</div>`;
          if (o != null) html += tipRow(k.text, t("js_observed"), `${fmtSigned(o)} °C`);
          if (f) {
            html += tipRow(k.warm[2], t("js_median"), `${fmtSigned(f[3])} °C`);
            html += tipRow(k.warm[1], t("js_range50"), `${fmtSigned(f[2])} … ${fmtSigned(f[4])}`);
            html += tipRow(k.warm[0], t("js_range90"), `${fmtSigned(f[0])} … ${fmtSigned(f[6])}`);
          }
          return html;
        },
      },
      xAxis: catAxis(k, dates.map(label), { boundaryGap: false }),
      yAxis: valueAxis(k, { unit: "°" }),
      series: [
        ...band(t("js_range90"), 0, 0, 6, k.warm[0]),
        ...band(t("js_range70"), 1, 1, 5, mix(k.warm[0], k.warm[1])),
        ...band(t("js_range50"), 2, 2, 4, k.warm[1]),
        {
          name: t("js_median"), type: "line", data: col(3), symbol: "circle", symbolSize: 6,
          lineStyle: { color: k.warm[2], width: 2, type: [6, 4] }, itemStyle: { color: k.warm[2] },
        },
        {
          name: t("js_observed"), type: "line", data: dates.map((d) => obsMap.get(d) ?? null), symbol: "circle", symbolSize: 6,
          lineStyle: { color: k.text, width: 2.5 }, itemStyle: { color: k.text },
          markLine: {
            silent: true, symbol: "none",
            lineStyle: { color: k.muted, type: "dotted" },
            label: { color: k.muted, fontSize: 11, formatter: t("js_historic"), position: "insideStartTop" },
            data: [{ yAxis: 2.5 }],
          },
        },
      ],
    };
  });
}

/** 50/50 mix of two CSS colours (hex), used for intermediate band shades. */
export function mix(a, b) {
  const p = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  if (!/^#[0-9a-f]{6}$/i.test(a) || !/^#[0-9a-f]{6}$/i.test(b)) return a;
  const [x, y] = [p(a), p(b)];
  return `#${x.map((c, i) => Math.round((c + y[i]) / 2).toString(16).padStart(2, "0")).join("")}`;
}
