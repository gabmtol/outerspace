// Generates the OuterSpace app icon candidates (1024x1024 SVG, macOS-style superellipse body).
// Usage: node scripts/gen-icon.mjs [outDir]   -> writes outerspace-<variant>.svg for every variant
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const outDir = process.argv[2] ?? "assets/icon/variants";
const C = 512, A = 412, N = 5, STEPS = 360;

function squircle(cx, cy, a, n) {
  const pts = [];
  for (let i = 0; i < STEPS; i++) {
    const t = (i / STEPS) * Math.PI * 2;
    const c = Math.cos(t), s = Math.sin(t);
    const x = cx + a * Math.sign(c) * Math.abs(c) ** (2 / n);
    const y = cy + a * Math.sign(s) * Math.abs(s) ** (2 / n);
    pts.push(`${x.toFixed(2)},${y.toFixed(2)}`);
  }
  return `M${pts[0]}L${pts.slice(1).join("L")}Z`;
}

const body = squircle(C, C, A, N);

// deterministic star field
let seed = 7;
const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
const stars = [];
for (let i = 0; i < 46; i++) {
  const x = 130 + rnd() * 764, y = 120 + rnd() * 560;
  const r = rnd() < 0.15 ? 3.2 : 1.2 + rnd() * 1.4;
  const o = (0.35 + rnd() * 0.6).toFixed(2);
  stars.push(`<circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="${r.toFixed(1)}" fill="#fff" opacity="${o}"/>`);
}

// centre of the hero element, and the orbit ring (local frame, tilted)
const O = { x: 512, y: 470 };
const R = { cx: 512, cy: 488, rx: 338, ry: 96, rot: -13 };
const satT = (32 * Math.PI) / 180;
const sat = { x: R.rx * Math.cos(satT), y: R.ry * Math.sin(satT) };

// a glass ball lens shows the world behind it shrunk and upside down
const lens = (s) => `translate(${O.x} ${O.y}) scale(${s}) translate(${-O.x} ${-O.y})`;

