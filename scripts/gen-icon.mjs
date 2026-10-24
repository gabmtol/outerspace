// Generates the OuterSpace app icon (1024x1024 SVG) with a macOS-style superellipse body.
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

const out = process.argv[2];
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

// screen panel (16:10)
const P = { x: 262, y: 318, w: 500, h: 312, r: 58 };
// orbit ring, local frame centred on the panel, tilted
const R = { cx: 512, cy: 488, rx: 338, ry: 96, rot: -13 };
const satT = (32 * Math.PI) / 180;
const sat = { x: R.rx * Math.cos(satT), y: R.ry * Math.sin(satT) };

const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>OuterSpace</title>
  <defs>
    <clipPath id="body"><path d="${body}"/></clipPath>
    <clipPath id="screen"><rect x="${P.x}" y="${P.y}" width="${P.w}" height="${P.h}" rx="${P.r}"/></clipPath>
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
    <linearGradient id="glassFill" x1="0" y1="0" x2="0.35" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.42"/>
      <stop offset="0.5" stop-color="#fff" stop-opacity="0.12"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0.06"/>
    </linearGradient>
    <linearGradient id="glassRim" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.95"/>
      <stop offset="0.45" stop-color="#fff" stop-opacity="0.25"/>
      <stop offset="0.8" stop-color="#ffd2b0" stop-opacity="0.35"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0.8"/>
    </linearGradient>
    <linearGradient id="ringGrad" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#9fb4ff"/>
      <stop offset="0.5" stop-color="#ffffff"/>
      <stop offset="1" stop-color="#ffc08a"/>
    </linearGradient>

    <filter id="frost" x="-10%" y="-10%" width="120%" height="120%">
      <feGaussianBlur stdDeviation="22"/>
      <feColorMatrix type="matrix" values="1.08 0 0 0 0.05  0 1.08 0 0 0.05  0 0 1.1 0 0.07  0 0 0 1 0"/>
    </filter>
    <filter id="soft" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="26"/></filter>
    <filter id="glow" x="-200%" y="-200%" width="500%" height="500%"><feGaussianBlur stdDeviation="10"/></filter>
    <filter id="iconShadow" x="-20%" y="-20%" width="140%" height="150%">
      <feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity="0.35"/>
    </filter>
    <filter id="cursorShadow" x="-50%" y="-50%" width="200%" height="200%">
      <feDropShadow dx="0" dy="4" stdDeviation="5" flood-color="#120a30" flood-opacity="0.45"/>
    </filter>

    <g id="scene">
      <rect width="1024" height="1024" fill="url(#sky)"/>
      <rect width="1024" height="1024" fill="url(#nebula)"/>
      ${stars.join("\n      ")}
      <ellipse cx="512" cy="815" rx="520" ry="210" fill="url(#dawn)"/>
      <circle cx="512" cy="1750" r="960" fill="url(#planet)"/>
      <circle cx="512" cy="1750" r="960" fill="none" stroke="#ffb26b" stroke-width="10" opacity="0.9" filter="url(#glow)"/>
      <circle cx="512" cy="1750" r="960" fill="none" stroke="#ffe2b8" stroke-width="2.5" opacity="0.9"/>
    </g>
  </defs>

  <g filter="url(#iconShadow)">
    <path d="${body}" fill="#0b0a1f"/>
  </g>

  <g clip-path="url(#body)">
    <use href="#scene"/>

    <!-- orbit, back half (behind the screen) -->
    <g transform="translate(${R.cx} ${R.cy}) rotate(${R.rot})">
      <ellipse rx="${R.rx}" ry="${R.ry}" fill="none" stroke="url(#ringGrad)" stroke-width="7" opacity="0.35" clip-path="url(#ringBack)"/>
    </g>

    <!-- floating glass screen -->
    <rect x="${P.x + 10}" y="${P.y + 34}" width="${P.w - 20}" height="${P.h - 10}" rx="${P.r}" fill="#05030f" opacity="0.55" filter="url(#soft)"/>
    <g clip-path="url(#screen)">
      <use href="#scene" filter="url(#frost)"/>
      <rect x="${P.x}" y="${P.y}" width="${P.w}" height="${P.h}" fill="url(#glassFill)"/>
      <ellipse cx="${P.x + 150}" cy="${P.y + 10}" rx="260" ry="70" fill="#fff" opacity="0.18" filter="url(#glow)"/>
    </g>
    <rect x="${P.x + 1.5}" y="${P.y + 1.5}" width="${P.w - 3}" height="${P.h - 3}" rx="${P.r - 1.5}" fill="none" stroke="url(#glassRim)" stroke-width="3"/>
    <rect x="${P.x + 5}" y="${P.y + 5}" width="${P.w - 10}" height="${P.h - 10}" rx="${P.r - 5}" fill="none" stroke="#fff" stroke-opacity="0.12" stroke-width="1.5"/>

    <!-- agent pointer on the virtual screen -->
    <g transform="translate(470 404) scale(4.2)" filter="url(#cursorShadow)">
      <path d="M0 0 L0 30 L8 23 L13.5 34 L18 32 L12.6 21.2 L23 21 Z" fill="#fff" stroke="#e9e4ff" stroke-width="0.8" stroke-linejoin="round"/>
    </g>

    <!-- orbit, front half + satellite -->
    <g transform="translate(${R.cx} ${R.cy}) rotate(${R.rot})">
      <ellipse rx="${R.rx}" ry="${R.ry}" fill="none" stroke="url(#ringGrad)" stroke-width="8" stroke-linecap="round" clip-path="url(#ringFront)"/>
      <circle cx="${sat.x.toFixed(1)}" cy="${sat.y.toFixed(1)}" r="34" fill="#ffd08a" opacity="0.8" filter="url(#glow)"/>
      <circle cx="${sat.x.toFixed(1)}" cy="${sat.y.toFixed(1)}" r="17" fill="#fff6e8"/>
    </g>

    <path d="${body}" fill="none" stroke="url(#bodyRim)" stroke-width="6"/>
  </g>
</svg>
`;

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, svg);
console.log("wrote", out);
