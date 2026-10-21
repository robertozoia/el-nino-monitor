import { t, getJSON, mountChart, showError, base, valueAxis, tipRow, fmtSigned, MONTHS, outlookChart } from "./common.js";

const YEARS = [1982, 1997, 2015, 2023];
const CURRENT = 2026;

async function compareChart() {
  let s;
  try {
    s = await getJSON("/api/series/nino34_weekly?from=1982-01-01");
  } catch (e) {
    return showError("chart-compare");
  }
  // Day-of-year x so every year shares one axis.
  const byYear = new Map();
  for (const [d, v] of s.points) {
    const y = Number(d.slice(0, 4));
    if (![...YEARS, CURRENT].includes(y)) continue;
    const doy = (Date.parse(d) - Date.UTC(y, 0, 1)) / 864e5;
    if (!byYear.has(y)) byYear.set(y, []);
    byYear.get(y).push([doy, v]);
  }
  const monthStarts = Array.from({ length: 12 }, (_, i) => (Date.UTC(2001, i, 1) - Date.UTC(2001, 0, 1)) / 864e5);
  mountChart("chart-compare", (k) => {
    const color = (y) => (y === CURRENT ? k.text : k.series[YEARS.indexOf(y)]);
    const series = [...YEARS, CURRENT].map((y) => ({
      name: y === CURRENT ? `${y} (${t("js_current")})` : String(y),
      type: "line",
      data: byYear.get(y) || [],
      showSymbol: false,
      z: y === CURRENT ? 10 : 2,
      lineStyle: { width: y === CURRENT ? 3 : 1.75, color: color(y) },
      itemStyle: { color: color(y) },
      emphasis: { focus: "series" },
      endLabel: y === CURRENT ? { show: true, formatter: (p) => fmtSigned(p.value[1], 1), color: k.text, fontWeight: 700 } : undefined,
    }));
    return {
      ...base(k),
      grid: { left: 40, right: 44, top: 40, bottom: 30 },
      tooltip: {
        ...base(k).tooltip,
        formatter: (ps) => {
          const doy = ps[0].value[0];
          const dt = new Date(Date.UTC(2001, 0, 1) + doy * 864e5);
          let html = `<div style="font-weight:700;margin-bottom:4px">${t("js_week")} ~${dt.getUTCDate()} ${MONTHS[dt.getUTCMonth()]}</div>`;
          for (const p of ps) html += tipRow(p.color, p.seriesName, `${fmtSigned(p.value[1], 1)} °C`);
          return html;
        },
      },
      xAxis: {
        type: "value", min: 0, max: 365, interval: 365 / 12,
        axisLine: { lineStyle: { color: k.border } }, axisTick: { show: false }, splitLine: { show: false },
        axisLabel: {
          color: k.muted, fontSize: 11,
          formatter: (v) => {
            const i = monthStarts.findIndex((m) => Math.abs(m - v) < 16);
            return i >= 0 ? MONTHS[i] : "";
          },
        },
      },
      yAxis: valueAxis(k, { unit: "°" }),
      series,
    };
  });
}

compareChart();
outlookChart("chart-outlook");
