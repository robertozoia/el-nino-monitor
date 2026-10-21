// Isometric SVG scenes for the Learn page: the equatorial Pacific (ocean + Walker
// circulation), Peru's coast, a world map plate and a history "skyline". Everything is
// schematic: the models below are tuned to look right, not to simulate the ocean.

const T = window.I18N || {};
const tr = (k) => T[k] ?? k;

// ------------------------------------------------------------------ geometry
// Scene axes: X east (or offshore in the Peru scene), Y up, Z north.
const UX = [0.94, 0.30];
const VZ = [0.60, -0.40];

function projector(S, ux = UX, vz = VZ) {
  const P = (x, y, z) => [S * (x * ux[0] + z * vz[0]), S * (x * ux[1] + z * vz[1] - y)];
  // Inverse on a horizontal plane at height y.
  const inv = (sx, sy, y = 0) => {
    const a = sx / S, b = sy / S + y;
    const det = ux[0] * vz[1] - vz[0] * ux[1];
    return [(vz[1] * a - vz[0] * b) / det, (-ux[1] * a + ux[0] * b) / det];
  };
  return { P, inv, S };
}

const f2 = (n) => Math.round(n * 100) / 100;
const pt = (p) => `${f2(p[0])},${f2(p[1])}`;
const poly = (P, list) => list.map((q) => pt(P(...q))).join(" ");
const clamp = (v, a = 0, b = 1) => Math.max(a, Math.min(b, v));
const smooth = (a, b, x) => { const t = clamp((x - a) / (b - a)); return t * t * (3 - 2 * t); };
const gauss = (x, mu, s) => Math.exp(-(((x - mu) / s) ** 2));

// ------------------------------------------------------------------ colour

function parseColor(c) {
  c = c.trim();
  if (c.startsWith("#")) {
    if (c.length === 4) c = "#" + [...c.slice(1)].map((x) => x + x).join("");
    return [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16));
  }
  const m = c.match(/[\d.]+/g);
  return m ? m.slice(0, 3).map(Number) : [128, 128, 128];
}
const mix = (a, b, t) => a.map((v, i) => Math.round(v + (b[i] - v) * clamp(t)));
const rgb = (c) => `rgb(${c[0]},${c[1]},${c[2]})`;
let PAL = null;
export function refreshPalette() {
  const cs = getComputedStyle(document.documentElement);
  const g = (n) => parseColor(cs.getPropertyValue(n) || "#888");
  PAL = { cool: g("--sst-cool"), mid: g("--sst-mid"), warm: g("--sst-warm"), deep: g("--water-deep"), navy: g("--navy"), bg: g("--bg") };
}
/** Absolute sea temperature (°C) → colour. */
export function sstColor(t) {
  return t < 25.5 ? mix(PAL.cool, PAL.mid, (t - 18.5) / 7) : mix(PAL.mid, PAL.warm, (t - 25.5) / 4.5);
}
/** Anomaly (°C) → colour on the same diverging scale. */
export function anomColor(a) {
  return a < 0 ? mix(PAL.mid, PAL.cool, -a / 2) : mix(PAL.mid, PAL.warm, a / 2.6);
}
const darken = (c, k) => mix(c, [0, 0, 0], k);
const lighten = (c, k) => mix(c, [255, 255, 255], k);

// ------------------------------------------------------------------ glyphs

function arrow(a, b, cls, width, color, head = 1) {
  const dx = b[0] - a[0], dy = b[1] - a[1];
  const len = Math.hypot(dx, dy) || 1;
  const ux = dx / len, uy = dy / len;
  const h = Math.min(len * 0.55, (5 + width * 1.6) * head);
  const base = [b[0] - ux * h, b[1] - uy * h];
  const w = h * 0.55;
  return `<path class="arrow ${cls}" d="M${pt(a)}L${pt(base)}" stroke="${color}" stroke-width="${width}"/>` +
    `<path d="M${pt(b)}L${f2(base[0] - uy * w)},${f2(base[1] + ux * w)}L${f2(base[0] + uy * w)},${f2(base[1] - ux * w)}Z" fill="${color}"/>`;
}

function cloud(x, y, r, extra = "") {
  const d = `M${f2(x - 1.7 * r)},${f2(y + 0.55 * r)}A${f2(0.62 * r)},${f2(0.62 * r)} 0 0 1 ${f2(x - 1.05 * r)},${f2(y - 0.25 * r)}` +
    `A${f2(0.85 * r)},${f2(0.85 * r)} 0 0 1 ${f2(x + 0.15 * r)},${f2(y - 0.78 * r)}` +
    `A${f2(0.75 * r)},${f2(0.75 * r)} 0 0 1 ${f2(x + 1.15 * r)},${f2(y - 0.1 * r)}` +
    `A${f2(0.6 * r)},${f2(0.6 * r)} 0 0 1 ${f2(x + 1.7 * r)},${f2(y + 0.55 * r)}Z`;
  return `<path class="cloud" d="${d}" ${extra}/>`;
}

function storm(x, y, r, rain, dropTo) {
  // Towering cumulonimbus: two stacked cauliflower blocks under a flat anvil.
  let s = `<path class="cloud" d="M${f2(x - 2.3 * r)},${f2(y - 2.2 * r)}Q${f2(x)},${f2(y - 2.75 * r)} ${f2(x + 2.4 * r)},${f2(y - 2.25 * r)}L${f2(x + 1.2 * r)},${f2(y - 1.75 * r)}L${f2(x - 1.1 * r)},${f2(y - 1.75 * r)}Z"/>`;
  s += cloud(x + 0.1 * r, y - 1.15 * r, r * 0.85);
  s += cloud(x, y, r * 1.15);
  if (rain > 0.02) {
    const n = Math.round(4 + rain * 8);
    s += `<g class="rain" opacity="${f2(clamp(0.35 + rain * 0.6))}">`;
    for (let i = 0; i < n; i++) {
      const lx = x - 1.5 * r + (3 * r * i) / (n - 1 || 1);
      const top = y + 0.75 * r;
      const bot = Math.min(dropTo, top + 6 + rain * 22);
      for (let yy = top; yy < bot - 3; yy += 9) {
        s += `<line x1="${f2(lx)}" y1="${f2(yy)}" x2="${f2(lx - 2)}" y2="${f2(yy + 5)}" style="animation-delay:${f2(((i * 37) % 10) / 14)}s"/>`;
      }
    }
    s += `</g>`;
  }
  return s;
}

