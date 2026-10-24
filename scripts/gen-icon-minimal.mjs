// Generates the minimal "virtual display" icon candidates (1024x1024 SVG, macOS-style superellipse body).
// Usage: node scripts/gen-icon-minimal.mjs [outDir]   -> writes outerspace-<name>.svg for every candidate
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const outDir = process.argv[2] ?? "assets/icon/minimal";

function squircle(cx, cy, a, n, steps = 360) {
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

const BODY = squircle(512, 512, 412, 5);
const BG = { light: "url(#bgLight)", dark: "url(#bgDark)", orange: "url(#bgOrange)" };

const rr = ({ x, y, w, h, r }, attrs = "") => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}" ${attrs}/>`;
const inset = (b, d) => ({ x: b.x + d, y: b.y + d, w: b.w - 2 * d, h: b.h - 2 * d, r: b.r - d });

// a glass slab: translucent body, specular sheen on top, bright rim
function glass(b, { fill = "url(#glassFill)", shadow = true } = {}) {
  return `
    ${rr(b, `fill="${fill}"${shadow ? ` filter="url(#drop)"` : ""}`)}
    ${rr(b, `fill="url(#sheen)"`)}
    ${rr(inset(b, 1.5), `fill="none" stroke="url(#rim)" stroke-width="3"`)}`;
}

// a glass slab that blurs whatever is drawn in <g id=sceneId> behind it
function frosted(b, sceneId, id, fill = "url(#frostFill)") {
  return `
    <clipPath id="${id}">${rr(b)}</clipPath>
    ${rr({ ...b, y: b.y + 22 }, `fill="#1a0a00" opacity="0.28" filter="url(#soft)"`)}
    <g clip-path="url(#${id})">
      <use href="#${sceneId}" filter="url(#frost)"/>
      ${rr(b, `fill="${fill}"`)}
      ${rr(b, `fill="url(#sheen)"`)}
    </g>
    ${rr(inset(b, 1.5), `fill="none" stroke="url(#rim)" stroke-width="3"`)}`;
}

// solid Golden Gate orange slab with a glassy top sheen
function orangeSlab(b, { shadow = true } = {}) {
  return `
    ${rr(b, `fill="url(#orange)"${shadow ? ` filter="url(#drop)"` : ""}`)}
    ${rr(b, `fill="url(#sheen)"`)}
    ${rr(inset(b, 1.5), `fill="none" stroke="url(#rimWarm)" stroke-width="3"`)}`;
}