const sharedDefs = `
    <clipPath id="body"><path d="${body}"/></clipPath>
    <clipPath id="ringBack" clipPathUnits="userSpaceOnUse"><rect x="-600" y="-600" width="1200" height="600"/></clipPath>
    <clipPath id="ringFront" clipPathUnits="userSpaceOnUse"><rect x="-600" y="0" width="1200" height="600"/></clipPath>

    <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#070b26"/>
      <stop offset="0.45" stop-color="#1d1850"/>
      <stop offset="0.72" stop-color="#5a2a6e"/>
      <stop offset="0.9" stop-color="#d4566a"/>
      <stop offset="1" stop-color="#ff8a4c"/>
    </linearGradient>
    <radialGradient id="nebula" cx="0.28" cy="0.3" r="0.55">
      <stop offset="0" stop-color="#6c5cff" stop-opacity="0.55"/>
      <stop offset="1" stop-color="#6c5cff" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="dawn" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#ffd08a" stop-opacity="0.95"/>
      <stop offset="0.35" stop-color="#ff7a45" stop-opacity="0.75"/>
      <stop offset="1" stop-color="#ff5a5f" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="planet" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#1a1238"/>
      <stop offset="0.25" stop-color="#0b0a1f"/>
    </linearGradient>
    <linearGradient id="bodyRim" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.7"/>
      <stop offset="0.3" stop-color="#fff" stop-opacity="0.08"/>
      <stop offset="0.85" stop-color="#fff" stop-opacity="0"/>
      <stop offset="1" stop-color="#ffb07a" stop-opacity="0.45"/>
    </linearGradient>
    <linearGradient id="ringGrad" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#9fb4ff"/>
      <stop offset="0.5" stop-color="#ffffff"/>
      <stop offset="1" stop-color="#ffc08a"/>
    </linearGradient>
    <linearGradient id="glassRim" x1="0" y1="0" x2="0.4" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.95"/>
      <stop offset="0.45" stop-color="#fff" stop-opacity="0.18"/>
      <stop offset="0.8" stop-color="#ffd2b0" stop-opacity="0.4"/>
      <stop offset="1" stop-color="#ffc890" stop-opacity="0.9"/>
    </linearGradient>
    <linearGradient id="specular" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.85"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>

    <filter id="soft" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="26"/></filter>
    <filter id="glow" x="-200%" y="-200%" width="500%" height="500%"><feGaussianBlur stdDeviation="10"/></filter>
    <filter id="haze" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="5"/></filter>
    <filter id="lensBlur" x="-10%" y="-10%" width="120%" height="120%"><feGaussianBlur stdDeviation="3"/></filter>
    <filter id="frost" x="-10%" y="-10%" width="120%" height="120%">
      <feGaussianBlur stdDeviation="9"/>
      <feColorMatrix type="matrix" values="1.1 0 0 0 0.06  0 1.1 0 0 0.06  0 0 1.12 0 0.08  0 0 0 1 0"/>
    </filter>
    <filter id="dim" x="-10%" y="-10%" width="120%" height="120%">
      <feGaussianBlur stdDeviation="4"/>
      <feColorMatrix type="matrix" values="0.4 0 0 0 0  0 0.35 0 0 0  0 0 0.45 0 0  0 0 0 1 0"/>
    </filter>
    <filter id="iconShadow" x="-20%" y="-20%" width="140%" height="150%">
      <feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity="0.35"/>
    </filter>

    <g id="scene">
      <rect width="1024" height="1024" fill="url(#sky)"/>
      <rect width="1024" height="1024" fill="url(#nebula)"/>
      ${stars.join("\n      ")}
      <ellipse cx="512" cy="815" rx="520" ry="210" fill="url(#dawn)"/>
      <circle cx="512" cy="1750" r="960" fill="url(#planet)"/>
      <circle cx="512" cy="1750" r="960" fill="none" stroke="#ffb26b" stroke-width="10" opacity="0.9" filter="url(#glow)"/>
      <circle cx="512" cy="1750" r="960" fill="none" stroke="#ffe2b8" stroke-width="2.5" opacity="0.9"/>
    </g>`;

const ringBack = `
    <g transform="translate(${R.cx} ${R.cy}) rotate(${R.rot})">
      <ellipse rx="${R.rx}" ry="${R.ry}" fill="none" stroke="url(#ringGrad)" stroke-width="7" opacity="0.35" clip-path="url(#ringBack)"/>
    </g>`;

const ringFront = (satellite) => `
    <g transform="translate(${R.cx} ${R.cy}) rotate(${R.rot})">
      <ellipse rx="${R.rx}" ry="${R.ry}" fill="none" stroke="url(#ringGrad)" stroke-width="8" stroke-linecap="round" clip-path="url(#ringFront)"/>${satellite ? `
      <circle cx="${sat.x.toFixed(1)}" cy="${sat.y.toFixed(1)}" r="34" fill="#ffd08a" opacity="0.8" filter="url(#glow)"/>
      <circle cx="${sat.x.toFixed(1)}" cy="${sat.y.toFixed(1)}" r="17" fill="#fff6e8"/>` : ""}
    </g>`;

