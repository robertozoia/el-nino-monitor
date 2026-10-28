import { t, getJSON, mountChart, showError, base, valueAxis, timeAxis, tipRow, fmtSigned, fmtMonth, fmtDate } from "./common.js";

async function icenChart() {
  let s;
  try {
    s = await getJSON("/api/series/icen");
  } catch (e) {
    return showError("chart-icen");
  }
  mountChart("chart-icen", (k) => {
    const band = (lo, hi, color, name) => [
      { yAxis: lo, itemStyle: { color, opacity: 0.22 }, label: { show: true, position: "insideLeft", formatter: name, color: k.muted, fontSize: 10 } },
      { yAxis: hi },
    ];
    return {
      ...base(k),
      grid: { left: 44, right: 18, top: 20, bottom: 70 },
      legend: { show: false },
      tooltip: {
        ...base(k).tooltip,
        formatter: (ps) => `<div style="font-weight:700;margin-bottom:4px">${fmtMonth(ps[0].value[0])}</div>` +
          tipRow(k.text, "ICEN", `${fmtSigned(ps[0].value[1], 2)} °C`),
      },
      dataZoom: [
        { type: "inside", start: 40, end: 100, zoomOnMouseWheel: "ctrl", moveOnMouseWheel: false, preventDefaultMouseMove: false },
        { type: "slider", start: 40, end: 100, height: 22, bottom: 14, borderColor: k.border, textStyle: { color: k.muted },
          dataBackground: { lineStyle: { color: k.muted }, areaStyle: { color: k.surface2 } } },
      ],
      xAxis: timeAxis(k),
      // Axis follows the data; the open-ended top band is clipped to it.
      yAxis: valueAxis(k, { unit: "°", max: (v) => Math.ceil(v.max + 0.2) }),
      series: [{
        type: "line", data: s.points, showSymbol: false, lineStyle: { color: k.text, width: 1.75 }, itemStyle: { color: k.text },
        markArea: {
          silent: true,
          data: [
            band(0.4, 1.0, k.warm[0], t("js_weak")),
            band(1.0, 1.7, k.warm[1], t("js_moderate")),
            band(1.7, 3.0, k.warm[2], t("js_strong")),
            band(3.0, 9.0, k.warm[3], t("js_extraordinary")),
          ],
        },
      }],
    };
  });
}

async function n12Chart() {
  const from = new Date(Date.now() - 548 * 864e5).toISOString().slice(0, 10);
  let weekly, daily;
  try {
    [weekly, daily] = await Promise.all([
      getJSON(`/api/series/nino12_weekly?from=${from}`),
      getJSON(`/api/series/nino12_oisst?from=${from}`).catch(() => ({ points: [] })),
    ]);
  } catch (e) {
    return showError("chart-n12");
  }
  mountChart("chart-n12", (k) => ({
    ...base(k),
    grid: { left: 40, right: 16, top: 36, bottom: 30 },
    tooltip: {
      ...base(k).tooltip,
      formatter: (ps) => `<div style="font-weight:700;margin-bottom:4px">${fmtDate(new Date(ps[0].value[0]).toISOString())}</div>` +
        ps.map((p) => tipRow(p.color, p.seriesName, `${fmtSigned(p.value[1], 1)} °C`)).join(""),
    },
    xAxis: timeAxis(k),
    yAxis: valueAxis(k, { unit: "°" }),
    series: [
      { name: t("js_daily_n12"), type: "line", data: daily.points, showSymbol: false, lineStyle: { width: 1, color: k.series[0] }, itemStyle: { color: k.series[0] } },
      { name: t("js_weekly_n12"), type: "line", data: weekly.points, showSymbol: false, lineStyle: { width: 2.5, color: k.series[1] }, itemStyle: { color: k.series[1] } },
    ],
  }));
}

/** Daily precipitation by department: bars with chip toggles (CHIRPS v3.0). */
async function precipChart() {
  const meta = window.DATA?.precip;
  const chartEl = document.getElementById("chart-precip");
  if (!meta?.series?.length || !chartEl) return;
  const from = new Date(Date.now() - 130 * 864e5).toISOString().slice(0, 10);
  const selected = new Set(meta.series.filter((s) => s.default).map((s) => s.code));
  const byCode = new Map();
  try {
    const all = await Promise.all(
      meta.series.map((s) => getJSON(`/api/series/precip_${s.code}?from=${from}`).catch(() => ({ points: [] }))),
    );
    meta.series.forEach((s, i) => byCode.set(s.code, all[i]));
  } catch (e) {
    return showError("chart-precip");
  }
  if (![...byCode.values()].some((s) => s.points.length)) return showError("chart-precip");

  const chart = mountChart("chart-precip", (k) => ({
    ...base(k),
    grid: { left: 44, right: 16, top: 30, bottom: 34 },
    legend: { show: false },
    tooltip: {
      ...base(k).tooltip,
      formatter: (ps) => {
        if (!ps.length) return "";
        return `<div style="font-weight:700;margin-bottom:4px">${fmtDate(ps[0].value[0])}</div>` +
          ps.map((p) => tipRow(p.color, p.seriesName, `${Number(p.value[1]).toFixed(1)} ${t("js_mm_day")}`)).join("");
      },
    },
    xAxis: timeAxis(k),
    yAxis: valueAxis(k, { unit: " mm" }),
    series: meta.series
      .filter((s) => selected.has(s.code))
      .map((s, i) => ({
        name: s.name,
        type: "bar",
        data: byCode.get(s.code).points,
        barGap: "0%",
        barCategoryGap: "30%",
        itemStyle: { color: k.series[i % k.series.length] },
      })),
  }));

  const chips = document.getElementById("precip-chips");
  if (!chips || !chart) return;
  chips.querySelectorAll(".chip").forEach((b) => {
    b.addEventListener("click", () => {
      const code = b.dataset.code;
      if (selected.has(code)) selected.delete(code);
      else selected.add(code);
      b.setAttribute("aria-pressed", String(selected.has(code)));
      chart.rerender();
    });
  });
}

icenChart();
n12Chart();
precipChart();
