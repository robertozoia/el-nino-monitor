// Learn page controller: lessons, panel tabs, experiment controls, callouts with leader
// lines, progress, and the isometric stage (learn-scenes.js).

import { createStage, pacificModel, peruModel, lonLabel } from "./learn-scenes.js";

const T = window.I18N || {};
const tr = (k) => T[k] ?? k;
const LIVE = window.LIVE || {};
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const clamp = (v, a = 0, b = 1) => Math.max(a, Math.min(b, v));
const signed = (v, d = 1) => (v > 0 ? "+" : v < 0 ? "−" : "") + Math.abs(v).toFixed(d);
const deg = (v) => `${v.toFixed(1)} °C`;

const enso = (v) => v >= 2 ? "very_strong" : v >= 1.5 ? "strong" : v >= 1 ? "moderate" : v >= 0.5 ? "weak" : v > -0.5 ? "neutral" : "la_nina";
const icen = (v) => v >= 3 ? "extraordinary" : v >= 1.7 ? "strong" : v >= 1 ? "moderate" : v >= 0.4 ? "weak" : v > -1 ? "neutral" : "cold";

// ------------------------------------------------------------------ lessons from the DOM

const lessons = $$("#lesson-source article.lesson").map((a) => ({
  id: a.dataset.id,
  scene: a.dataset.scene,
  thumb: a.dataset.thumb,
  title: $("h2", a).innerHTML,
  short: $(".short", a).textContent,
  tabs: Object.fromEntries($$("[data-tab]", a).map((s) => [s.dataset.tab, s.innerHTML])),
  experiment: $("[data-part=experiment]", a)?.innerHTML ?? "",
  callouts: $$("[data-part=callouts] li", a).map((li) => ({ id: li.dataset.id, icon: li.dataset.icon, html: li.innerHTML })),
}));

const liveN34 = LIVE.regions?.nino34_weekly?.value;
const liveN12 = LIVE.regions?.nino12_weekly?.value;
const nowState = () => {
  const e = liveN34 ?? 0;
  return { e, coastal: clamp((liveN12 ?? e) - 0.8 * e, -1, 4) };
};
const month = new Date().getMonth() + 1;

// Starting state of the scene for each lesson.
const PRESETS = {
  stage: () => ({ e: 0, wind: null, coastal: 0, kamp: 0, burst: 0 }),
  winds: () => ({ e: 0, wind: 1, coastal: 0, kamp: 0, burst: 0 }),
  kelvin: () => ({ e: 0, wind: null, coastal: 0, kamp: 0, kpos: 0.12, burst: 0 }),
  phases: () => ({ e: 0, wind: null, coastal: 0, kamp: 0, burst: 0 }),
  costero: () => ({ coastal: 0 }),
  measure: () => ({ mode: "regions", live: LIVE.regions || {} }),
  global: () => ({ mode: "impacts", season: month >= 5 && month <= 9 ? "jja" : "djf" }),
  history: () => ({ event: null }),
  now: () => ({ ...nowState(), wind: null, kamp: 0, burst: 0 }),
};
const FOCUS = { measure: "pacific" };

// Where each callout card sits relative to its anchor, as a fraction of the stage size.
const PLACE = {
  warm_pool: [-0.1, 0.34], convection: [-0.12, -0.16], trades: [0.02, -0.34], cold_tongue: [0.12, -0.26],
  thermocline: [0.1, 0.3], upwelling: [0.04, 0.26], fish: [0.16, -0.08], sea_level: [0.0, 0.3],
  westerly: [-0.04, -0.3], kelvin: [0.06, 0.26],
  humboldt: [-0.1, -0.24], desert: [0.18, 0.2], andes: [0.08, -0.28], storms: [-0.2, -0.12], anchoveta: [-0.06, 0.28],
  nino4: [-0.16, -0.24], nino34: [-0.02, -0.3], nino3: [0.04, 0.26], nino12: [0.16, 0.12],
  g_peru: [0.14, 0.16], g_indo: [-0.12, 0.22], g_us: [0.08, -0.26], g_africa: [-0.12, 0.22],
};

// ------------------------------------------------------------------ elements

const stageEl = $("#stage"), svg = $("#scene"), leaders = $("#leaders"), calloutBox = $("#callouts");
const tooltip = $("#tooltip"), infoBody = $("#info-body"), expBody = $("#exp-body"), strip = $("#lesson-strip");
const stage = createStage(svg);
let current = null, state = {}, tab = "overview", labelsOn = true, seqTimer = null;

