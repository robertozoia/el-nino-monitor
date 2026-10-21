import {
  t, getJSON, mountChart, showError, base, valueAxis, timeAxis, catAxis, tipRow, fmtSigned, fmtMonth, MONTHS,
  outlookChart, seasonLabel,
} from "./common.js";

// ------------------------------------------------------------------ history

const HISTORY = {
  roni: { thresholds: [0.5, 1.0, 1.5, 2.0], invert: false },
  oni: { thresholds: [0.5, 1.0, 1.5, 2.0], invert: false },
  mei: { thresholds: [0.5, 1.0, 1.5, 2.0], invert: false },
  soi: { thresholds: [], invert: true },
  icen: { thresholds: [0.4, 1.0, 1.7, 3.0], invert: false },
};

let historyChart = null;

async function showHistory(id) {
  const cfg = HISTORY[id];
  document.getElementById("hist-sub").textContent = t(`js_hist_sub_${id}`);
  let s;
  try {
    s = await getJSON(`/api/series/${id}`);
  } catch (e) {
    return showError("chart-history");
  }
  const build = (k) => {
    // Diverging colour by sign: warm = El Niño side (negative for SOI).
    const warm = k.warm[2], cool = k.cool[2];
    // Bounded pieces: open-ended ones trip an ECharts gradient bug with dataZoom.
    const pieces = cfg.invert
      ? [{ min: -100, max: 0, color: warm }, { min: 0, max: 100, color: cool }]
      : [{ min: -100, max: 0, color: cool }, { min: 0, max: 100, color: warm }];
    return {
      ...base(k),
      grid: { left: 44, right: 18, top: 24, bottom: 70 },
      legend: { show: false },
      tooltip: {
        ...base(k).tooltip,
        formatter: (ps) => {
          const p = ps[0];
          return `<div style="font-weight:700;margin-bottom:4px">${fmtMonth(p.value[0])}</div>` +
            tipRow(p.color, id.toUpperCase(), `${fmtSigned(p.value[1], 2)}${s.unit && s.unit !== "σ" ? " " + s.unit : ""}`);
        },
      },
      visualMap: { show: false, type: "piecewise", dimension: 1, seriesIndex: 0, pieces },
      dataZoom: [
        { type: "inside", start: 55, end: 100, zoomOnMouseWheel: "ctrl", moveOnMouseWheel: false, preventDefaultMouseMove: false },
        { type: "slider", start: 55, end: 100, height: 22, bottom: 14, borderColor: k.border, fillerColor: "rgba(127,127,127,.12)",
          textStyle: { color: k.muted }, dataBackground: { lineStyle: { color: k.muted }, areaStyle: { color: k.surface2 } } },
      ],
      xAxis: timeAxis(k),
      yAxis: valueAxis(k, { unit: s.unit === "°C" ? "°" : "" }),
      series: [{
        type: "line", data: s.points, showSymbol: false, lineStyle: { width: 1.5 }, areaStyle: { opacity: 0.35 },
        markLine: cfg.thresholds.length ? {
          silent: true, symbol: "none", label: { show: false },
          lineStyle: { color: k.muted, type: "dotted", width: 1 },
          data: cfg.thresholds.map((y) => ({ yAxis: y })),
        } : undefined,
      }],
    };
  };
  if (historyChart) historyChart.destroy();
  historyChart = mountChart("chart-history", build);
  document.getElementById("hist-foot").textContent =
    `${t("source")}: ${s.source === "igp_icen" ? "IGP / ENFEN" : s.source === "psl_mei" ? "NOAA PSL" : "NOAA CPC"}.`;
}

function picker(groupId, onPick) {
  const group = document.getElementById(groupId);
  group.addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    for (const x of group.querySelectorAll("button")) x.setAttribute("aria-pressed", String(x === b));
    onPick(b.dataset.id);
  });
}

// ------------------------------------------------------------------ analogs

let analogChart = null;