function sun(x, y, r) {
  let s = `<g stroke="var(--sst-warm)" stroke-width="2" stroke-linecap="round" fill="none"><circle cx="${f2(x)}" cy="${f2(y)}" r="${f2(r)}" fill="var(--card-solid)"/>`;
  for (let i = 0; i < 8; i++) {
    const a = (i * Math.PI) / 4;
    s += `<line x1="${f2(x + Math.cos(a) * r * 1.5)}" y1="${f2(y + Math.sin(a) * r * 1.5)}" x2="${f2(x + Math.cos(a) * r * 2)}" y2="${f2(y + Math.sin(a) * r * 2)}"/>`;
  }
  return s + `</g>`;
}

function fish(x, y, s, flip = false) {
  const k = flip ? -1 : 1;
  return `<path class="fish" d="M${f2(x - s * k)},${f2(y)}Q${f2(x)},${f2(y - 0.6 * s)} ${f2(x + 0.7 * s * k)},${f2(y)}Q${f2(x)},${f2(y + 0.6 * s)} ${f2(x - s * k)},${f2(y)}ZM${f2(x + 0.6 * s * k)},${f2(y)}L${f2(x + 1.2 * s * k)},${f2(y - 0.45 * s)}L${f2(x + 1.2 * s * k)},${f2(y + 0.45 * s)}Z"/>`;
}

function label(p, text, cls = "lbl", anchor = "middle", dy = 0) {
  return `<text class="${cls}" x="${f2(p[0])}" y="${f2(p[1] + dy)}" text-anchor="${anchor}">${text}</text>`;
}

function lonLabel(lon) {
  lon = ((lon % 360) + 360) % 360;
  if (Math.abs(lon - 180) < 0.5) return "180°";
  return lon < 180 ? `${Math.round(lon)}°${tr("learn_lon_e")}` : `${Math.round(360 - lon)}°${tr("learn_lon_w")}`;
}
export { lonLabel };

// ================================================================== Pacific

const PAC = { L: 160, W: 46, D: 30, AT: 38, S: 4.4 };

/** The schematic ocean–atmosphere model shared by rendering and readouts. */
export function pacificModel(st) {
  const e = st.e ?? 0;
  const wind = st.wind ?? clamp(1 - 0.33 * e, 0.05, 1.7);
  const tilt = wind;
  const hw = 12.5 + 7.5 * tilt;
  const he = Math.max(1.5, 12.5 - 7.5 * tilt);
  const kamp = st.kamp ?? 0, kpos = st.kpos ?? 0;
  const coastal = st.coastal ?? 0;
  const thermo = (x) => hw + (he - hw) * Math.pow(x, 1.6) + kamp * gauss(x, kpos, 0.07);
  const sst = (x, zq) => {
    const warm = 29.7 - 0.9 * x - 0.45 * Math.max(e, 0) * (1 - smooth(0.15, 0.4, x));
    const tongue = Math.pow(wind, 0.8) * 6.2 * smooth(0.28, 1, x) * Math.exp(-((zq + 0.2) ** 2) / 0.28);
    const coast = Math.pow(wind, 0.8) * 2.6 * gauss(1 - x, 0, 0.08) * (zq < 0.3 ? 1 : 0.4);
    const anom = (0.55 * e * gauss(x, 0.58, 0.32) + 0.55 * Math.max(e, 0) * gauss(x, 0.97, 0.12)) * Math.exp(-(zq * zq) / 0.45);
    const extra = coastal * gauss(1 - x, 0, 0.13) * (zq < 0.3 ? 1 : 0.5);
    const kel = 0.35 * kamp / 6 * gauss(x, kpos, 0.08) * Math.exp(-(zq * zq) / 0.2);
    return warm - tongue - coast + anom + extra + kel;
  };
  const xr = clamp(0.09 + 0.21 * e, 0.04, 0.68);   // rising branch (heaviest rain)
  const upwelling = clamp(wind * (1 - 0.6 * smooth(0.6, 1, kpos) * kamp / 6), 0, 1.6);
  return { e, wind, hw, he, thermo, sst, xr, upwelling, coastal };
}