let seen = new Set();
try { seen = new Set(JSON.parse(localStorage.getItem("learn-seen") || "[]")); } catch (e) { /* storage blocked */ }
const saveSeen = () => { try { localStorage.setItem("learn-seen", JSON.stringify([...seen])); } catch (e) { /* storage blocked */ } };

// ------------------------------------------------------------------ lesson strip

strip.innerHTML = lessons.map((l, i) =>
  `<button type="button" class="lesson-card" data-id="${l.id}"><svg class="thumb" aria-hidden="true"><use href="#${l.thumb}"/></svg>` +
  `<span>${i + 1}. ${l.short}</span><svg class="chev" aria-hidden="true"><use href="#i-chev"/></svg></button>`).join("");
strip.addEventListener("click", (ev) => {
  const b = ev.target.closest(".lesson-card");
  if (b) go(b.dataset.id);
});

function updateProgress() {
  $("#progress-count").textContent = `${seen.size}/${lessons.length}`;
  $("#progress-bar").style.width = `${(100 * seen.size) / lessons.length}%`;
  $$(".lesson-card", strip).forEach((b) => b.classList.toggle("seen", seen.has(b.dataset.id)));
}

// ------------------------------------------------------------------ tabs

$$(".tabs [data-tab]").forEach((b) => b.addEventListener("click", () => { tab = b.dataset.tab; renderInfo(); }));

function renderInfo() {
  $$(".tabs [data-tab]").forEach((b) => b.setAttribute("aria-selected", String(b.dataset.tab === tab)));
  infoBody.innerHTML = `<h3>${current.title}</h3>` + (current.tabs[tab] || "");
  infoBody.scrollTop = 0;
}

// ------------------------------------------------------------------ navigation

function go(id, { push = true } = {}) {
  const lesson = lessons.find((l) => l.id === id) || lessons[0];
  stopSequence();
  current = lesson;
  state = PRESETS[lesson.id]();
  if (lesson.scene === "pacific") state.layers = { atmo: true, sst: true, thermo: true };
  seen.add(lesson.id);
  saveSeen();
  updateProgress();
  $$(".lesson-card", strip).forEach((b) => {
    const on = b.dataset.id === lesson.id;
    b.setAttribute("aria-current", String(on));
    if (on) b.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  });
  if (push && location.hash !== `#${lesson.id}`) history.replaceState(null, "", `#${lesson.id}`);
  document.title = `${$(".short", $(`article[data-id="${lesson.id}"]`)).textContent} · ${tr("learn_title")}`;
  $("#scene-title").textContent = lesson.short;

  renderInfo();
  expBody.innerHTML = lesson.experiment;
  bindControls();
  buildCallouts();
  stage.show(lesson.scene, FOCUS[lesson.id]);
  if (lesson.scene === "history") loadHistory();
  draw();
}

function step(delta) {
  const i = lessons.indexOf(current);
  go(lessons[(i + delta + lessons.length) % lessons.length].id);
}
$("#btn-prev").addEventListener("click", () => step(-1));
$("#btn-next").addEventListener("click", () => step(1));
document.addEventListener("keydown", (ev) => {
  if (ev.target.closest("input, textarea, select") || ev.altKey || ev.metaKey || ev.ctrlKey) return;
  if (ev.key === "ArrowRight") step(1);
  if (ev.key === "ArrowLeft") step(-1);
});
window.addEventListener("hashchange", () => {
  const id = location.hash.slice(1);
  if (id && id !== current?.id) go(id, { push: false });
});

// ------------------------------------------------------------------ drawing

function draw() {
  stage.render(state);
  updateReadouts();
  queueLayout();
}

