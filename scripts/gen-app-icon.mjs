// Generates the final OuterSpace icon ("Twin") and its tray state variants.
// Usage: node scripts/gen-app-icon.mjs [outDir]
//   outerspace.svg            app icon (macOS-style margin + shadow)
//   tray-{on,off,error}.svg   full-bleed versions for the notification area, back screen tinted by state
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const outDir = process.argv[2] ?? "assets/icon";

function squircle(cx, cy, a, n = 5, steps = 360) {
  const pts = [];
  for (let i = 0; i < steps; i++) {
    const t = (i / steps) * Math.PI * 2;
    const c = Math.cos(t), s = Math.sin(t);
    const x = cx + a * Math.sign(c) * Math.abs(c) ** (2 / n);
    const y = cy + a * Math.sign(s) * Math.abs(s) ** (2 / n);
    pts.push(`${x.toFixed(2)},${y.toFixed(2)}`);
  }
  return `M${pts[0]}L${pts.slice(1).join("L")}Z`;
}

const rr = ({ x, y, w, h, r }, attrs = "") => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}" ${attrs}/>`;
const inset = (b, d) => ({ x: b.x + d, y: b.y + d, w: b.w - 2 * d, h: b.h - 2 * d, r: b.r - d });

const TINTS = {
  on: ["#FFB86E", "#FF7B32", "#E0502A"],
  off: ["#C9C8D2", "#9A99A6", "#76757F"],
  error: ["#FF9A8F", "#F2555A", "#C8343A"],
};

const BACK = { x: 352, y: 312, w: 420, h: 268, r: 46 };
const FRONT = { x: 252, y: 444, w: 420, h: 268, r: 46 };

function icon({ tint, bodyRadius, scale, shadow }) {
  const body = squircle(512, 512, bodyRadius);
  const [t0, t1, t2] = TINTS[tint];
  const zoom = `translate(512 512) scale(${scale}) translate(-512 -512)`;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>OuterSpace</title>
  <defs>
    <clipPath id="body"><path d="${body}"/></clipPath>
    <clipPath id="front">${rr(FRONT)}</clipPath>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2C2742"/><stop offset="1" stop-color="#100E1B"/></linearGradient>
    <linearGradient id="back" x1="0" y1="0" x2="0.25" y2="1"><stop offset="0" stop-color="${t0}"/><stop offset="0.55" stop-color="${t1}"/><stop offset="1" stop-color="${t2}"/></linearGradient>
    <linearGradient id="frostFill" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.26"/><stop offset="1" stop-color="#fff" stop-opacity="0.08"/></linearGradient>
    <linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.42"/><stop offset="0.32" stop-color="#fff" stop-opacity="0"/></linearGradient>
    <linearGradient id="rim" x1="0" y1="0" x2="0.5" y2="1"><stop offset="0" stop-color="#fff"/><stop offset="0.5" stop-color="#fff" stop-opacity="0.2"/><stop offset="1" stop-color="#fff" stop-opacity="0.6"/></linearGradient>
    <linearGradient id="rimWarm" x1="0" y1="0" x2="0.5" y2="1"><stop offset="0" stop-color="#FFE9D6"/><stop offset="0.5" stop-color="#FFE9D6" stop-opacity="0.15"/><stop offset="1" stop-color="#FFE9D6" stop-opacity="0.5"/></linearGradient>
    <linearGradient id="bodyRim" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.45"/><stop offset="0.3" stop-color="#fff" stop-opacity="0.06"/><stop offset="1" stop-color="#fff" stop-opacity="0.12"/></linearGradient>
    <filter id="soft" x="-30%" y="-30%" width="160%" height="170%"><feGaussianBlur stdDeviation="22"/></filter>
    <filter id="frost" x="-10%" y="-10%" width="120%" height="120%"><feGaussianBlur stdDeviation="26"/></filter>
    <filter id="iconShadow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity="0.3"/></filter>
    <g id="scene">
      <rect width="1024" height="1024" fill="url(#bg)"/>
      ${rr(BACK, `fill="url(#back)"`)}
      ${rr(BACK, `fill="url(#sheen)"`)}
      ${rr(inset(BACK, 1.5), `fill="none" stroke="url(#rimWarm)" stroke-width="3"`)}
    </g>
  </defs>
  <path d="${body}" fill="url(#bg)"${shadow ? ` filter="url(#iconShadow)"` : ""}/>
  <g clip-path="url(#body)">
    <g transform="${zoom}">
      <use href="#scene"/>
      ${rr({ ...FRONT, y: FRONT.y + 22 }, `fill="#1a0a00" opacity="0.28" filter="url(#soft)"`)}
      <g clip-path="url(#front)">
        <use href="#scene" filter="url(#frost)"/>
        ${rr(FRONT, `fill="url(#frostFill)"`)}
        ${rr(FRONT, `fill="url(#sheen)"`)}
      </g>
      ${rr(inset(FRONT, 1.5), `fill="none" stroke="url(#rim)" stroke-width="${3 / scale}"`)}
    </g>
    <path d="${body}" fill="none" stroke="url(#bodyRim)" stroke-width="6"/>
  </g>
</svg>
`;
}

mkdirSync(outDir, { recursive: true });
const out = {
  "outerspace.svg": icon({ tint: "on", bodyRadius: 412, scale: 1, shadow: true }),
  "tray-on.svg": icon({ tint: "on", bodyRadius: 500, scale: 1.3, shadow: false }),
  "tray-off.svg": icon({ tint: "off", bodyRadius: 500, scale: 1.3, shadow: false }),
  "tray-error.svg": icon({ tint: "error", bodyRadius: 500, scale: 1.3, shadow: false }),
};
for (const [name, svg] of Object.entries(out)) {
  writeFileSync(join(outDir, name), svg);
  console.log("wrote", join(outDir, name));
}