function makePacific(root) {
  const { L, W, D, AT, S } = PAC;
  const { P, inv } = projector(S);
  const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
  g.dataset.scene = "pacific";
  root.appendChild(g);
  let model = null, st = {};
  const XL = (lon) => (lon - 120) / 160 * L;

  function box(x0, x1, z0, z1, y0, y1, top = "land-top", side = "land-side", side2 = "land-side2") {
    return `<polygon class="${side}" points="${poly(P, [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]])}"/>` +
      `<polygon class="${side2}" points="${poly(P, [[x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0]])}"/>` +
      `<polygon class="${top}" points="${poly(P, [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]])}"/>`;
  }

  function walkerLoop(x0, x1, strength, reverse) {
    // Loop in the vertical plane over the equator; anticlockwise seen from the south
    // (rising at x0, eastward aloft, sinking at x1, westward at the surface).
    const cx = (x0 + x1) / 2, rx = Math.abs(x1 - x0) / 2, cy = 19, ry = 15;
    const pts = [];
    for (let i = 0; i <= 72; i++) {
      const th = Math.PI - (i / 72) * 2 * Math.PI;
      const c = Math.cos(th), s = Math.sin(th);
      const X = cx + (reverse ? -1 : 1) * rx * Math.sign(c) * Math.pow(Math.abs(c), 0.55);
      const Y = cy + ry * Math.sign(s) * Math.pow(Math.abs(s), 0.55);
      pts.push(P(X, Y, W / 2));
    }
    const width = 1.2 + 2.6 * strength;
    const col = "var(--navy-2)";
    let s = `<path class="arrow flow" d="M${pts.map(pt).join("L")}" stroke="${col}" stroke-width="${f2(width)}" opacity="${f2(clamp(0.3 + strength * 0.6))}"/>`;
    for (const i of [14, 32, 50, 68]) s += arrow(pts[i - 3], pts[i], "", 0.01, col, 0.9 + strength * 0.5);
    return s;
  }

  function render(state) {
    st = state;
    const m = (model = pacificModel(state));
    const layers = state.layers || { atmo: true, sst: true, thermo: true };
    let s = "";

    // Gradient for the warm layer on the equatorial cross-section.
    const a = P(0, 0, 0), b = P(L, 0, 0);
    s += `<defs><linearGradient id="g-warm" gradientUnits="userSpaceOnUse" x1="${f2(a[0])}" y1="${f2(a[1])}" x2="${f2(b[0])}" y2="${f2(b[1])}">`;
    for (let i = 0; i <= 10; i++) s += `<stop offset="${i / 10}" stop-color="${rgb(sstColor(layers.sst ? m.sst(i / 10, 0) : 26))}"/>`;
    s += `</linearGradient><linearGradient id="g-deep" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--water-deep-2)"/><stop offset="1" stop-color="var(--water-deep)"/></linearGradient></defs>`;

    // West: Australia and the Indonesian islands (behind the ocean).
    s += box(-34, -12, 0, 16, -D, 1.6);
    s += box(-24, -12, 22, 28, -3, 1.2) + box(-16, -6, 30, 35, -3, 1.2) + box(-30, -20, 32, 38, -3, 1.2);
    s += label(P(-23, 1.6, 8), tr("learn_s_australia"), "lbl-strong", "middle", -4);
    s += label(P(-16, 1.2, 34), tr("learn_s_indonesia"), "lbl", "middle", -10);

    // Ocean top: sea temperature cells.
    const nx = 40, nz = 9;
    s += `<g class="sst-cells">`;
    for (let j = nz - 1; j >= 0; j--) {
      for (let i = 0; i < nx; i++) {
        const x0 = (i / nx) * L, x1 = ((i + 1) / nx) * L, z0 = (j / nz) * W, z1 = ((j + 1) / nz) * W;
        const zq = ((z0 + z1) / 2 - W / 2) / (W / 2);
        const c = layers.sst ? sstColor(m.sst((i + 0.5) / nx, zq)) : lighten(PAL.mid, 0.1);
        s += `<polygon class="cell" fill="${rgb(c)}" stroke="${rgb(c)}" stroke-width=".6" points="${poly(P, [[x0, 0, z0], [x1, 0, z0], [x1, 0, z1], [x0, 0, z1]])}"/>`;
      }
    }
    s += `</g>`;
    // Equator line on the surface.
    s += `<path d="M${pt(P(0, 0, W / 2))}L${pt(P(L, 0, W / 2))}" stroke="var(--navy)" stroke-width="1" stroke-dasharray="2 5" opacity=".55"/>`;

    // Front face: equatorial cross-section, warm layer above the thermocline.
    const N = 48;
    const top = [], tl = [];
    for (let i = 0; i <= N; i++) {
      const x = i / N;
      top.push([x * L, 0, 0]);
      tl.push([x * L, -m.thermo(x), 0]);
    }
    s += `<polygon class="deep" points="${poly(P, [[0, 0, 0], [L, 0, 0], [L, -D, 0], [0, -D, 0]])}"/>`;
    if (layers.thermo) {
      s += `<polygon fill="url(#g-warm)" points="${poly(P, top.concat(tl.slice().reverse()))}"/>`;
      s += `<path class="thermo-line" d="M${tl.map((q) => pt(P(...q))).join("L")}"/>`;
      s += label(P(L * 0.3, -m.thermo(0.3), 0), tr("learn_s_thermocline"), "lbl lbl-in", "middle", 16);
    } else {
      s += `<polygon fill="url(#g-warm)" opacity=".5" points="${poly(P, [[0, 0, 0], [L, 0, 0], [L, -4, 0], [0, -4, 0]])}"/>`;
    }
    s += `<polygon class="edge" points="${poly(P, [[0, 0, 0], [L, 0, 0], [L, -D, 0], [0, -D, 0]])}"/>`;
    s += `<polygon class="edge" points="${poly(P, [[0, 0, 0], [L, 0, 0], [L, 0, W], [0, 0, W]])}"/>`;

    // Depth ticks (1 unit = 10 m) and longitudes along the bottom edge.
    for (const dm of [0, 100, 200, 300]) {
      const q = P(0, -dm / 10, 0);
      s += `<path d="M${pt(q)}l-6,0" stroke="var(--muted)" stroke-width="1"/>` + label([q[0] - 9, q[1] + 3.5], `${dm} m`, "tick", "end");
    }
    for (const lon of [120, 150, 180, 210, 240, 270]) {
      const q = P(XL(lon), -D, 0);
      s += `<path d="M${pt(q)}l0,6" stroke="var(--muted)" stroke-width="1"/>` + label([q[0], q[1] + 17], lonLabel(lon), "tick");
    }
    s += label(P(L * 0.6, -D * 0.82, 0), tr("learn_s_equator"), "lbl lbl-in", "middle");

    // Upwelling arrows on the cross-section, near South America.
    if (m.upwelling > 0.05) {
      const op = f2(clamp(m.upwelling / 1.2, 0.15, 1));
      for (const [x, dx] of [[0.86, -3], [0.92, -2], [0.97, -1]]) {
        const pts = [];
        for (let k = 0; k <= 12; k++) {
          const t = k / 12;
          pts.push(P(x * L + dx * t * t, -(m.he + 7) * (1 - t) - 0.5 * t, 0));
        }
        s += `<g opacity="${op}"><path class="arrow flow-slow" d="M${pts.map(pt).join("L")}" stroke="var(--sst-cool)" stroke-width="3"/>` +
          arrow(pts[10], pts[12], "", 0.01, "var(--sst-cool)", 0.9) + `</g>`;
      }
    }

    // Kelvin wave marker.
    if ((state.kamp ?? 0) > 0.3) {
      const kx = (state.kpos ?? 0) * L;
      const q = P(kx, -m.thermo(state.kpos) - 2, 0);
      s += `<g opacity="${f2(clamp(state.kamp / 6))}">` + arrow([q[0] - 18, q[1] + 16], [q[0] + 22, q[1] + 16], "", 2.2, "var(--sst-warm)") + `</g>`;
    }

    // Fish near the coast of South America.
    const nf = Math.round(7 * clamp(m.upwelling / 1.1));
    for (let i = 0; i < nf; i++) {
      const q = P(L * (0.89 + 0.014 * (i % 4)) - (i % 2) * 3, 0, 3 + i * 2.4);
      s += fish(q[0], q[1], 7, true);
    }

    // East: South America with the Andes.
    const X0 = L, X1 = L + 24;
    s += box(X0, X1, 0, W, -D, 1.8);
    const ridge = (z) => [X0 + 9 + 1.5 * Math.sin(z * 0.25), 1.8 + 7 + 4 * Math.sin(z * 0.42 + 1) + 2 * Math.sin(z * 1.1), z];
    for (let z = W; z > 0; z -= 3) {
      const z2 = Math.max(0, z - 3);
      const r1 = ridge(z), r2 = ridge(z2);
      s += `<polygon class="land-top" points="${poly(P, [[X0 + 2, 1.8, z], r1, r2, [X0 + 2, 1.8, z2]])}"/>`;
      s += `<polygon class="land-side" points="${poly(P, [r1, [X0 + 18, 1.8, z], [X0 + 18, 1.8, z2], r2])}"/>`;
    }
    s += label(P(X0 + 14, 1.8, 6), tr("learn_s_samerica"), "lbl-strong", "middle", 18);
    s += label(P(X0 + 2, 1.8, 13), tr("learn_s_peru"), "lbl", "start", 4);

    // Atmosphere: Walker circulation, trade winds, storms.
    if (layers.atmo) {
      const xr = m.xr * L, xs = 0.93 * L;
      const strength = clamp(m.wind, 0.15, 1.6);
      s += walkerLoop(xr, xs, strength * 0.8, false);
      if (m.e > 0.5) s += walkerLoop(xr, 0.02 * L, clamp((m.e - 0.5) / 1.5) * 0.8, true);

      // Surface winds: easterly trades east of the rising branch; westerlies west of it in El Niño.
      const burst = state.burst ?? 0;
      for (const z of [W / 2 - 9, W / 2, W / 2 + 9]) {
        for (let x = 0.1; x < 0.92; x += 0.13) {
          let u = x > m.xr ? -m.wind : clamp(0.55 * m.e, 0, 1);
          if (burst > 0 && x < 0.42) u = burst * 1.3;
          if (Math.abs(u) < 0.08) continue;
          const len = 6 + 7 * Math.abs(u);
          const a1 = P(x * L - (Math.sign(u) * len) / 2, 0.8, z), b1 = P(x * L + (Math.sign(u) * len) / 2, 0.8, z);
          s += arrow(a1, b1, "flow", 1.6 + 1.2 * Math.abs(u), u > 0 ? "var(--sst-warm)" : "var(--navy)");
        }
      }

      // Rising air and storms over the warmest water; high pressure where air sinks.
      const c = P(xr, 24, W / 2);
      const surf = P(xr, 0, W / 2);
      s += storm(c[0], c[1], 17 + 7 * clamp(strength), clamp(0.5 + 0.3 * strength), surf[1]);
      s += label([surf[0], P(xr, 4, W / 2)[1]], tr("learn_s_low"), "hl", "middle");
      const hs = P(xs, 5, W / 2);
      s += label(hs, tr("learn_s_high"), "hl", "middle");
      if (m.e > 0.9) { const q = P(4, 16, W / 2 + 4); s += sun(q[0], q[1], 9); }
      // Rain on the Peruvian coast when the sea there gets hot enough.
      const tc = m.sst(0.97, -0.3);
      if (tc > 26) { const q = P(L - 6, 15, 9); s += storm(q[0], q[1], 13, clamp((tc - 26) / 2.5), P(L - 6, 0, 9)[1]); }
    }

    g.innerHTML = s;
  }

  const anchors = {
    warm_pool: () => P(L * 0.12, 0, W * 0.62),
    convection: () => { const q = P(model.xr * L, 24, W / 2); return [q[0], q[1] - 46]; },
    trades: () => P(L * 0.62, 0.8, W / 2 + 9),
    cold_tongue: () => P(L * 0.86, 0, W * 0.38),
    thermocline: () => P(L * 0.5, -model.thermo(0.5), 0),
    upwelling: () => P(L * 0.9, -model.he - 4, 0),
    fish: () => P(L * 0.92, 0, 6),
    sea_level: () => P(L * 0.04, 0, W * 0.2),
    westerly: () => P(L * 0.22, 0.8, W / 2),
    kelvin: () => P((st.kpos ?? 0.3) * L, -model.thermo(st.kpos ?? 0.3), 0),
  };

  function hit(ux, uy) {
    const [x, z] = inv(ux, uy, 0);
    if (x < 0 || x > L || z < 0 || z > W) return null;
    const xq = x / L, zq = (z - W / 2) / (W / 2);
    const lat = zq * 20;
    const t = model.sst(xq, zq);
    return `<b>${tr("learn_sea_temp")}</b><br>${t.toFixed(1)}&nbsp;°C · ${lonLabel(120 + xq * 160)} ${Math.abs(lat).toFixed(0)}°${lat >= 0 ? "N" : "S"}`;
  }

  function view() {
    const c = [[-36, -D - 7, 0], [L + 26, -D - 7, 0], [-36, AT + 6, W], [L + 26, AT, W], [L + 26, -D, W], [-36, AT + 6, 0]].map((q) => P(...q));
    return bbox(c, 0.04, 0.05);
  }

  return { g, render, anchors, hit, view, get model() { return model; } };
}