async function showAnalogs(id) {
  let a;
  try {
    a = await getJSON(`/api/analogs/${id}`);
  } catch (e) {
    return showError("chart-analogs");
  }
  const labels = Array.from({ length: 24 }, (_, i) => `${MONTHS[i % 12]}${i >= 12 ? " +1" : ""}`);
  const build = (k) => {
    let slot = 0;
    const series = a.events.map((ev) => {
      const color = ev.current ? k.text : k.series[slot++];
      return {
        name: ev.current ? `${ev.label} (${t("js_current")})` : ev.label,
        type: "line", data: ev.values, showSymbol: false, connectNulls: false,
        z: ev.current ? 10 : 2,
        lineStyle: { width: ev.current ? 3 : 1.75, color }, itemStyle: { color },
        emphasis: { focus: "series" },
        endLabel: ev.current ? { show: true, formatter: (p) => fmtSigned(p.value, 2), color: k.text, fontWeight: 700 } : undefined,
      };
    });
    return {
      ...base(k),
      grid: { left: 44, right: 50, top: 44, bottom: 40 },
      tooltip: {
        ...base(k).tooltip,
        formatter: (ps) => {
          let html = `<div style="font-weight:700;margin-bottom:4px">${labels[ps[0].dataIndex]}</div>`;
          for (const p of ps) if (p.value != null) html += tipRow(p.color, p.seriesName, fmtSigned(p.value, 2));
          return html;
        },
      },
      xAxis: catAxis(k, labels, { boundaryGap: false, name: t("js_month_axis"), nameLocation: "middle", nameGap: 26, nameTextStyle: { color: k.muted, fontSize: 11 } }),
      yAxis: valueAxis(k, { unit: "°" }),
      series,
    };
  };
  if (analogChart) analogChart.destroy();
  analogChart = mountChart("chart-analogs", build);
}

// ------------------------------------------------------------------ strength probabilities

const BINS = ["≤ −2.0", "−2.0…−1.5", "−1.5…−1.0", "−1.0…−0.5", "−0.5…+0.5", "+0.5…+1.0", "+1.0…+1.5", "+1.5…+2.0", "≥ +2.0"];

async function strengthChart() {
  let f;
  try {
    f = await getJSON("/api/forecast/cpc_strength");
  } catch (e) {
    return showError("chart-strength");
  }
  const seasons = f.rows.map((r) => seasonLabel(r.season));
  mountChart("chart-strength", (k) => {
    const colors = [k.cool[3], k.cool[2], k.cool[1], k.cool[0], k.neutral, k.warm[0], k.warm[1], k.warm[2], k.warm[3]];
    // Only bins that carry probability somewhere; order kept from cold to warm.
    const used = BINS.map((_, i) => i).filter((i) => f.rows.some((r) => r.values[i] > 0));
    return {
      ...base(k),
      grid: { left: 44, right: 12, top: 56, bottom: 30 },
      legend: { ...base(k).legend, icon: "roundRect", itemWidth: 10, itemHeight: 10, data: used.slice().reverse().map((i) => `${BINS[i]} °C`) },
      tooltip: {
        ...base(k).tooltip, axisPointer: { type: "shadow" },
        formatter: (ps) => {
          let html = `<div style="font-weight:700;margin-bottom:4px">${ps[0].name}</div>`;
          for (const p of ps.slice().reverse()) if (p.value > 0) html += tipRow(p.color, p.seriesName, `${p.value}%`);
          return html;
        },
      },
      xAxis: catAxis(k, seasons),
      yAxis: valueAxis(k, { max: 100, axisLabel: { color: k.muted, formatter: "{value}%" } }),
      series: used.map((i) => ({
        name: `${BINS[i]} °C`, type: "bar", stack: "p", barWidth: "62%",
        data: f.rows.map((r) => r.values[i]),
        itemStyle: { color: colors[i], borderColor: k.surface, borderWidth: 1 },
      })),
    };
  });
}

picker("hist-picker", showHistory);
picker("analog-picker", showAnalogs);
showHistory("roni");
showAnalogs("roni");
outlookChart("chart-outlook");
strengthChart();