// ── variant 1: a liquid-glass planet the ring orbits, like a glass Saturn ──────────────────────
function orb() {
  const r = 182;
  return {
    satellite: true,
    defs: `
    <clipPath id="hero"><circle cx="${O.x}" cy="${O.y}" r="${r}"/></clipPath>
    <radialGradient id="orbClear" cx="0.35" cy="0.3" r="0.7">
      <stop offset="0" stop-color="#fff" stop-opacity="0.16"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0"/>
    </radialGradient>
    <radialGradient id="orbShade" cx="0.5" cy="0.45" r="0.55">
      <stop offset="0.55" stop-color="#0a0620" stop-opacity="0"/>
      <stop offset="0.9" stop-color="#0a0620" stop-opacity="0.32"/>
      <stop offset="1" stop-color="#0a0620" stop-opacity="0.55"/>
    </radialGradient>`,
    hero: `
    <circle cx="${O.x}" cy="${O.y + 16}" r="${r + 10}" fill="#05030f" opacity="0.45" filter="url(#soft)"/>
    <g clip-path="url(#hero)">
      <g filter="url(#lensBlur)"><use href="#scene" transform="${lens("-0.42 -0.42")}"/></g>
      <circle cx="${O.x}" cy="${O.y}" r="${r}" fill="url(#orbClear)"/>
      <circle cx="${O.x}" cy="${O.y}" r="${r}" fill="url(#orbShade)"/>
      <ellipse cx="${O.x}" cy="${O.y + r - 22}" rx="110" ry="30" fill="#ff8a4c" opacity="0.4" filter="url(#glow)"/>
      <circle cx="${O.x}" cy="${O.y}" r="${r - 8}" fill="none" stroke="#fff" stroke-width="14" opacity="0.26" filter="url(#haze)"/>
    </g>
    <circle cx="${O.x}" cy="${O.y}" r="${r - 1.5}" fill="none" stroke="url(#glassRim)" stroke-width="3"/>
    <ellipse cx="${O.x - 62}" cy="${O.y - 104}" rx="86" ry="40" transform="rotate(-28 ${O.x - 62} ${O.y - 104})" fill="url(#specular)" opacity="0.75" filter="url(#haze)"/>
    <ellipse cx="${O.x - 92}" cy="${O.y - 118}" rx="18" ry="9" transform="rotate(-28 ${O.x - 92} ${O.y - 118})" fill="#fff" opacity="0.9"/>`,
  };
}

// ── variant 2: a tinted-glass moon eclipsing a golden sun, with the "diamond ring" flare ───────
function eclipse() {
  const r = 152;
  const a = (-52 * Math.PI) / 180;
  const d = { x: O.x + r * Math.cos(a), y: O.y + r * Math.sin(a) };
  const streamers = [[18, 300, 14], [74, 250, 10], [128, 280, 12], [196, 310, 13], [252, 240, 9], [318, 270, 11]]
    .map(([rot, rx, ry]) => `<ellipse cx="${O.x}" cy="${O.y}" rx="${rx}" ry="${ry}" transform="rotate(${rot} ${O.x} ${O.y})" fill="#ffe2b0" opacity="0.18" filter="url(#haze)"/>`)
    .join("\n    ");
  return {
    satellite: false,
    defs: `
    <clipPath id="hero"><circle cx="${O.x}" cy="${O.y}" r="${r}"/></clipPath>
    <radialGradient id="corona" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0.4" stop-color="#fff8e6"/>
      <stop offset="0.47" stop-color="#ffe7b0" stop-opacity="0.95"/>
      <stop offset="0.56" stop-color="#ffc070" stop-opacity="0.55"/>
      <stop offset="0.74" stop-color="#ff8a4c" stop-opacity="0.18"/>
      <stop offset="1" stop-color="#ff8a4c" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="moonTint" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#2a1f55" stop-opacity="0.55"/>
      <stop offset="0.5" stop-color="#07051a" stop-opacity="0.82"/>
      <stop offset="1" stop-color="#07051a" stop-opacity="0.9"/>
    </linearGradient>`,
    hero: `
    <circle cx="${O.x}" cy="${O.y}" r="${r / 0.44}" fill="url(#corona)"/>
    ${streamers}
    <g clip-path="url(#hero)">
      <g filter="url(#dim)"><use href="#scene" transform="${lens("-0.36 -0.36")}"/></g>
      <circle cx="${O.x}" cy="${O.y}" r="${r}" fill="url(#moonTint)"/>
      <circle cx="${O.x}" cy="${O.y}" r="${r - 6}" fill="none" stroke="#fff" stroke-width="8" opacity="0.1" filter="url(#haze)"/>
    </g>
    <circle cx="${O.x}" cy="${O.y}" r="${r}" fill="none" stroke="#ff9a6a" stroke-width="9" opacity="0.6" filter="url(#haze)"/>
    <circle cx="${O.x}" cy="${O.y}" r="${r - 1}" fill="none" stroke="#ffd6b0" stroke-width="2.5"/>
    <g transform="translate(${d.x.toFixed(1)} ${d.y.toFixed(1)})">
      <circle r="44" fill="#fff4dc" opacity="0.9" filter="url(#glow)"/>
      <path d="M-120 0 L120 0 M0 -88 L0 88" stroke="#fff" stroke-width="3" stroke-linecap="round" opacity="0.75" filter="url(#lensBlur)"/>
      <path d="M-36 -36 L36 36 M-36 36 L36 -36" stroke="#fff" stroke-width="2" stroke-linecap="round" opacity="0.45"/>
      <circle r="14" fill="#fff"/>
    </g>`,
  };
}