// ================================================================== Peru

const PERU = { X: 64, C: 30, Zn: 50, D: 14, S: 10 };
const CITIES = [["Tumbes", 49.6], ["Piura", 44], ["Chiclayo", 38.6], ["Trujillo", 34], ["Chimbote", 31], ["Lima", 20.5], ["Pisco", 14.8]];
const RIVERS = [49, 44.6, 38, 33.4, 20];

export function peruModel(st) {
  const c = st.coastal ?? 0;
  const { C, X, Zn } = PERU;
  const sst = (x, z) => 17 + 9.5 * Math.pow(clamp(z / Zn), 1.6) + 2.3 * ((x - C) / (X - C)) - 1.6 * gauss(x - C, 0, 3.5) * (1 - clamp(c / 4)) + c * (0.75 + 0.35 * z / Zn);
  const north = sst(C + 3, 45);
  const rain = clamp((north - 25.8) / 2.5, 0, 1.4);
  const fishLevel = clamp(1 - (c - 0.4) / 2.6, 0.08, 1);
  return { coastal: c, sst, north, rain, fishLevel, piura: sst(C + 3, 44) };
}

function terrain(x, z) {
  const andes = 15 * gauss(x, 8, 6.5) * (0.78 + 0.18 * Math.sin(z * 0.33) + 0.08 * Math.sin(z * 1.27));
  return 0.8 + 1.4 * clamp(1 - x / PERU.C) + andes;
}