const variants = {
  // 1 — a real screen and its virtual twin behind it, seen through frosted glass
  twin() {
    const back = { x: 352, y: 312, w: 420, h: 268, r: 46 };
    const front = { x: 252, y: 444, w: 420, h: 268, r: 46 };
    return {
      bg: "dark",
      scene: `<g id="scene">${rr({ x: 0, y: 0, w: 1024, h: 1024, r: 0 }, `fill="${BG.dark}"`)}${orangeSlab(back, { shadow: false })}</g>`,
      body: `<use href="#scene"/>${frosted(front, "scene", "frontClip")}`,
    };
  },

  // 2 — dashed screen on a solid stand: the hardware is real, the display is not
  dashed() {
    const s = { x: 232, y: 282, w: 560, h: 356, r: 52 };
    return {
      bg: "light",
      body: `
    ${rr(s, `fill="#fff" fill-opacity="0.55" filter="url(#dropLight)"`)}
    ${rr(inset(s, 10), `fill="none" stroke="url(#orange)" stroke-width="22" stroke-linecap="round" pathLength="100" stroke-dasharray="6 4" stroke-dashoffset="-2"`)}
    <path d="M512 668 V722 M422 734 H602" stroke="url(#orange)" stroke-width="24" stroke-linecap="round" fill="none"/>`,
    };
  },

  // 3 — your screen in front, the virtual one peeking in from outside the frame
  peek() {
    const peek = { x: 600, y: 206, w: 380, h: 240, r: 40 };
    const main = { x: 196, y: 362, w: 480, h: 304, r: 46 };
    return {
      bg: "orange",
      body: `${glass(peek, { fill: "url(#glassFillThin)" })}${glass(main, { fill: "url(#glassFillSolid)" })}`,
    };
  },

  // 4 — an empty glass screen with the agent as a warm point of light
  presence() {
    const s = { x: 212, y: 312, w: 600, h: 380, r: 58 };
    return {
      bg: "dark",
      body: `
    <clipPath id="screenClip">${rr(s)}</clipPath>
    ${glass(s, { fill: "url(#glassFillDark)" })}
    <g clip-path="url(#screenClip)">
      <circle cx="512" cy="502" r="170" fill="#FF7B32" opacity="0.55" filter="url(#glow)"/>
    </g>
    <circle cx="512" cy="502" r="54" fill="url(#orange)"/>
    <ellipse cx="496" cy="482" rx="24" ry="14" fill="#fff" opacity="0.55"/>`,
    };
  },

  // 5 — displays layered in depth
  stack() {
    const a = { x: 372, y: 288, w: 420, h: 268, r: 42 };
    const b = { x: 302, y: 378, w: 420, h: 268, r: 42 };
    const c = { x: 232, y: 468, w: 420, h: 268, r: 42 };
    return {
      bg: "light",
      body: `${glass(a, { fill: "url(#glassFillThin)" })}${glass(b, { fill: "url(#glassFill)" })}${orangeSlab(c)}`,
    };
  },

  // 6 — half solid, half outline: physical | virtual
  split() {
    const s = { x: 212, y: 322, w: 600, h: 380, r: 58 };
    return {
      bg: "dark",
      body: `
    <clipPath id="leftHalf"><rect x="0" y="0" width="512" height="1024"/></clipPath>
    ${rr(inset(s, 9), `fill="#fff" fill-opacity="0.06" stroke="url(#orange)" stroke-width="18"`)}
    <g clip-path="url(#leftHalf)">${orangeSlab(s, { shadow: false })}</g>`,
    };
  },

  // 7 — a screen behind a frosted pane: the work happens out of focus
  frosted() {
    const screen = { x: 262, y: 226, w: 500, h: 318, r: 46 };
    const pane = { x: 190, y: 430, w: 644, h: 336, r: 68 };
    return {
      bg: "light",
      scene: `<g id="scene">
      ${rr({ x: 0, y: 0, w: 1024, h: 1024, r: 0 }, `fill="${BG.light}"`)}
      ${orangeSlab(screen, { shadow: false })}
      <path d="M512 544 V610 M412 622 H612" stroke="#E0502A" stroke-width="28" stroke-linecap="round" fill="none"/>
    </g>`,
      body: `<use href="#scene"/>${frosted(pane, "scene", "paneClip", "url(#frostFillLight)")}`,
    };
  },

  // 8 — viewfinder corners framing a glow: a display that only exists as a frame
  brackets() {
    const x0 = 232, x1 = 792, y0 = 332, y1 = 692, L = 118, r = 44;
    const corner = (x, y, dx, dy) =>
      `M${x} ${y + dy * L} V${y + dy * r} Q${x} ${y} ${x + dx * r} ${y} H${x + dx * L}`;
    const d = [corner(x0, y0, 1, 1), corner(x1, y0, -1, 1), corner(x0, y1, 1, -1), corner(x1, y1, -1, -1)].join(" ");
    return {
      bg: "dark",
      body: `
    <ellipse cx="512" cy="512" rx="210" ry="130" fill="#FF7B32" opacity="0.6" filter="url(#glow)"/>
    <path d="${d}" fill="none" stroke="url(#bracket)" stroke-width="30" stroke-linecap="round" stroke-linejoin="round" filter="url(#drop)"/>`,
    };
  },

  // 9 — a glass screen and its reflection: a copy that isn't really there
  reflection() {
    const bezel = { x: 262, y: 250, w: 500, h: 316, r: 48 };
    const shot = (id) => `<g id="${id}">${glass(bezel, { fill: "url(#glassFillDark)", shadow: false })}${orangeSlab(inset(bezel, 20), { shadow: false })}</g>`;
    return {
      bg: "dark",
      body: `
    <linearGradient id="fade" x1="0" y1="586" x2="0" y2="740" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity="0.3"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>
    <mask id="fadeMask" maskUnits="userSpaceOnUse" x="0" y="0" width="1024" height="1024"><rect x="0" y="586" width="1024" height="438" fill="url(#fade)"/></mask>
    <g mask="url(#fadeMask)"><g transform="translate(0 1172) scale(1 -1)">${shot("mirror")}</g></g>
    ${shot("screen")}`,
    };
  },

  // 10 — send a window to the other display
  send() {
    const s = { x: 186, y: 340, w: 470, h: 300, r: 46 };
    return {
      bg: "orange",
      body: `
    ${glass(s, { fill: "url(#glassFillThin)" })}
    <path d="M440 490 H782 M712 418 L790 490 L712 562" fill="none" stroke="#fff" stroke-width="46" stroke-linecap="round" stroke-linejoin="round" filter="url(#drop)"/>`,
    };
  },
};