// ── variant 3: a four-point glass star — the agent — caught in orbit ───────────────────────────
function spark() {
  const star = (s) => {
    const h = 210 * s, w = 176 * s;
    const k1 = 34 * s, k2 = 72 * s, k3 = 72 * s, k4 = 34 * s;
    return `M0 ${-h} C${k1} ${-k2} ${k3} ${-k4} ${w} 0 C${k3} ${k4} ${k1} ${k2} 0 ${h} C${-k1} ${k2} ${-k3} ${k4} ${-w} 0 C${-k3} ${-k4} ${-k1} ${-k2} 0 ${-h}Z`;
  };
  const big = star(1), small = star(0.3);
  const sm = { x: O.x + 170, y: O.y - 158 };
  return {
    satellite: true,
    defs: `
    <clipPath id="hero"><path transform="translate(${O.x} ${O.y})" d="${big}"/></clipPath>
    <radialGradient id="halo" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#d9d2ff" stop-opacity="0.55"/>
      <stop offset="1" stop-color="#8f7dff" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="starFill" x1="0" y1="0" x2="0.3" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.62"/>
      <stop offset="0.5" stop-color="#fff" stop-opacity="0.16"/>
      <stop offset="1" stop-color="#ffb380" stop-opacity="0.45"/>
    </linearGradient>`,
    hero: `
    <circle cx="${O.x}" cy="${O.y}" r="270" fill="url(#halo)"/>
    <path transform="translate(${O.x} ${O.y + 18})" d="${big}" fill="#05030f" opacity="0.4" filter="url(#soft)"/>
    <g clip-path="url(#hero)">
      <g filter="url(#frost)"><use href="#scene" transform="${lens("1.3")}"/></g>
      <path transform="translate(${O.x} ${O.y})" d="${big}" fill="url(#starFill)"/>
      <path transform="translate(${O.x} ${O.y})" d="${big}" fill="none" stroke="#fff" stroke-width="20" opacity="0.32" filter="url(#haze)"/>
      <ellipse cx="${O.x - 30}" cy="${O.y - 70}" rx="34" ry="90" transform="rotate(14 ${O.x - 30} ${O.y - 70})" fill="url(#specular)" opacity="0.55" filter="url(#haze)"/>
    </g>
    <path transform="translate(${O.x} ${O.y})" d="${big}" fill="none" stroke="url(#glassRim)" stroke-width="3" stroke-linejoin="round"/>
    <circle cx="${sm.x}" cy="${sm.y}" r="40" fill="#e9e4ff" opacity="0.45" filter="url(#glow)"/>
    <path transform="translate(${sm.x} ${sm.y})" d="${small}" fill="#fff" opacity="0.95"/>`,
  };
}

function render(v) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>OuterSpace</title>
  <defs>${sharedDefs}${v.defs}
  </defs>

  <g filter="url(#iconShadow)">
    <path d="${body}" fill="#0b0a1f"/>
  </g>

  <g clip-path="url(#body)">
    <use href="#scene"/>
${ringBack}
${v.hero}
${ringFront(v.satellite)}

    <path d="${body}" fill="none" stroke="url(#bodyRim)" stroke-width="6"/>
  </g>
</svg>
`;
}

mkdirSync(outDir, { recursive: true });
for (const [name, make] of Object.entries({ orb, eclipse, spark })) {
  const file = join(outDir, `outerspace-${name}.svg`);
  writeFileSync(file, render(make()));
  console.log("wrote", file);
}