function makePeru(root) {
  const { X, C, Zn, D, S } = PERU;
  const { P, inv } = projector(S);
  const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
  g.dataset.scene = "peru";
  root.appendChild(g);
  let model = null;

  function render(state) {
    const m = (model = peruModel(state));
    let s = "";
    const a = P(C, 0, 0), b = P(X, 0, 0);
    s += `<defs><linearGradient id="g-warm-pe" gradientUnits="userSpaceOnUse" x1="${f2(a[0])}" y1="${f2(a[1])}" x2="${f2(b[0])}" y2="${f2(b[1])}">`;
    for (let i = 0; i <= 6; i++) s += `<stop offset="${i / 6}" stop-color="${rgb(sstColor(m.sst(C + (i / 6) * (X - C), 2)))}"/>`;
    s += `</linearGradient></defs>`;

    // Sea surface.
    const nx = 12, nz = 14;
    for (let j = nz - 1; j >= 0; j--) {
      for (let i = 0; i < nx; i++) {
        const x0 = C + (i / nx) * (X - C), x1 = C + ((i + 1) / nx) * (X - C), z0 = (j / nz) * Zn, z1 = ((j + 1) / nz) * Zn;
        const c = rgb(sstColor(m.sst((x0 + x1) / 2, (z0 + z1) / 2)));
        s += `<polygon class="cell" fill="${c}" stroke="${c}" stroke-width=".6" points="${poly(P, [[x0, 0, z0], [x1, 0, z0], [x1, 0, z1], [x0, 0, z1]])}"/>`;
      }
    }
    s += `<polygon class="edge" points="${poly(P, [[C, 0, 0], [X, 0, 0], [X, 0, Zn], [C, 0, Zn]])}"/>`;

    // Humboldt Current flowing north, fading as the coast warms.
    const hop = f2(clamp(1 - m.coastal / 3.5, 0.12, 1));
    for (const x of [44, 54]) {
      for (let z = 4; z < 44; z += 13) {
        s += `<g opacity="${hop}">` + arrow(P(x, 0.3, z), P(x, 0.3, z + 8), "flow", 2.2, "var(--sst-cool)") + `</g>`;
      }
    }

    // Anchoveta: plentiful near the coast, fewer and further south when it warms.
    const nf = Math.round(9 * m.fishLevel);
    const shift = clamp(m.coastal, 0, 4) * 4;
    for (let i = 0; i < nf; i++) {
      const q = P(C + 2.5 + (i % 3) * 2.6, 0, Math.max(1.5, 30 - shift - i * 3));
      s += fish(q[0], q[1], 8, i % 2 === 0);
    }

    // Front face: ocean cross-section.
    const h = 3 + 2.2 * Math.max(m.coastal, 0);
    const N = 16, top = [], tl = [];
    for (let i = 0; i <= N; i++) {
      const x = C + (i / N) * (X - C);
      top.push([x, 0, 0]);
      tl.push([x, -(h * (0.55 + 0.45 * (i / N))), 0]);
    }
    s += `<polygon class="deep" points="${poly(P, [[C, 0, 0], [X, 0, 0], [X, -D, 0], [C, -D, 0]])}"/>`;
    s += `<polygon fill="url(#g-warm-pe)" points="${poly(P, top.concat(tl.slice().reverse()))}"/>`;
    s += `<path class="thermo-line" d="M${tl.map((q) => pt(P(...q))).join("L")}"/>`;
    s += `<polygon class="edge" points="${poly(P, [[C, 0, 0], [X, 0, 0], [X, -D, 0], [C, -D, 0]])}"/>`;
    // Coastal upwelling.
    const up = clamp(1 - m.coastal / 3.2, 0, 1);
    if (up > 0.05) {
      for (const [x0, dx] of [[C + 9, -4], [C + 5, -2.5]]) {
        const pts = [];
        for (let k = 0; k <= 10; k++) { const t = k / 10; pts.push(P(x0 + dx * t * t, -(h + 5) * (1 - t) - 0.3 * t, 0)); }
        s += `<g opacity="${f2(clamp(up, 0.2, 1))}"><path class="arrow flow-slow" d="M${pts.map(pt).join("L")}" stroke="var(--sst-cool)" stroke-width="3"/>` + arrow(pts[8], pts[10], "", 0.01, "var(--sst-cool)", 0.9) + `</g>`;
      }
    }

    // Land: profile slices from north (back) to south (front), with rivers and cities
    // drawn right after the slice behind them so nearer ridges hide them correctly.
    const riverW = 1 + m.rain * 4.5;
    const riverCol = m.rain > 0.75 ? "var(--mud)" : "var(--rain)";
    const placed = new Set();
    for (let z = Zn; z >= 0; z -= 2.5) {
      const prof = [];
      for (let x = 0; x <= C; x += 1) prof.push([x, terrain(x, z), z]);
      const isFront = z === 0;
      const body = prof.concat([[C, 0, z], [C, isFront ? -D : 0, z], [0, isFront ? -D : 0, z]]);
      s += `<polygon class="${isFront ? "land-side" : "land-top"}" points="${poly(P, body)}"/>`;
      s += `<path d="M${prof.map((q) => pt(P(...q))).join("L")}" fill="none" stroke="var(--land-line)" stroke-width="1"/>`;
      RIVERS.forEach((rz, i) => {
        if (placed.has(`r${i}`) || rz < z - 2.5) return;
        placed.add(`r${i}`);
        const pts = [];
        for (let x = 4; x <= C; x += 1) pts.push(P(x, terrain(x, rz) + 0.15, rz + 0.8 * Math.sin(x * 0.6 + i)));
        s += `<path class="river" d="M${pts.map(pt).join("L")}" stroke="${riverCol}" stroke-width="${f2(riverW)}" opacity="${f2(0.55 + 0.4 * clamp(m.rain))}"/>`;
      });
      CITIES.forEach(([name, cz]) => {
        if (placed.has(name) || cz < z - 2.5) return;
        placed.add(name);
        const q = P(C - 1.2, terrain(C - 1.2, cz) + 0.2, cz);
        s += `<g class="city"><circle cx="${f2(q[0])}" cy="${f2(q[1])}" r="3.6"/>${label([q[0] - 7, q[1] + 4], name, "", "end")}</g>`;
      });
    }
    s += label(P(6, terrain(6, 4), 4), tr("learn_s_andes"), "lbl-strong", "middle", -26);
    s += label(P(X, -D, 0), tr("learn_s_pacific"), "lbl-strong", "end", 22);

    // Storms over the warm northern coast.
    if (m.rain > 0.02) {
      for (const [x, z, r] of [[36, 46, 10], [33, 40, 8.5], [41, 36, 7]].slice(0, m.rain > 0.5 ? 3 : 2)) {
        const c = P(x, 15 + 3 * m.rain, z), floor = P(x - 4, 1, z)[1];
        s += storm(c[0], c[1], 2.3 * r * (0.75 + 0.35 * clamp(m.rain)), m.rain, floor);
      }
    } else if (m.coastal < 0.5) {
      const q = P(20, 14, 42); s += sun(q[0], q[1], 11);
    }
    g.innerHTML = s;
  }

  const anchors = {
    humboldt: () => P(54, 0.3, 20),
    desert: () => P(24, terrain(24, 28), 28),
    andes: () => P(8, terrain(8, 30), 30),
    storms: () => { const q = P(36, 18, 46); return [q[0], q[1] - 40]; },
    anchoveta: () => P(C + 4, 0, Math.max(2, 26 - clamp(model.coastal, 0, 4) * 4)),
  };

  function hit(ux, uy) {
    const [x, z] = inv(ux, uy, 0);
    if (x < C || x > X || z < 0 || z > Zn) return null;
    const lat = 18 - (z / Zn) * 14.5;
    return `<b>${tr("learn_sea_temp")}</b><br>${model.sst(x, z).toFixed(1)}&nbsp;°C · ${lat.toFixed(1)}°S`;
  }

  function view() {
    const c = [[0, -D - 3, 0], [X, -D - 3, 0], [0, 20, Zn], [X, 0, Zn], [X, -D, Zn], [0, 22, 0]].map((q) => P(...q));
    return bbox(c, 0.08, 0.06);
  }

  return { g, render, anchors, hit, view };
}