const defs = `
    <clipPath id="body"><path d="${BODY}"/></clipPath>
    <linearGradient id="bgLight" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFF9F4"/><stop offset="1" stop-color="#F2DCCD"/></linearGradient>
    <linearGradient id="bgDark" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2C2742"/><stop offset="1" stop-color="#100E1B"/></linearGradient>
    <linearGradient id="bgOrange" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#FF9E52"/><stop offset="1" stop-color="#E2502A"/></linearGradient>
    <linearGradient id="orange" x1="0" y1="0" x2="0.25" y2="1"><stop offset="0" stop-color="#FFB86E"/><stop offset="0.55" stop-color="#FF7B32"/><stop offset="1" stop-color="#E0502A"/></linearGradient>
    <linearGradient id="glassFill" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.7"/><stop offset="1" stop-color="#fff" stop-opacity="0.38"/></linearGradient>
    <linearGradient id="glassFillThin" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.42"/><stop offset="1" stop-color="#fff" stop-opacity="0.14"/></linearGradient>
    <linearGradient id="glassFillSolid" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.97"/><stop offset="1" stop-color="#FFF1E8" stop-opacity="0.86"/></linearGradient>
    <linearGradient id="glassFillDark" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.16"/><stop offset="1" stop-color="#fff" stop-opacity="0.05"/></linearGradient>
    <linearGradient id="frostFill" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.26"/><stop offset="1" stop-color="#fff" stop-opacity="0.08"/></linearGradient>
    <linearGradient id="frostFillLight" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.5"/><stop offset="1" stop-color="#fff" stop-opacity="0.3"/></linearGradient>
    <linearGradient id="sheen" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.42"/><stop offset="0.32" stop-color="#fff" stop-opacity="0"/></linearGradient>
    <linearGradient id="rim" x1="0" y1="0" x2="0.5" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="1"/><stop offset="0.5" stop-color="#fff" stop-opacity="0.2"/><stop offset="1" stop-color="#fff" stop-opacity="0.6"/></linearGradient>
    <linearGradient id="rimWarm" x1="0" y1="0" x2="0.5" y2="1"><stop offset="0" stop-color="#FFE3C4" stop-opacity="1"/><stop offset="0.5" stop-color="#FFE3C4" stop-opacity="0.15"/><stop offset="1" stop-color="#FFE3C4" stop-opacity="0.5"/></linearGradient>
    <linearGradient id="bracket" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#FFD0AE"/></linearGradient>
    <linearGradient id="bodyRimDark" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.45"/><stop offset="0.3" stop-color="#fff" stop-opacity="0.06"/><stop offset="1" stop-color="#fff" stop-opacity="0.12"/></linearGradient>
    <linearGradient id="bodyRimLight" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.95"/><stop offset="0.3" stop-color="#fff" stop-opacity="0.2"/><stop offset="1" stop-color="#fff" stop-opacity="0.35"/></linearGradient>
    <filter id="drop" x="-30%" y="-30%" width="160%" height="170%"><feDropShadow dx="0" dy="18" stdDeviation="22" flood-color="#2a0d00" flood-opacity="0.3"/></filter>
    <filter id="dropLight" x="-30%" y="-30%" width="160%" height="170%"><feDropShadow dx="0" dy="14" stdDeviation="20" flood-color="#8a3a10" flood-opacity="0.14"/></filter>
    <filter id="soft" x="-30%" y="-30%" width="160%" height="170%"><feGaussianBlur stdDeviation="22"/></filter>
    <filter id="frost" x="-10%" y="-10%" width="120%" height="120%"><feGaussianBlur stdDeviation="26"/></filter>
    <filter id="glow" x="-80%" y="-80%" width="260%" height="260%"><feGaussianBlur stdDeviation="42"/></filter>
    <filter id="iconShadow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity="0.3"/></filter>`;

function render(v) {
  const rim = v.bg === "dark" ? "url(#bodyRimDark)" : "url(#bodyRimLight)";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <title>OuterSpace</title>
  <defs>${defs}
    ${v.scene ?? ""}
  </defs>
  <g filter="url(#iconShadow)"><path d="${BODY}" fill="${BG[v.bg]}"/></g>
  <g clip-path="url(#body)">
    <rect width="1024" height="1024" fill="${BG[v.bg]}"/>
    ${v.body}
    <path d="${BODY}" fill="none" stroke="${rim}" stroke-width="6"/>
  </g>
</svg>
`;
}

mkdirSync(outDir, { recursive: true });
for (const [name, make] of Object.entries(variants)) {
  const file = join(outDir, `outerspace-${name}.svg`);
  writeFileSync(file, render(make()));
  console.log("wrote", file);
}