// Smoothly moves numeric state fields to new values.
let tweenId = 0;
function tween(target, dur = 700) {
  const id = ++tweenId, from = { ...state }, t0 = performance.now();
  const keys = Object.keys(target).filter((k) => typeof target[k] === "number");
  for (const k of keys) if (typeof from[k] !== "number") from[k] = k === "wind" ? pacificModel(state).wind : 0;
  Object.assign(state, Object.fromEntries(Object.entries(target).filter(([k]) => !keys.includes(k))));
  const frame = (now) => {
    if (id !== tweenId) return;
    const k = clamp((now - t0) / dur), e = 1 - (1 - k) ** 3;
    for (const key of keys) state[key] = from[key] + (target[key] - from[key]) * e;
    draw();
    if (k < 1) requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

function out(name, text, cls) {
  $$(`[data-out="${name}"]`, expBody).forEach((el) => {
    el.innerHTML = text;
    el.classList.remove("warm", "cool");
    if (cls) el.classList.add(cls);
  });
}
const level = (v, keys, cuts) => tr(keys[cuts.findIndex((c) => v < c) === -1 ? keys.length - 1 : cuts.findIndex((c) => v < c)]);
const warmCls = (v, cut = 0.4) => (v >= cut ? "warm" : v <= -cut ? "cool" : null);

function updateReadouts() {
  if (!current) return;
  if (current.scene === "pacific") {
    const m = pacificModel(state);
    const w = m.sst(0.08, 0), e = m.sst(0.97, -0.25);
    out("sst_west", deg(w));
    out("sst_east", deg(e), e > 25.5 ? "warm" : "cool");
    out("sst_diff", deg(w - e));
    out("wind", `${Math.round(m.wind * 100)} %`);
    out("thermo_east", `${Math.round(m.he * 10)} m`);
    out("upwelling", level(m.upwelling, ["learn_lvl_none", "learn_lvl_weak", "learn_lvl_normal", "learn_lvl_strong"], [0.15, 0.7, 1.2]));
    out("fish", level(m.upwelling, ["learn_fish_few", "learn_fish_some", "learn_fish_many"], [0.35, 0.75]));
    out("enso", `${signed(m.e)} °C`, warmCls(m.e, 0.5));
    out("cat_noaa", tr(`cat_${enso(m.e)}`), warmCls(m.e, 0.5));
    out("rain_lon", lonLabel(120 + m.xr * 160));
  } else if (current.scene === "peru") {
    const m = peruModel(state);
    out("coastal", `${signed(m.coastal)} °C`, warmCls(m.coastal));
    out("sst_piura", deg(m.piura), m.piura > 25.8 ? "warm" : null);
    out("cat_enfen", tr(`cat_${icen(m.coastal)}`), warmCls(m.coastal));
    out("rain", level(m.rain, ["learn_rain_none", "learn_rain_light", "learn_rain_heavy", "learn_rain_extreme"], [0.05, 0.3, 0.8]), m.rain > 0.3 ? "warm" : null);
    out("fish", level(m.fishLevel, ["learn_fish_few", "learn_fish_some", "learn_fish_many"], [0.35, 0.7]));
  }
  const an = state.anomaly;
  if (typeof an === "number") {
    out("anomaly", `${signed(an)} °C`, warmCls(an));
    out("cat_noaa", tr(`cat_${enso(an)}`), warmCls(an, 0.5));
    out("cat_enfen", tr(`cat_${icen(an)}`), warmCls(an));
  }
}

// ------------------------------------------------------------------ controls

function bindControls() {
  if (current.id === "measure") state.anomaly = 1.5;
  for (const el of $$("[data-control]", expBody)) {
    const c = el.dataset.control;
    if (c === "layer") {
      el.addEventListener("change", () => { state.layers = { ...state.layers, [el.value]: el.checked }; draw(); });
    } else if (c === "wind") {
      el.addEventListener("input", () => { state.wind = el.value / 100; draw(); });
    } else if (c === "enso") {
      el.addEventListener("input", () => { state.e = Number(el.value); state.wind = null; syncSeg("phase", state.e); draw(); });
    } else if (c === "coastal") {
      el.addEventListener("input", () => { state.coastal = Number(el.value); draw(); });
    } else if (c === "anomaly") {
      el.addEventListener("input", () => { state.anomaly = Number(el.value); updateReadouts(); });
    } else if (c === "burst") {
      el.addEventListener("click", runSequence);
    } else if (el.classList.contains("seg") || el.classList.contains("chips")) {
      el.addEventListener("click", (ev) => {
        const b = ev.target.closest("button");
        if (!b) return;
        $$("button", el).forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
        onChoice(c, b.value);
      });
    }
  }
  if (current.id === "now") {
    const n34 = $('[data-control="nowmode"]', expBody);
    if (liveN34 == null && n34) n34.hidden = true;
  }
  if (current.id === "global") syncSeg("season", state.season);
}

function syncSeg(control, value) {
  const el = $(`[data-control="${control}"]`, expBody);
  if (!el) return;
  const buttons = $$("button", el);
  const target = control === "phase"
    ? buttons.reduce((best, b) => Math.abs(b.value - value) < Math.abs(best.value - value) ? b : best)
    : buttons.find((b) => b.value === value);
  buttons.forEach((b) => b.setAttribute("aria-pressed", String(b === target && (control !== "phase" || Math.abs(b.value - value) < 0.35))));
}

function onChoice(control, value) {
  if (control === "phase") {
    const v = Number(value);
    const range = $('[data-control="enso"]', expBody);
    if (range) range.value = v;
    state.wind = null;
    tween({ e: v }, 800);
  } else if (control === "season") {
    state.season = value;
    draw();
  } else if (control === "event") {
    selectEvent(Number(value));
  } else if (control === "nowmode") {
    tween(value === "live" ? nowState() : { e: 0, coastal: 0 }, 900);
  }
}

$("#exp-reset").addEventListener("click", () => go(current.id));

// Westerly wind burst → Kelvin wave → warming in the east (lesson 3).
function runSequence() {
  stopSequence();
  const btn = $('[data-control="burst"]', expBody);
  if (btn) btn.disabled = true;
  Object.assign(state, PRESETS.kelvin());
  const dur = 9000, t0 = performance.now();
  const steps = $$(".steps li", expBody);
  const frame = (now) => {
    const k = clamp((now - t0) / dur);
    state.burst = k < 0.08 ? k / 0.08 : clamp(1 - (k - 0.3) / 0.15);
    state.kpos = 0.12 + 0.86 * clamp((k - 0.08) / 0.62);
    state.kamp = 7 * clamp((k - 0.04) / 0.1) * (1 - clamp((k - 0.78) / 0.22));
    state.e = 1.7 * smooth(0.62, 1, k);
    const s = k < 0.04 ? 0 : k < 0.2 ? 1 : k < 0.68 ? 2 : 3;
    steps.forEach((li, i) => { li.classList.toggle("on", i === s); li.classList.toggle("done", i < s); });
    const tl = $('[data-out="timeline"]', expBody);
    if (tl) tl.style.width = `${k * 100}%`;
    out("month", `${tr("learn_month")} ${Math.floor(k * 3.99)}`);
    draw();
    if (k < 1) seqTimer = requestAnimationFrame(frame);
    else if (btn) btn.disabled = false;
  };
  seqTimer = requestAnimationFrame(frame);
}
function stopSequence() { cancelAnimationFrame(seqTimer); seqTimer = null; tweenId++; }
const smooth = (a, b, x) => { const t = clamp((x - a) / (b - a)); return t * t * (3 - 2 * t); };

// ------------------------------------------------------------------ history data

let historyLoaded = false;
async function loadHistory() {
  if (historyLoaded) return;
  const hist = stage.scenes.history;
  try {
    const r = await fetch("/api/series/oni");
    if (!r.ok) throw new Error(String(r.status));
    const d = await r.json();
    const byT = new Map(d.points.map(([t, v]) => [t.slice(0, 7), v]));
    const bars = [];
    for (let y = 1950; y <= 2025; y++) {
      const v = byT.get(`${y + 1}-01`);
      if (v != null) bars.push({ year: y, v, label: `${y}–${String(y + 1).slice(2)}` });
    }
    const last = d.points[d.points.length - 1];
    if (last && last[0] >= "2026-06") bars.push({ year: 2026, v: last[1], label: "2026–27*", partial: last[0].slice(0, 7) });
    hist.setData(bars);
    historyLoaded = true;
  } catch (e) {
    hist.setMessage(tr("learn_no_history"));
  }
  if (current?.scene === "history") { stage.show("history"); draw(); }
}

function selectEvent(year) {
  state.event = year;
  $$(".event-notes [data-event]", expBody).forEach((p) => (p.hidden = Number(p.dataset.event) !== year));
  const chip = $(`[data-control="event"] button[value="${year}"]`, expBody);
  $$('[data-control="event"] button', expBody).forEach((b) => b.setAttribute("aria-pressed", String(b === chip)));
  const note = $(`.event-notes [data-event="${year}"]`, expBody);
  const bar = barByYear(year);
  current.callouts = note && bar
    ? [{ id: `ev_${year}`, icon: bar.v >= 0 ? "thermo" : "wave", html: `<b>${bar.label}</b><span>ONI ${signed(bar.v)}&nbsp;°C · ${tr(`cat_${enso(bar.v)}`)}</span>`, place: [0.06, -0.24] }]
    : [];
  buildCallouts();
  draw();
}
function barByYear(year) {
  for (let i = 0; ; i++) { const b = stage.scenes.history.bar(i); if (!b) return null; if (b.year === year) return b; }
}

// ------------------------------------------------------------------ callouts

function buildCallouts() {
  calloutBox.innerHTML = "";
  current.callouts.forEach((c, i) => {
    const card = document.createElement("div");
    card.className = "callout";
    card.dataset.id = c.id;
    card.innerHTML = `<svg class="ic" aria-hidden="true"><use href="#i-${c.icon || "pulse"}"/></svg><div>${c.html}</div>`;
    const liveKey = $("[data-live]", card)?.dataset.live;
    const lv = liveKey && LIVE.regions?.[liveKey];
    if (lv) card.querySelector("div").insertAdjacentHTML("beforeend", `<span class="live">${lv.label}&nbsp;°C · ${lv.date}</span>`);
    const dot = document.createElement("button");
    dot.type = "button";
    dot.className = "callout-dot";
    dot.textContent = String(i + 1);
    dot.setAttribute("aria-label", card.querySelector("b")?.textContent || c.id);
    dot.addEventListener("click", () => {
      const open = !card.classList.contains("open");
      $$(".callout", calloutBox).forEach((x) => x.classList.remove("open"));
      card.classList.toggle("open", open);
      queueLayout();
    });
    calloutBox.append(card, dot);
  });
}

let layoutQueued = false;
function queueLayout() {
  if (layoutQueued) return;
  layoutQueued = true;
  requestAnimationFrame(() => { layoutQueued = false; layoutCallouts(); });
}

function toStage(p) {
  const m = svg.getScreenCTM();
  if (!m) return null;
  const r = stageEl.getBoundingClientRect();
  const q = new DOMPoint(p[0], p[1]).matrixTransform(m);
  return [q.x - r.left, q.y - r.top];
}

function layoutCallouts() {
  const W = stageEl.clientWidth, H = stageEl.clientHeight;
  const compact = W < 700;
  stageEl.classList.toggle("compact", compact);
  // Keep scene text a readable size whatever the zoom.
  const vb = svg.viewBox.baseVal;
  if (vb && vb.width) svg.style.setProperty("--u", String(clamp(Math.max(vb.width / W, vb.height / H), 0.25, 2.4).toFixed(3)));
  let paths = "";
  const placed = [];
  const cards = $$(".callout", calloutBox);
  for (const card of cards) {
    const c = current.callouts.find((x) => x.id === card.dataset.id);
    const dot = card.nextElementSibling;
    const a = stage.anchor(card.dataset.id);
    const p = a && toStage(a);
    if (!p) { card.classList.add("hidden"); dot.style.display = "none"; continue; }
    card.classList.remove("hidden");
    dot.style.display = "";
    dot.style.left = `${p[0]}px`;
    dot.style.top = `${p[1]}px`;
    const cw = card.offsetWidth, ch = card.offsetHeight;
    const [fx, fy] = c.place || PLACE[c.id] || [0.1, -0.2];
    let x = p[0] + fx * W - cw / 2, y = p[1] + fy * H - ch / 2;
    if (compact) { x = p[0] - cw / 2; y = p[1] + (p[1] > H / 2 ? -ch - 22 : 22); }
    x = clamp(x, 10, W - cw - 10);
    y = clamp(y, 10, H - ch - 56);
    // Nudge down past cards already placed.
    for (let guard = 0; guard < 8; guard++) {
      const hit = placed.find((r) => x < r[0] + r[2] + 6 && x + cw + 6 > r[0] && y < r[1] + r[3] + 6 && y + ch + 6 > r[1]);
      if (!hit) break;
      y = hit[1] + hit[3] + 8;
      if (y + ch > H - 56) { y = clamp(hit[1] - ch - 8, 10, H - ch - 56); x = clamp(x + (p[0] < W / 2 ? -40 : 40), 10, W - cw - 10); }
    }
    placed.push([x, y, cw, ch]);
    card.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
    // Elbow leader: vertical from the anchor, then horizontal into the card's side.
    const ex = p[0] < x ? x : p[0] > x + cw ? x + cw : null;
    const ey = clamp(p[1], y + 12, y + ch - 12);
    if (ex == null) {
      const yy = p[1] < y ? y : y + ch;
      paths += `<path d="M${p[0]},${p[1]}V${yy}"/>`;
    } else {
      paths += `<path d="M${p[0]},${p[1]}V${ey}H${ex}"/>`;
    }
    paths += `<circle cx="${p[0]}" cy="${p[1]}" r="4.5"/>`;
  }
  leaders.innerHTML = paths;
}

svg.addEventListener("camera", queueLayout);
new ResizeObserver(queueLayout).observe(stageEl);

$("#btn-labels").addEventListener("click", (ev) => {
  labelsOn = !labelsOn;
  ev.currentTarget.setAttribute("aria-pressed", String(labelsOn));
  calloutBox.classList.toggle("off", !labelsOn);
  leaders.classList.toggle("off", !labelsOn);
});
$("#btn-full").addEventListener("click", () => {
  if (document.fullscreenElement) document.exitFullscreen();
  else stageEl.requestFullscreen?.().catch(() => {});
});

// ------------------------------------------------------------------ hover

function svgPoint(ev) {
  const m = svg.getScreenCTM();
  return m ? new DOMPoint(ev.clientX, ev.clientY).matrixTransform(m.inverse()) : null;
}
function showTip(ev, html) {
  if (!html) { tooltip.hidden = true; return; }
  const r = stageEl.getBoundingClientRect();
  tooltip.innerHTML = html;
  tooltip.hidden = false;
  const x = ev.clientX - r.left, y = ev.clientY - r.top;
  const w = tooltip.offsetWidth, h = tooltip.offsetHeight;
  tooltip.style.left = `${clamp(x + 14, 6, r.width - w - 6)}px`;
  tooltip.style.top = `${clamp(y - h - 12, 6, r.height - h - 6)}px`;
}
svg.addEventListener("pointermove", (ev) => {
  hideHint();
  const bar = ev.target.closest?.(".bar");
  if (bar) {
    const b = stage.scenes.history.bar(Number(bar.dataset.i));
    const note = b.partial ? `<br>${tr("js_current")}: ${b.partial}` : "";
    return showTip(ev, `<b>${b.label}</b><br>ONI ${signed(b.v)}&nbsp;°C · ${tr(`cat_${enso(b.v)}`)}${note}`);
  }
  const imp = ev.target.closest?.(".impact");
  if (imp) return showTip(ev, `<b>${imp.dataset.name}</b><br>${tr(`js_typical_${imp.dataset.kind}`)}`);
  const box = ev.target.closest?.(".region-box");
  if (box) {
    const lv = LIVE.regions?.[`${box.dataset.region}_weekly`];
    const name = { nino4: "Niño 4", nino34: "Niño 3.4", nino3: "Niño 3", nino12: "Niño 1+2" }[box.dataset.region];
    return showTip(ev, `<b>${name}</b>${lv ? `<br>${lv.label}&nbsp;°C · ${lv.date}` : ""}`);
  }
  const p = svgPoint(ev);
  showTip(ev, p && stage.hit(p.x, p.y));
});
svg.addEventListener("pointerleave", () => { tooltip.hidden = true; });
svg.addEventListener("click", (ev) => {
  const bar = ev.target.closest?.(".bar");
  if (bar && current.id === "history") {
    const b = stage.scenes.history.bar(Number(bar.dataset.i));
    selectEvent(b.year);
  }
});

const hint = $("#stage-hint");
let hintTimer = setTimeout(hideHint, 7000);
function hideHint() { hint.classList.add("gone"); clearTimeout(hintTimer); }

// ------------------------------------------------------------------ theme

const isDark = () => {
  const f = document.documentElement.dataset.theme;
  return f ? f === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
};
$("#theme-toggle").addEventListener("click", () => {
  const next = isDark() ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  try { localStorage.setItem("theme", next); } catch (e) { /* storage blocked */ }
  stage.refreshPalette();
  draw();
});
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => { stage.refreshPalette(); draw(); });

// ------------------------------------------------------------------ start

go(location.hash.slice(1) || lessons[0].id, { push: false });
const splash = $("#splash");
Promise.race([document.fonts?.ready ?? Promise.resolve(), new Promise((r) => setTimeout(r, 1200))]).then(() => {
  queueLayout();
  setTimeout(() => splash.classList.add("done"), 250);
});