// ================================================================== World plate

const WORLD = { S: 2.35, LON0: -30, LAT0: -60, LAT1: 75, T: 5 };
// Niño boxes: [id, lon0, lon1, lat0, lat1] in degrees east.
export const NINO_BOXES = [
  ["nino4", 160, 210, -5, 5],
  ["nino34", 190, 240, -5, 5],
  ["nino3", 210, 270, -5, 5],
  ["nino12", 270, 280, -10, 0],
];

function makeWorld(root) {
  const { S, LON0, LAT0, LAT1, T: TH } = WORLD;
  const { P } = projector(S);
  const W = 360, H = LAT1 - LAT0;
  const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
  g.dataset.scene = "world";
  root.appendChild(g);
  let world = null, layers = {}, mode = "regions", season = "djf", live = {};
  // Flat map (x = lon − LON0, y = lat) → screen, as an SVG matrix.
  const m = `matrix(${UX[0] * S} ${UX[1] * S} ${-VZ[0] * S} ${-VZ[1] * S} ${f2(-LAT0 * VZ[0] * S)} ${f2(-LAT0 * VZ[1] * S)})`;
  // Note: the plate's flat y axis is latitude pointing north, i.e. scene Z = lat − LAT0.
  const flat = (lon, lat) => P(((lon - LON0) % 360 + 360) % 360, 0, lat - LAT0);

  fetch("/static/learn/world.json").then((r) => r.json()).then((d) => { world = d; if (g.isConnected && g.style.display !== "none") render(); }).catch(() => {});
  for (const k of ["djf", "jja"]) {
    fetch(`/static/layers/typical_${k}.geojson`).then((r) => r.json()).then((d) => { layers[k] = d; if (mode === "impacts") render(); }).catch(() => {});
  }

  function ringPath(coords, shift = 0) {
    let d = "";
    for (let i = 0; i < coords.length; i += 2) d += (i ? "L" : "M") + f2(coords[i] - LON0 + shift) + "," + f2(-coords[i + 1]);
    return d + "Z";
  }

  function render(state = {}) {
    mode = state.mode ?? mode; season = state.season ?? season; live = state.live ?? live;
    let s = "";
    // Plate thickness (south and east faces).
    s += `<polygon class="land-side" points="${poly(P, [[0, 0, 0], [W, 0, 0], [W, -TH, 0], [0, -TH, 0]])}"/>`;
    s += `<polygon class="land-side2" points="${poly(P, [[W, 0, 0], [W, 0, H], [W, -TH, H], [W, -TH, 0]])}"/>`;
    // Map: drawn flat with y = −lat, then mapped with a matrix (lat flips via the matrix).
    const mm = `matrix(${f2(UX[0] * S)} ${f2(UX[1] * S)} ${f2(-VZ[0] * S)} ${f2(-VZ[1] * S)} ${f2(-LAT0 * VZ[0] * S)} ${f2(-LAT0 * VZ[1] * S)})`;
    void m;
    s += `<defs><clipPath id="clip-plate"><rect x="0" y="${-LAT1}" width="${W}" height="${H}"/></clipPath></defs>`;
    s += `<g transform="${mm}" clip-path="url(#clip-plate)">`;
    s += `<rect x="0" y="${-LAT1}" width="${W}" height="${H}" fill="var(--paper-ocean)"/>`;
    for (let lon = 0; lon <= 360; lon += 30) s += `<path class="edge" d="M${lon},${-LAT1}V${-LAT0}" opacity=".25"/>`;
    for (let lat = -60; lat <= 75; lat += 30) s += `<path class="edge" d="M0,${-lat}H${W}" opacity="${lat === 0 ? 0.7 : 0.25}" ${lat === 0 ? 'stroke-dasharray="4 4"' : ""}/>`;
    if (world) {
      let d = "";
      for (const r of world.rings) {
        let minx = Infinity, maxx = -Infinity;
        for (let i = 0; i < r.length; i += 2) { minx = Math.min(minx, r[i]); maxx = Math.max(maxx, r[i]); }
        d += ringPath(r);
        if (maxx > LON0 + 360) d += ringPath(r, -360);
        if (minx < LON0) d += ringPath(r, 360);
      }
      s += `<path d="${d}" fill="var(--land)" stroke="var(--land-line)" stroke-width=".7" vector-effect="non-scaling-stroke" stroke-linejoin="round"/>`;
    }
    if (mode === "regions") {
      for (const [id, lo0, lo1, la0, la1] of NINO_BOXES) {
        const v = live[`${id}_weekly`]?.value;
        const fill = v != null ? `style="fill:${rgb(anomColor(v))};fill-opacity:.85"` : "";
        const main = id === "nino34" || id === "nino12";
        s += `<rect class="region-box${main ? " main" : ""}" data-region="${id}" x="${lo0 - LON0}" y="${-la1}" width="${lo1 - lo0}" height="${la1 - la0}" ${fill}/>`;
      }
    }
    if (mode === "impacts" && layers[season]) {
      for (const f of layers[season].features) {
        const ring = f.geometry.coordinates[0];
        const d = "M" + ring.map(([lo, la]) => `${f2((((lo - LON0) % 360) + 360) % 360)},${f2(-la)}`).join("L") + "Z";
        s += `<path class="impact ${f.properties.kind}" d="${d}" data-name="${f.properties[document.documentElement.lang] || f.properties.en}" data-kind="${f.properties.kind}"/>`;
      }
    }
    s += `</g>`;
    s += `<polygon class="edge" points="${poly(P, [[0, 0, 0], [W, 0, 0], [W, 0, H], [0, 0, H]])}"/>`;
    for (const lon of [120, 180, 240, 300]) {
      const q = P(lon - LON0, -TH, 0);
      s += label([q[0], q[1] + 14], lonLabel(lon), "tick");
    }
    s += label(P(W - 6, 0, -LAT0), "0°", "tick", "start", -4);
    g.innerHTML = s;
  }

  const center = (id) => {
    const b = NINO_BOXES.find((r) => r[0] === id);
    return flat((b[1] + b[2]) / 2, b[4]);
  };
  const anchors = {
    nino4: () => center("nino4"), nino34: () => center("nino34"), nino3: () => center("nino3"), nino12: () => center("nino12"),
    g_peru: () => flat(-80, -5), g_indo: () => flat(125, -6), g_us: () => flat(-97, 31), g_africa: () => flat(28, -20),
  };

  function view(focus) {
    if (focus === "pacific") {
      const c = [flat(135, -22), flat(292, -22), flat(135, 22), flat(292, 22)];
      return bbox(c, 0.04, 0.2);
    }
    const c = [[0, -TH, 0], [W, -TH, 0], [0, 0, H], [W, 0, H], [W, -TH, H]].map((q) => P(...q));
    return bbox(c, 0.03, 0.06);
  }

  return { g, render, anchors, view, hit: () => null };
}

// ================================================================== History skyline

// A flatter oblique view than the other scenes, so 77 columns fit the stage.
const HIST = { sp: 2.5, bw: 1.8, bd: 3.4, Hs: 11, S: 4.6, ux: [0.98, 0.12], vz: [0.5, -0.36] };

function makeHistory(root) {
  const { sp, bw, bd, Hs, S } = HIST;
  const { P } = projector(S, HIST.ux, HIST.vz);
  const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
  g.dataset.scene = "history";
  root.appendChild(g);
  let bars = [], selected = null, message = "";
  const n = () => Math.max(bars.length, 77);

  function setData(list) { bars = list; }
  function setMessage(msg) { message = msg; }

  function render(state = {}) {
    selected = state.event ?? selected;
    const N = n(), len = N * sp + 2;
    let s = "";
    // Back wall with thresholds.
    const wallTop = 2.8 * Hs, wallBot = -2.2 * Hs;
    s += `<polygon fill="var(--card)" stroke="var(--line)" points="${poly(P, [[-2, wallBot, bd + 2], [len, wallBot, bd + 2], [len, wallTop, bd + 2], [-2, wallTop, bd + 2]])}"/>`;
    for (const v of [-1.5, -1, -0.5, 0.5, 1, 1.5, 2, 2.5]) {
      const a = P(-2, v * Hs, bd + 2), b = P(len, v * Hs, bd + 2);
      s += `<path d="M${pt(a)}L${pt(b)}" stroke="${v > 0 ? "var(--sst-warm)" : "var(--sst-cool)"}" stroke-width="1" stroke-dasharray="${v % 1 === 0 ? "" : "3 4"}" opacity=".45"/>`;
      s += label([a[0] - 6, a[1] + 3.5], `${v > 0 ? "+" : "−"}${Math.abs(v).toFixed(1)}`, "tick", "end");
    }
    const cats = [[0.5, "cat_weak"], [1, "cat_moderate"], [1.5, "cat_strong"], [2, "cat_very_strong"], [-0.5, "cat_la_nina"]];
    for (const [v, k] of cats) { const q = P(len, v * Hs, bd + 2); s += label([q[0] + 6, q[1] + (v > 0 ? -4 : 12)], tr(k), "tick", "start"); }
    // Hanging (La Niña) bars first, then the base plate, then El Niño bars.
    const barMarkup = (b, i) => {
      const x0 = i * sp, x1 = x0 + bw, z0 = 0, z1 = bd, h = b.v * Hs;
      const y0 = Math.min(0, h), y1 = Math.max(0, h);
      const c = Math.abs(b.v) < 0.5 ? mix(PAL.mid, PAL.navy, 0.18) : anomColor(b.v * 1.1);
      const sel = selected === b.year ? " sel" : "";
      return `<g class="bar${sel}" data-i="${i}">` +
        `<polygon fill="${rgb(c)}" points="${poly(P, [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]])}"/>` +
        `<polygon fill="${rgb(darken(c, 0.22))}" points="${poly(P, [[x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0]])}"/>` +
        `<polygon fill="${rgb(lighten(c, 0.25))}" points="${poly(P, [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]])}"/></g>`;
    };
    bars.forEach((b, i) => { if (b.v < 0) s += barMarkup(b, i); });
    s += `<polygon class="land-top" points="${poly(P, [[-2, 0, -1], [len, 0, -1], [len, 0, bd + 1], [-2, 0, bd + 1]])}" opacity=".85"/>`;
    s += `<polygon class="land-side" points="${poly(P, [[-2, 0, -1], [len, 0, -1], [len, -1.2, -1], [-2, -1.2, -1]])}"/>`;
    bars.forEach((b, i) => { if (b.v >= 0) s += barMarkup(b, i); });
    bars.forEach((b, i) => {
      if (b.year % 10 === 0) { const q = P(i * sp + bw / 2, -1.2, -1); s += label([q[0], q[1] + 13], String(b.year), "tick"); }
    });
    if (!bars.length && message) s += label(P(len / 2, Hs, 0), message, "lbl", "middle");
    g.innerHTML = s;
  }

  function barTop(year) {
    const i = bars.findIndex((b) => b.year === year);
    if (i < 0) return null;
    return P(i * sp + bw / 2, Math.max(0, bars[i].v * Hs), bd / 2);
  }

  const anchors = new Proxy({}, { get: (_, id) => () => barTop(Number(String(id).replace("ev_", ""))) });

  function view() {
    const N = n(), len = N * sp + 2;
    const c = [[-8, -2.2 * Hs, -1], [len + 16, -2.2 * Hs, -1], [-8, 2.8 * Hs, bd + 2], [len + 16, 2.8 * Hs, bd + 2], [len + 16, -2.2 * Hs, bd + 2]].map((q) => P(...q));
    return bbox(c, 0.02, 0.06);
  }

  return { g, render, anchors, view, setData, setMessage, hit: () => null, bar: (i) => bars[i] };
}

// ================================================================== stage

function bbox(points, px, py) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const [x, y] of points) { x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y); }
  const w = x1 - x0, h = y1 - y0;
  return [x0 - w * px, y0 - h * py, w * (1 + 2 * px), h * (1 + 2 * py)];
}

export function createStage(svg) {
  refreshPalette();
  const root = document.createElementNS("http://www.w3.org/2000/svg", "g");
  svg.appendChild(root);
  const scenes = { pacific: makePacific(root), peru: makePeru(root), world: makeWorld(root), history: makeHistory(root) };
  let current = null, vb = null, vbAnim = null;

  function setViewBox(target, animate) {
    cancelAnimationFrame(vbAnim);
    if (!animate || !vb) {
      vb = target;
      svg.setAttribute("viewBox", vb.map(f2).join(" "));
      svg.dispatchEvent(new Event("camera"));
      return;
    }
    const from = vb.slice(), t0 = performance.now(), dur = 650;
    const step = (now) => {
      const k = clamp((now - t0) / dur), e = k < 0.5 ? 2 * k * k : 1 - (-2 * k + 2) ** 2 / 2;
      vb = from.map((v, i) => v + (target[i] - v) * e);
      svg.setAttribute("viewBox", vb.map(f2).join(" "));
      svg.dispatchEvent(new Event("camera"));
      if (k < 1) vbAnim = requestAnimationFrame(step);
    };
    vbAnim = requestAnimationFrame(step);
  }

  return {
    scenes,
    get name() { return current; },
    show(name, focus) {
      const same = current === name;
      for (const [k, sc] of Object.entries(scenes)) sc.g.style.display = k === name ? "" : "none";
      current = name;
      setViewBox(scenes[name].view(focus), same);
    },
    render(state) { scenes[current].render(state); },
    anchor(id) {
      const f = scenes[current].anchors[id];
      return f ? f() : null;
    },
    hit(ux, uy) { return scenes[current].hit(ux, uy); },
    refreshPalette,
  };
}
