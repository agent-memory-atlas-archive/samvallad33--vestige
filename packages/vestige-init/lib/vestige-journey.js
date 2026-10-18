#!/usr/bin/env node
// VESTIGE v5 · PRECEDENT — follow one real stop around the closed loop.
//
// The map is the loop: six stations on a ring. One stop from this machine travels around it:
//   1 THE STOP        what the agent tried, and the gate stopping it
//   2 THE RECORD      every receipt re-derived from its own bytes; try to forge one
//   3 THE MEMORY      the stop as a Vestige memory, and the rule memory it derives from
//   4 THE PRECEDENT   every earlier case under the same rule, the permit state, the exact precedent() call
//   5 THE AFTERMATH   what the agent did next, and whether the action ever came back
//   6 THE VERDICT     the card, to share
// Everything on screen is read from this machine. Nothing is written except the journey's own progress
// file, and the images you ask it to save.

const fs = require('fs');
const os = require('os');
const path = require('path');
const stage = require('./vestige-stage.js');
const precedent = require('./vestige-precedent.js');
const story = require('./vestige-story.js');
const booking = require('./vestige-booking.js');
const replay = require('./vestige-replay.js');
const { FONT } = require('./vestige-font.js');

const { Canvas, mix, chars } = stage;

const BRAND_A = [90, 120, 255];
const BRAND_B = [190, 80, 255];
const TEXT = [222, 228, 240];
const SOFT = [140, 148, 178];
const FAINT = [92, 98, 128];
const RED = [255, 96, 88];
const GREEN = [96, 228, 150];
const AMBER = [255, 196, 96];
const WHITE = [250, 252, 255];

const THEMES = {
  title: { top: [12, 12, 34], bottom: [3, 3, 10], accent: BRAND_B, star: [150, 160, 220] },
  map: { top: [12, 14, 36], bottom: [3, 4, 12], accent: BRAND_A, star: [130, 140, 200] },
  stop: { top: [26, 20, 52], bottom: [9, 8, 22], accent: RED, star: [170, 150, 210] },
  record: { top: [6, 34, 32], bottom: [2, 10, 12], accent: [90, 236, 178], star: [80, 190, 150] },
  memory: { top: [36, 14, 56], bottom: [10, 5, 22], accent: [206, 148, 255], star: [170, 120, 230] },
  precedent: { top: [42, 30, 8], bottom: [12, 8, 3], accent: [255, 204, 104], star: [200, 160, 90] },
  aftermath: { top: [6, 30, 48], bottom: [2, 10, 18], accent: [124, 222, 255], star: [100, 170, 220] },
  verdict: { top: [24, 26, 62], bottom: [9, 9, 24], accent: BRAND_B, star: [150, 150, 220] },
};

const STATIONS = [
  { id: 'stop', name: 'THE STOP', short: 'STOP', line: 'what your agent tried' },
  { id: 'record', name: 'THE RECORD', short: 'RECORD', line: 'every receipt, re-derived from its own bytes' },
  { id: 'memory', name: 'THE MEMORY', short: 'MEMORY', line: 'the stop became a memory your agents recall' },
  { id: 'precedent', name: 'THE PRECEDENT', short: 'PRECEDENT', line: 'every earlier case under the same rule' },
  { id: 'aftermath', name: 'THE AFTERMATH', short: 'AFTERMATH', line: 'what the agent did after it was stopped' },
  { id: 'verdict', name: 'THE VERDICT', short: 'VERDICT', line: 'the card: proof that travels' },
];

const FRAME_MS = 70;
const HOLD_TICK_MS = 140;
const CLOSING = 'a remembered yes is not permission today.';

// ── small helpers ───────────────────────────────────────────────────────────────────────────────
const fmt = (n) => Number(n || 0).toLocaleString('en-US');
const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];
// a time that is not a time reads as '?', never as 'undefined NaN'
const validTime = (ts) => ts !== null && ts !== undefined && !Number.isNaN(new Date(ts).getTime());
const monthDay = (ts) => { if (!validTime(ts)) return '?'; const d = new Date(ts); return `${MONTHS[d.getMonth()]} ${d.getDate()}`; };
const clock = (ts) => { if (!validTime(ts)) return '?'; const d = new Date(ts); return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`; };
const fit = (text, width) => { const list = chars(text); return width <= 0 ? '' : list.length <= width ? list.join('') : list.slice(0, Math.max(0, width - 1)).join('') + '…'; };
const ease = (t) => (t <= 0 ? 0 : t >= 1 ? 1 : t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);
const clamp01 = (t) => Math.max(0, Math.min(1, t));
function ago(ts, now = Date.now()) {
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 90) return 'just now';
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 129600) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}
function wrap(text, width) {
  const out = [];
  let line = '';
  for (const word of String(text).split(/\s+/).filter(Boolean)) {
    if (!line) line = word;
    else if (chars(line + ' ' + word).length <= width) line += ' ' + word;
    else { out.push(line); line = word; }
  }
  if (line) out.push(line);
  return out.map((l) => fit(l, width));
}
// Types a string in over frames: how much of it shows at frame f.
const typed = (text, f, start, perFrame = 3, last = Infinity) => { const list = chars(text); if (f >= last) return list.join(''); const n = Math.max(0, Math.min(list.length, Math.floor((f - start) * perFrame))); return list.slice(0, n).join(''); };

function layout(cols, rows) {
  const w = Math.min(cols, 96);
  const h = Math.min(rows, 32);
  const x = Math.floor((cols - w) / 2);
  const y = Math.floor((rows - h) / 2);
  const compact = w < 78 || h < 24;
  const cw = compact ? w - 4 : Math.min(w - 4, 80);
  const chh = h - 4;
  const ox = x + Math.floor((w - cw) / 2);
  const designed = compact ? chh : 20;
  const oy = y + 2 + Math.max(0, Math.floor((chh - designed) / 2));
  return { cols, rows, x, y, w, h, compact, ox, oy, cw, ch: compact ? chh : 20, top: y, bottom: y + h - 1 };
}
const tooSmall = (cols, rows) => cols < 60 || rows < 20;

// ── backdrop: a sky per station, stars only where no word or picture lives ─────────────────────
function backdrop(cv, theme, seed, tick, { density = 1, rain = null } = {}) {
  for (let y = 0; y < cv.rows; y++) {
    const bg = mix(theme.top, theme.bottom, cv.rows > 1 ? y / (cv.rows - 1) : 0);
    for (let x = 0; x < cv.cols; x++) { const cell = cv.cells[y * cv.cols + x]; cell.bg = bg; cell.ch = ' '; cell.fg = null; cell.bold = false; cell.dim = false; cell.rev = false; }
  }
  const rand = stage.seeded(seed);
  const count = Math.round((cv.cols * cv.rows) / 70 * density);
  for (let i = 0; i < count; i++) {
    const x = Math.floor(rand() * cv.cols);
    const y = Math.floor(rand() * cv.rows);
    const phase = Math.floor(rand() * 40);
    const kind = rand();
    if (!cv.free(x, y)) continue;
    const t = ((tick + phase) % 40) / 40;
    const glow = 0.25 + 0.75 * Math.abs(Math.sin(t * Math.PI));
    const ch = kind < 0.08 ? '+' : kind < 0.35 ? '·' : '.';
    cv.put(x, y, ch, { fg: mix(theme.bottom, theme.star, glow), bg: cv.at(x, y).bg });
  }
  if (rain) rain(cv, tick);
}

// A column of real digest characters falling, used behind the title and the record.
function hashRain(pool, theme, { strength = 1, onlyFree = true, speedSeed = 7, every = 2 } = {}) {
  return (cv, tick) => {
    const rand = stage.seeded(speedSeed);
    for (let x = 0; x < cv.cols; x += every) {
      const speed = 0.35 + rand() * 0.9;
      const len = 4 + Math.floor(rand() * 9);
      const offset = Math.floor(rand() * cv.rows * 3);
      const head = Math.floor((tick * speed + offset) % (cv.rows + len * 2)) - len;
      const source = pool[Math.floor(x / every) % pool.length] || '0123456789abcdef';
      for (let k = 0; k < len; k++) {
        const y = head - k;
        if (y < 0 || y >= cv.rows || (onlyFree && !cv.free(x, y))) continue;
        const ch = source[(y + x + Math.floor(tick / 3)) % source.length];
        const t = k === 0 ? 1 : 1 - k / len;
        const col = k === 0 ? mix(theme.accent, WHITE, 0.4) : mix(cv.at(x, y).bg, theme.accent, 0.12 + 0.55 * t * strength);
        cv.put(x, y, ch, { fg: col, bg: cv.at(x, y).bg, bold: k === 0 });
      }
    }
  };
}

// ── chrome: the bar above and the keys below ───────────────────────────────────────────────────
// Called first in every scene: the bars, and a margin round the content, are never decorated.
function prepare(cv, L) {
  cv.reserve(0, L.top - 1, cv.cols - 1, L.top + 1);
  cv.reserve(0, L.bottom - 1, cv.cols - 1, L.bottom + 1);
}
const pad = (cv, x0, y0, x1, y1, p = 2) => cv.reserve(x0 - p, y0 - 1, x1 + p, y1 + 1);
function chrome(cv, L, ctx, { station = null, keys = [] } = {}) {
  const y0 = L.top;
  const y1 = L.bottom;
  const left = L.x + 2;
  const right = L.x + L.w - 2;
  cv.gradient(left, y0, 'VESTIGE', BRAND_A, BRAND_B, { bold: true });
  cv.put(left + 8, y0, 'v5 · PRECEDENT', { fg: SOFT });
  if (ctx.c) {
    const dots = STATIONS.map((st, i) => (ctx.visited.has(i) || i === station ? '●' : '○')).join('');
    const label = station === null ? `case ${fmt(ctx.c.number)}` : `case ${fmt(ctx.c.number)} · ${station + 1}/6 ${L.compact ? '' : STATIONS[station].name}`.trimEnd();
    const text = `${label}  `;
    const startX = right - chars(text).length - 6;
    cv.put(startX, y0, text, { fg: SOFT });
    STATIONS.forEach((_st, i) => cv.put(startX + chars(text).length + i, y0, dots[i], { fg: i === station ? WHITE : ctx.visited.has(i) ? (THEMES[STATIONS[i].id].accent) : FAINT, bold: i === station }));
  }
  if (ctx.toast && ctx.toast.until > Date.now()) {
    // a toast takes the bar's place for a moment; two texts never share the row
    cv.put(left, y1, fit(ctx.toast.text, L.w - 4), { fg: ctx.toast.bad ? RED : GREEN, bold: true });
    return;
  }
  // the last key (leave) always shows; others drop from the end of the middle until the row fits
  const list = keys.slice();
  const width = (ks) => chars(ks.join('   ')).length;
  while (list.length > 2 && width(list) > L.w - 4) list.splice(list.length - 2, 1);
  let x = left;
  list.forEach((k, i) => {
    if (i) x += 3;
    const space = k.indexOf(' ');
    const name = space > 0 ? k.slice(0, space) : k;
    x = cv.put(x, y1, name, { fg: WHITE, bold: true });
    if (space > 0) x = cv.put(x, y1, k.slice(space), { fg: SOFT });
  });
}

const KEYS_STATION = ['enter next', '← back', 'm map', '? keys', 'q leave'];

// ── big letters out of real digests ────────────────────────────────────────────────────────────
// Letters from the bundle's 5x7 face, two pixel rows per cell (half blocks), at scale 1 or 2.
function bigWordCells(word, scale = 1) {
  const pixels = new Set();
  let px = 0;
  for (const letter of chars(word)) {
    const glyph = FONT.get(letter);
    if (glyph) glyph.slice(0, 7).forEach((row, y) => row.forEach((on, x) => {
      if (!on) return;
      for (let dx = 0; dx < scale; dx++) for (let dy = 0; dy < scale; dy++) pixels.add(`${px + x * scale + dx},${y * scale + dy}`);
    }));
    px += 6 * scale;
  }
  const width = px - scale;
  const height = Math.ceil((7 * scale) / 2);
  const cells = [];
  for (let cy = 0; cy < height; cy++) for (let x = 0; x < width; x++) {
    const top = pixels.has(`${x},${cy * 2}`);
    const bottom = pixels.has(`${x},${cy * 2 + 1}`);
    if (top || bottom) cells.push({ x, y: cy, ch: top && bottom ? '█' : top ? '▀' : '▄' });
  }
  return { cells, width, height };
}

// ── title ──────────────────────────────────────────────────────────────────────────────────────
const TITLE_FRAMES = 46;
function renderTitle(cv, L, ctx, f, tick = 0) {
  const scale = cv.cols >= 116 && cv.rows >= 30 ? 2 : 1;
  const word = bigWordCells('PRECEDENT', scale);
  const wx = Math.floor((cv.cols - word.width) / 2);
  const wy = L.oy + (L.compact ? 0 : scale === 2 ? 0 : 2);
  const textTop = wy + word.height + 2;
  prepare(cv, L);
  // the word and the words under it are reserved: rain falls around them, never on them
  pad(cv, wx, wy, wx + word.width - 1, wy + word.height - 1);
  pad(cv, L.ox, textTop, L.ox + L.cw - 1, Math.min(L.bottom - 2, textTop + 9));
  const rainOn = hashRain(ctx.pool, THEMES.title, { strength: f < 26 ? 0.8 : Math.max(0.12, 0.8 - (f - 26) / 16), every: f < 30 ? 3 : 5 });
  backdrop(cv, THEMES.title, 11, tick + f, { density: 0.5, rain: rainOn });
  chrome(cv, L, ctx, { keys: f >= TITLE_FRAMES - 1 ? (ctx.c ? ['enter follow the case', ctx.cases.length > 1 ? 'n another case' : '', '? keys', 'q leave'].filter(Boolean) : ['q leave']) : ['any key skips'] });
  // The receipts' own digits fall into the letters, then each cell sets solid.
  const pool = ctx.pool.join('');
  word.cells.forEach(({ x, y, ch }, i) => {
    const lockAt = 4 + (x / word.width) * 16 + (y % 2) * 2;
    const at = cv.at(wx + x, wy + y);
    if (!at) return;
    if (f < lockAt) {
      if (f >= lockAt - 5) cv.put(wx + x, wy + y, pool[(i * 7 + x + f) % pool.length] || '0', { fg: mix(THEMES.record.accent, WHITE, 0.3), bg: at.bg, bold: true });
      return;
    }
    const settle = clamp01((f - lockAt) / 5);
    const shimmer = Math.max(0, Math.sin((x / word.width) * Math.PI * 2 - tick * 0.22));
    const base = mix(BRAND_A, BRAND_B, x / word.width);
    const color = settle < 1 ? mix(WHITE, base, settle) : mix(base, WHITE, shimmer * shimmer * 0.35);
    cv.put(wx + x, wy + y, settle < 0.4 ? (pool[(i * 3 + f) % pool.length] || '0') : ch, { fg: color, bg: at.bg, bold: true });
  });
  const mid = (text) => Math.floor((cv.cols - chars(text).length) / 2);
  const lines = [];
  lines.push(['v5', 26, { fg: WHITE, bold: true }]);
  lines.push(['the guard that remembers', 28, { fg: TEXT }]);
  if (ctx.c) {
    const s = ctx.c.story;
    lines.push(['', 0, {}]);
    lines.push([L.compact ? `case ${fmt(ctx.c.number)} · ${ago(s.ts)} · ${s.source} · ${s.commitment}` : `case ${fmt(ctx.c.number)} · ${ago(s.ts)} · ${s.source} was stopped by ${s.commitment}`, 32, { fg: SOFT }]);
    lines.push([fit(`$ ${s.action}`, L.cw), 34, { fg: TEXT, bold: true }]);
    lines.push(['', 0, {}]);
    lines.push(['follow it around the loop', 39, { fg: THEMES.title.accent, bold: true }]);
  } else {
    lines.push(['', 0, {}]);
    lines.push([ctx.empty || 'no cases on this machine yet', 32, { fg: TEXT, bold: true }]);
    lines.push(['a case appears the first time Operator stops one of your agents', 34, { fg: SOFT }]);
    if (ctx.log && ctx.log.records.length) lines.push([`${fmt(ctx.log.records.length)} receipts on the chain so far, every one checked`, 36, { fg: SOFT }]);
    lines.push(['', 0, {}]);
    lines.push(['set up Vestige and Operator:  npx @vestige/init', 38, { fg: THEMES.title.accent, bold: true }]);
  }
  if (ctx.since) lines.push([ctx.since, 41, { fg: FAINT }]);
  lines.forEach(([raw, start, style], i) => {
    if (!raw) return;
    const text = fit(raw, cv.cols - 4);
    const shown = typed(text, f, start, 6, TITLE_FRAMES - 1);
    if (!shown) return;
    cv.put(mid(text), textTop + i, shown, { ...style, bg: undefined });
  });
  if (f >= TITLE_FRAMES - 1 && ctx.c) {
    const blink = Math.floor(tick / 4) % 2 === 0;
    const y = textTop + lines.length + 1;
    if (y < L.bottom) cv.put(mid('press enter'), y, 'press enter', { fg: blink ? WHITE : SOFT, bold: blink });
  }
}

// ── the ring map ───────────────────────────────────────────────────────────────────────────────
const ANGLES = [-90, -30, 30, 90, 150, 210].map((d) => (d * Math.PI) / 180);
function ringGeometry(L) {
  const mx = L.ox + Math.floor(L.cw / 2);
  const my = L.oy + Math.floor(L.ch / 2);
  // side labels sit above or below their badge, so the ring can use the width it has
  const rx = Math.min(L.compact ? 24 : 32, Math.floor(L.cw / 2) - 6);
  const ry = Math.min(L.compact ? 6 : 8, Math.floor(L.ch / 2) - 2);
  const at = (a) => ({ x: Math.round(mx + rx * Math.cos(a)), y: Math.round(my + ry * Math.sin(a)) });
  return { mx, my, rx, ry, at, nodes: ANGLES.map(at) };
}

function renderMap(cv, L, ctx, { from = null, to = 0, t = 1, tick = 0, finale = false, travelling = false } = {}) {
  const g = ringGeometry(L);
  prepare(cv, L);
  const placements = g.nodes.map((p, i) => {
    const label = STATIONS[i].short;
    const side = Math.abs(Math.cos(ANGLES[i])) > 0.5;
    const upper = Math.sin(ANGLES[i]) < 0;
    // top and bottom badges carry the label beside them; the four side badges above or below
    const lx = side ? p.x - Math.floor(chars(label).length / 2) : p.x + 5;
    const ly = side ? (upper ? p.y - 2 : p.y + 2) : p.y;
    return { ...p, label, lx, ly };
  });
  placements.forEach((p) => { cv.reserve(p.x - 3, p.y - 1, p.x + 3, p.y + 1); cv.reserve(p.lx - 1, p.ly, p.lx + chars(p.label).length, p.ly); });
  const centerW = Math.max(24, Math.min(Math.floor(2 * (0.866 * g.rx - 4)) - 2, 52));
  cv.reserve(g.mx - Math.floor(centerW / 2) - 1, g.my - 3, g.mx + Math.ceil(centerW / 2) + 1, g.my + 3);
  backdrop(cv, THEMES.map, 21, tick, { density: 0.8 });
  const keys = travelling ? ['any key skips', 'q leave'] : finale ? ['enter next case', 'r again', 'q leave'] : ['← → choose', 'enter go', '1-6 jump', 'esc close', 'q leave'];
  chrome(cv, L, ctx, { keys });

  // the ring: lit where the case has already travelled
  const steps = Math.round((g.rx + g.ry) * 5);
  for (let k = 0; k < steps; k++) {
    const a = -Math.PI / 2 + (k / steps) * Math.PI * 2;
    const x = Math.round(g.mx + g.rx * Math.cos(a));
    const y = Math.round(g.my + g.ry * Math.sin(a));
    if (!cv.free(x, y)) continue;
    const seg = Math.floor(((a + Math.PI / 2) / (Math.PI * 2)) * 6) % 6;
    const lit = ctx.visited.has(seg) && ctx.visited.has((seg + 1) % 6);
    const pulse = 0.5 + 0.5 * Math.sin(k * 0.35 - tick * 0.5);
    const col = lit ? mix(THEMES[STATIONS[seg].id].accent, WHITE, finale ? pulse * 0.4 : 0.1) : mix(THEMES.map.bottom, SOFT, 0.75);
    cv.put(x, y, lit ? '•' : '·', { fg: col, bg: cv.at(x, y).bg, bold: lit });
  }

  // the comet, travelling from one station to the next
  if (from !== null && t < 1) {
    let a0 = ANGLES[from];
    let a1 = ANGLES[to];
    if (to === 0 && from === 5) a1 += Math.PI * 2;
    if (a1 < a0 && !(from === 5 && to === 0)) a1 = a0 - Math.abs(a1 - a0); // going back travels backwards
    for (let k = 5; k >= 0; k--) {
      const tt = clamp01(ease(t) - k * 0.035);
      const a = a0 + (a1 - a0) * tt;
      const x = Math.round(g.mx + g.rx * Math.cos(a));
      const y = Math.round(g.my + g.ry * Math.sin(a));
      const cell = cv.at(x, y);
      if (!cell) continue;
      cv.put(x, y, k === 0 ? '●' : k < 3 ? '•' : '·', { fg: mix(THEMES[STATIONS[to].id].accent, WHITE, k === 0 ? 0.6 : 0.1), bg: cell.bg, bold: k === 0 });
    }
  }

  placements.forEach((p, i) => {
    const theme = THEMES[STATIONS[i].id];
    const here = !finale && i === to && (from === null || t >= 1);
    const visited = ctx.visited.has(i);
    const edge = here ? WHITE : visited ? theme.accent : SOFT;
    if (here) { cv.put(p.x - 3, p.y, '▸', { fg: WHITE, bold: true }); cv.put(p.x + 3, p.y, '◂', { fg: WHITE, bold: true }); }
    const glow = finale ? 0.5 + 0.5 * Math.sin(tick * 0.4 + i) : 0;
    cv.put(p.x - 2, p.y - 1, '╭───╮', { fg: mix(edge, WHITE, glow * 0.5), bold: here });
    cv.put(p.x - 2, p.y, '│', { fg: edge, bold: here });
    cv.put(p.x - 1, p.y, ` ${i + 1} `, { fg: here ? [10, 10, 20] : visited ? WHITE : SOFT, bg: here ? theme.accent : undefined, bold: true });
    cv.put(p.x + 2, p.y, '│', { fg: edge, bold: here });
    cv.put(p.x - 2, p.y + 1, '╰───╯', { fg: mix(edge, WHITE, glow * 0.5), bold: here });
    cv.put(p.lx, p.ly, p.label, { fg: here ? WHITE : visited ? theme.accent : SOFT, bold: here || visited });
  });

  const center = (text, y, style) => cv.put(g.mx - Math.floor(chars(text).length / 2), y, text, style);
  const unseen = STATIONS.map((_st, i) => i).filter((i) => !ctx.visited.has(i));
  if (finale && unseen.length) {
    center('THE LOOP IS CLOSED', g.my - 2, { fg: WHITE, bold: true });
    center(fit(`not seen yet: ${unseen.map((i) => i + 1).join(' · ')}`, centerW), g.my, { fg: AMBER });
    center(fit('r goes round again', centerW), g.my + 1, { fg: SOFT });
    const closing = centerW >= chars(CLOSING).length ? [CLOSING] : ['a remembered yes is', 'not permission today.'];
    closing.forEach((w, i) => center(w, g.my + 2 + i, { fg: THEMES.verdict.accent, bold: true }));
  } else if (finale) {
    center('THE LOOP IS CLOSED', g.my - 2, { fg: WHITE, bold: true });
    const words = centerW >= 44 ? ['stopped · recorded · remembered · ruled on'] : ['stopped · recorded', 'remembered · ruled on'];
    words.forEach((w, i) => center(w, g.my + i, { fg: TEXT }));
    const closing = centerW >= chars(CLOSING).length ? [CLOSING] : ['a remembered yes is', 'not permission today.'];
    closing.forEach((w, i) => center(w, g.my + 2 + i, { fg: THEMES.verdict.accent, bold: true }));
  } else if (ctx.c) {
    const s = ctx.c.story;
    center(fit(`case ${fmt(ctx.c.number)} · ${s.source} · ${clock(s.ts)}`, centerW), g.my - 2, { fg: TEXT, bold: true });
    center(fit(`${s.commitment}${s.rule ? ' · ' + s.rule : ''}`, centerW), g.my - 1, { fg: SOFT });
    const next = STATIONS[to];
    center(fit(`${to + 1} ${next.name}`, centerW), g.my + 1, { fg: THEMES[next.id].accent, bold: true });
    wrap(next.line, centerW).slice(0, 2).forEach((w, i) => center(w, g.my + 2 + i, { fg: SOFT }));
  }
}

// ── station 1: THE STOP ────────────────────────────────────────────────────────────────────────
const STOP_FRAMES = 50;
function renderStop(cv, L, ctx, f, tick) {
  const s = ctx.c.story;
  if (!L.compact && cv.cols >= booking.STAGE_COLS && cv.rows >= booking.STAGE_ROWS + 2) {
    if (!ctx.stopFrames || ctx.stopFrames.cols !== cv.cols || ctx.stopFrames.rows !== cv.rows) {
      ctx.stopFrames = { cols: cv.cols, rows: cv.rows, frames: booking.storyFrames(s, { cols: cv.cols, rows: cv.rows, grids: true }) };
    }
    const frame = ctx.stopFrames.frames[Math.min(f, STOP_FRAMES - 1)].grid;
    cv.blit(frame, 0, 0);
    cv.reserve(0, 0, cv.cols - 1, cv.rows - 1);
    // clear the rows the chrome uses so the bars read cleanly
    for (const y of [L.top, L.bottom]) for (let x = 0; x < cv.cols; x++) { const c = cv.at(x, y); if (c) { c.ch = ' '; c.fg = null; } }
    chrome(cv, L, ctx, { station: 0, keys: f >= STOP_FRAMES - 1 ? KEYS_STATION : ['any key skips'] });
    return;
  }
  // compact: the same beats as words
  prepare(cv, L);
  pad(cv, L.ox, L.oy, L.ox + L.cw - 1, L.oy + L.ch - 1);
  backdrop(cv, THEMES.stop, 31, tick);
  chrome(cv, L, ctx, { station: 0, keys: f >= STOP_FRAMES - 1 ? KEYS_STATION : ['any key skips'] });
  let y = L.oy;
  cv.gradient(L.ox, y++, 'BASED ON A TRUE STORY', BRAND_A, BRAND_B, { bold: true });
  cv.put(L.ox, y++, fit(booking.placeAndTime(s), L.cw), { fg: SOFT });
  y++;
  if (f >= 6) cv.put(L.ox, y, `${s.source} tried`, { fg: SOFT });
  y++;
  if (f >= 8) cv.put(L.ox, y, fit('$ ' + typed(s.action, f, 8, 4, STOP_FRAMES - 1), L.cw), { fg: TEXT, bold: true });
  y += 2;
  if (f >= 22) {
    const on = f < 32 ? Math.floor(f / 2) % 2 === 0 : true;
    cv.put(L.ox, y, ' '.repeat(L.cw), { bg: on ? RED : mix(RED, THEMES.stop.bottom, 0.5) });
    const bar = 'OPERATOR · STOP';
    cv.put(L.ox + Math.floor((L.cw - bar.length) / 2), y, bar, { fg: WHITE, bold: true, bg: on ? RED : mix(RED, THEMES.stop.bottom, 0.5) });
  }
  y += 2;
  if (f >= 26) cv.put(L.ox, y++, fit(booking.stoppedLine(s), L.cw), { fg: RED, bold: true });
  if (f >= 28) cv.put(L.ox, y++, 'nothing was run', { fg: TEXT });
  const kept = booking.keptSafeLine(s);
  if (f >= 30 && kept) cv.put(L.ox, y++, fit(kept, L.cw), { fg: GREEN });
  if (f >= 40) cv.put(L.ox, y + 1, fit(CLOSING, L.cw), { fg: TEXT, bold: true });
}

// ── station 2: THE RECORD ──────────────────────────────────────────────────────────────────────
const RECORD_FRAMES = 44;
function blockAt(cv, x, y, w, record, { position, highlight, theme, head, mark = '✔', markColor = GREEN, label }) {
  const edge = highlight ? theme.accent : mix(theme.top, theme.accent, 0.55);
  const title = ` ${label || `receipt ${fmt(position)}`} `;
  const top = '┌' + title + '─'.repeat(Math.max(0, w - 2 - chars(title).length)) + '┐';
  cv.put(x, y, top, { fg: edge, bold: highlight });
  cv.put(x, y + 1, '│', { fg: edge });
  const digest = record ? String(record.digest || '').slice(0, 16) : '';
  cv.put(x + 2, y + 1, digest, { fg: highlight ? WHITE : TEXT, bold: highlight });
  // at most five letters, so the tag never touches the digest
  const TAGS = { STOP: 'STOP', PASS: 'pass', SHADOW_STOP: 'flag', SHADOW_TASTE: 'flag', ALLOW: 'allow', PERMIT_GRANTED: 'yes', GATE_ERROR: 'error', MODE_SET: 'mode' };
  const tag = head ? 'HEAD' : record ? TAGS[record.decision] || String(record.decision || '').toLowerCase().slice(0, 5) : '';
  cv.put(x + w - 2 - chars(tag).length, y + 1, tag, { fg: tag === 'STOP' ? RED : tag === 'HEAD' ? theme.accent : SOFT, bold: tag === 'STOP' || tag === 'HEAD' });
  cv.put(x + w - 1, y + 1, '│', { fg: edge });
  cv.put(x, y + 2, '└' + '─'.repeat(Math.floor((w - 3) / 2)) + '┬' + '─'.repeat(Math.ceil((w - 3) / 2)) + '┘', { fg: edge, bold: highlight });
  if (mark) cv.put(x + w + 1, y + 1, mark, { fg: markColor, bold: true });
}

function renderRecord(cv, L, ctx, f, tick) {
  const theme = THEMES.record;
  const records = ctx.log.records;
  const c = ctx.c;
  const chain = ctx.chainFinal || ctx.chainProgress || { checked: 0, total: records.length, rederived: 0, linked: 0, forks: [], breaks: [], ms: 0 };
  const leftW = L.compact ? 0 : 26;
  const textX = L.compact ? L.ox : L.ox + leftW + 5;
  const textW = L.compact ? L.cw : L.cw - leftW - 5;
  prepare(cv, L);
  pad(cv, L.ox, L.oy, L.ox + L.cw - 1, L.oy + L.ch - 1);
  backdrop(cv, theme, 41, tick, { density: 0.5, rain: hashRain(ctx.pool, theme, { strength: 0.5, every: 3 }) });
  const forging = ctx.forge && ctx.forge.active;
  chrome(cv, L, ctx, { station: 1, keys: f >= RECORD_FRAMES - 1 ? (forging ? ['f or esc back to the record', 'enter next', 'q leave'] : ['enter next', 'f forge a receipt', '← back', 'm map', 'q leave']) : ['any key skips'] });
  cv.put(L.ox, L.oy, 'THE RECORD', { fg: theme.accent, bold: true });
  cv.put(L.ox, L.oy + 1, fit(STATIONS[1].line, L.cw), { fg: SOFT });

  // the walk: from this case back to the first receipt, then home again
  const casePos = c.index;
  let pos = casePos;
  if (f < 30) pos = Math.round(casePos - (casePos) * ease(f / 29));
  else if (f < 40) pos = Math.round(casePos * ease((f - 30) / 9));
  pos = Math.max(0, Math.min(records.length - 1, pos));
  const settled = f >= 40;

  if (!L.compact) {
    const bx = L.ox;
    const by = L.oy + 3;
    const span = Math.max(3, Math.min(4, Math.floor((L.ch - 4) / 4)));
    const first = Math.max(0, Math.min(records.length - span, pos - 1));
    for (let k = 0; k < span; k++) {
      const i = first + k;
      const record = records[i];
      if (!record) continue;
      const y = by + k * 4;
      const isCase = i === casePos && settled;
      const ok = Boolean(precedent.rederive(record));
      const forgedHere = forging && i === casePos;
      let mark = ok ? '✔' : '✗';
      let markColor = ok ? GREEN : RED;
      if (forgedHere && ctx.forge.f >= 8 && ctx.forge.f < 22) { mark = '✗'; markColor = RED; }
      blockAt(cv, bx, y, leftW, record, { position: i + 1, highlight: isCase, theme, head: i === records.length - 1, mark, markColor, label: i === 0 ? 'receipt 1 · genesis' : undefined });
      if (forgedHere && ctx.forge.f >= 2) {
        const edited = ctx.forge.f >= 22 ? ctx.forge.result.step2.newDigest.slice(0, 16) : `decision ${ctx.forge.f >= 5 ? 'PASS' : 'STOP'}`;
        cv.put(bx + 2, y + 1, fit(edited.padEnd(leftW - 4), leftW - 4), { fg: AMBER, bold: true });
        if (ctx.forge.f >= 22) cv.put(bx + leftW - 6, y + 1, 'PASS', { fg: AMBER, bold: true });
      }
      if (k < span - 1) {
        const next = records[i + 1];
        const linked = next && next.prev === record.digest;
        const broken = forging && i === casePos && ctx.forge.f >= 22;
        const ch = broken ? '✗' : '│';
        cv.put(bx + Math.floor((leftW - 3) / 2) + 1, y + 3, ch, { fg: broken ? RED : linked ? mix(theme.accent, WHITE, 0.2) : AMBER, bold: broken });
        cv.put(bx + Math.floor((leftW - 3) / 2) + 3, y + 3, broken ? 'link broken' : linked ? 'prev' : 'fork', { fg: broken ? RED : FAINT, bold: broken });
      }
    }
  }

  let y = L.oy + 3;
  const LABEL = 14;
  const line = (label, value, style = {}, note = '') => {
    if (y >= L.oy + L.ch) return;
    cv.put(textX, y, fit(label, LABEL), { fg: SOFT });
    const end = cv.put(textX + LABEL, y, fit(value, textW - LABEL), { fg: TEXT, bold: true, ...style });
    if (note && end + 2 < textX + textW) cv.put(end + 2, y, fit(note, textX + textW - end - 2), { fg: FAINT });
    y++;
  };
  if (forging) { renderForge(cv, L, ctx, textX, textW, y); return; }
  const shown = (n) => fmt(Math.round(n * clamp01(f / 26)));
  const done = f >= 26;
  line('receipts', shown(chain.total));
  line('re-derived', shown(chain.rederived), { fg: chain.rederived === chain.total ? GREEN : AMBER }, chain.rederived === chain.total && done ? '✔ every one' : '');
  line('linked', shown(chain.linked));
  const forks = chain.forkCount !== undefined ? chain.forkCount : chain.forks.length;
  line('forks', done ? fmt(forks) : '', { fg: forks ? AMBER : TEXT }, forks && chain.forks[0] && done ? `${monthDay(Date.parse(chain.forks[0].ts))}: two writes at once` : '');
  line('broken', done ? fmt(chain.breaks.length) : '', { fg: chain.breaks.length ? RED : GREEN }, done && !chain.breaks.length ? 'none' : '');
  y++;
  if (f >= 30) {
    const firstTs = Date.parse(records[0] && records[0].ts);
    line('first', Number.isNaN(firstTs) ? '?' : monthDay(firstTs), {}, chain.genesis ? 'its prev is all zeros' : '');
    line('head', chain.headMatches ? 'matches the newest' : 'no HEAD file', { fg: chain.headMatches ? GREEN : AMBER, bold: false });
    line('checked in', `${(chain.ms / 1000).toFixed(2)} s`, { bold: false }, 'here, just now');
  }
  y++;
  if (f >= 36) {
    line('this case', `receipt ${fmt(c.index + 1)}`, { fg: theme.accent });
    if (y < L.oy + L.ch) cv.put(textX + LABEL, y++, fit(`${c.file}, line ${fmt(c.line)}`, textW - LABEL), { fg: FAINT });
  }
  if (settled && y + 1 < L.oy + L.ch) {
    y++;
    const end = cv.put(textX, y, 'f', { fg: WHITE, bold: true });
    cv.put(end + 1, y, fit('forge it (on a copy in memory)', textX + textW - end - 1), { fg: theme.accent });
  }
}

const FORGE_FRAMES = 52;
function renderForge(cv, L, ctx, x, w, y0) {
  const theme = THEMES.record;
  const fr = ctx.forge.f;
  const r = ctx.forge.result;
  let y = y0;
  const put = (text, style, at = 0) => { if (fr >= at && y < L.oy + L.ch) cv.put(x, y, fit(text, w), style); y++; };
  put('FORGE THIS RECEIPT', { fg: AMBER, bold: true });
  put('turn the STOP into a PASS', { fg: SOFT });
  y++;
  put(`1  edit it: ${r.step1.from} → ${r.step1.to}`, { fg: TEXT, bold: true }, 2);
  put(r.step1.digestStillMatches ? '   the digest still fits?!' : '   ✗ its digest no longer fits its bytes', { fg: r.step1.digestStillMatches ? AMBER : RED, bold: true }, 9);
  y++;
  put(`2  re-seal it: ${r.step2.newDigest.slice(0, 16)}`, { fg: TEXT, bold: true }, 16);
  put(r.step2.nextIndex === null ? '   it was the newest receipt, but…' : `   ✗ receipt ${fmt(r.step2.nextIndex + 1)} names the old digest`, { fg: r.step2.nextIndex === null ? AMBER : RED, bold: true }, 23);
  y++;
  put(`3  rewrite the ${fmt(r.step3.laterRecords)} receipts after it`, { fg: TEXT, bold: true }, 30);
  const witness = ctx.memory && ctx.memory.reachable && ctx.memory.stop && ctx.memory.stop.digestAgrees;
  put(witness ? `   ✗ Vestige memory still says ${ctx.c.digest.slice(0, 8)}` : '   ✗ any earlier copy of HEAD disagrees', { fg: RED, bold: true }, 37);
  y++;
  put('nothing on disk changed: this was a copy', { fg: theme.accent }, 44);
}

// ── station 3: THE MEMORY ──────────────────────────────────────────────────────────────────────
const MEMORY_FRAMES = 40;
// Half-cell precision, so 0.95 and 1.00 look different.
function bar(value, cells = 10) {
  const halves = Math.max(0, Math.min(cells * 2, Math.round((Number(value) || 0) * cells * 2)));
  const full = Math.floor(halves / 2);
  return '█'.repeat(full) + (halves % 2 ? '▌' : '') + '░'.repeat(cells - full - (halves % 2));
}
function renderMemory(cv, L, ctx, f, tick) {
  const theme = THEMES.memory;
  const c = ctx.c;
  const m = ctx.memory;
  // the picture shows only what is true: a link when both memories exist, one dim cell when the stop has none
  const state = !m ? 'pending' : !m.reachable ? 'down' : !m.stop ? 'absent' : 'present';
  const art = !L.compact;
  const artW = 24;
  const textX = art ? L.ox + artW + 4 : L.ox;
  const textW = art ? L.cw - artW - 4 : L.cw;
  prepare(cv, L);
  pad(cv, L.ox, L.oy, L.ox + L.cw - 1, L.oy + 2);
  if (art) pad(cv, L.ox, L.oy + 3, L.ox + artW, L.oy + 16, 1);
  pad(cv, textX, L.oy + 3, L.ox + L.cw - 1, L.oy + L.ch - 1);
  pad(cv, L.ox, L.oy + 16, L.ox + L.cw - 1, L.oy + L.ch - 1);
  backdrop(cv, theme, 51, tick, { density: 0.7 });
  chrome(cv, L, ctx, { station: 2, keys: f >= MEMORY_FRAMES - 1 ? KEYS_STATION : ['any key skips'] });
  cv.put(L.ox, L.oy, 'THE MEMORY', { fg: theme.accent, bold: true });
  cv.put(L.ox, L.oy + 1, fit(STATIONS[2].line, L.cw), { fg: SOFT });

  const somaA = L.oy + 4;
  const somaB = L.oy + 12;
  if (art) {
    const cx = L.ox + 11;
    const neuron = (y, lit, glow, dim = false) => {
      const col = dim ? mix(theme.top, SOFT, 0.35) : lit ? mix(theme.accent, WHITE, glow) : mix(theme.top, theme.accent, 0.5);
      cv.put(cx - 3, y - 1, '\\  |  /', { fg: col });
      cv.put(cx - 6, y, '───(   )───', { fg: col });
      cv.put(cx - 1, y, dim ? '○' : '●', { fg: dim ? col : lit ? WHITE : col, bold: !dim });
      cv.put(cx - 3, y + 1, '/  |  \\', { fg: col });
    };
    if (state === 'present') {
      neuron(somaA, f >= 6, 0.3 + 0.3 * Math.sin(tick * 0.5));
      if (m.rule) {
        for (let y = somaA + 2; y <= somaB - 2; y++) {
          const travel = f >= 10 ? ((f - 10) * 0.45 + tick * 0.3) % 8 : -1;
          const hot = f >= 10 && Math.abs((y - somaA - 2) - travel) < 0.8;
          cv.put(cx, y, hot ? '•' : '│', { fg: hot ? WHITE : mix(theme.top, theme.accent, f >= 10 ? 0.75 : 0.4), bold: hot });
        }
        if (f >= 12) cv.put(cx + 2, somaA + 5, 'derived_from', { fg: FAINT });
        neuron(somaB, f >= 16, 0.15);

      }
    } else if (state !== 'pending') {
      neuron(somaA, false, 0, true);
    }
  }

  let y = L.oy + 3;
  const put = (text, style = {}, at = 0) => { if (f >= at && y < L.oy + L.ch) cv.put(textX, y, fit(text, textW), style); y++; };
  const ask = () => {
    // the call to copy always gets the whole width, on its own line
    y = Math.max(y + 1, art ? L.oy + 17 : y);
    if (y < L.oy + L.ch) cv.put(L.ox, y++, 'ask your agent', { fg: SOFT });
    if (y < L.oy + L.ch) cv.put(L.ox, y++, fit(c.recall, L.cw), { fg: theme.accent, bold: true });
  };
  if (state === 'pending') { put('asking Vestige…', { fg: SOFT }); return; }
  if (state === 'down') {
    y = art ? somaA - 1 : y;
    put('Vestige memory did not answer', { fg: AMBER, bold: true });
    put(`on this machine (${m.where || '127.0.0.1:3927'})`, { fg: FAINT });
    y++;
    put('memories fade unless they are recalled.', { fg: SOFT });
    put('the record on the chain never fades.', { fg: SOFT });
    ask();
    return;
  }
  if (state === 'absent') {
    y = art ? somaA - 1 : y;
    put('this stop is not in memory', { fg: AMBER, bold: true });
    for (const w of wrap('the stop reflex records Claude Code and ZCode stops, from the day it was installed', textW)) put(w, { fg: SOFT });
    y++;
    if (m.total !== null) put(`Vestige holds ${fmt(m.total)} memories here`, { fg: TEXT });
    put('the record on the chain still holds this stop', { fg: SOFT });
    ask();
    return;
  }
  y = art ? somaA - 1 : y;
  put('this stop', { fg: SOFT }, 4);
  put(`${m.stop.id} · ${m.stop.type}`, { fg: WHITE, bold: true }, 6);
  put(m.stop.retention === null ? 'retention unknown' : `retention ${bar(m.stop.retention)} ${m.stop.retention.toFixed(2)}`, { fg: theme.accent }, 8);
  const lag = m.stop.createdAt ? Math.max(0, Math.round((m.stop.createdAt - c.story.ts) / 1000)) : null;
  put(lag !== null ? `written ${lag < 120 ? lag + ' s' : lag < 7200 ? Math.round(lag / 60) + ' min' : Math.round(lag / 3600) + ' h'} after the stop` + (m.stop.digestAgrees ? ', quoting its receipt' : '') : '', { fg: FAINT }, 10);
  y = art ? somaB - 1 : y + 1;
  if (m.rule) {
    put('its rule', { fg: SOFT }, 16);
    put(`${m.rule.id} · ${c.law.key}`, { fg: WHITE, bold: true }, 17);
    const fading = m.rule.retention !== null && m.rule.retention < 0.9;
    put(m.rule.retention === null ? 'retention unknown' : `retention ${bar(m.rule.retention)} ${m.rule.retention.toFixed(2)}${fading ? '  fading' : ''}`, { fg: fading ? mix(theme.accent, SOFT, 0.5) : theme.accent }, 18);
    if (!L.compact) put(m.rule.createdAt ? `first written ${monthDay(m.rule.createdAt)}` : '', { fg: FAINT }, 19);
  } else {
    put('its rule has no memory of its own yet', { fg: SOFT }, 16);
  }
  // below the picture the words get the whole width
  y = Math.max(y + 1, art ? somaB + 4 : y);
  const full = (text, style, at) => { if (f >= at && y < L.oy + L.ch) cv.put(L.ox, y, fit(text, L.cw), style); y++; };
  if (m.rememberedUnderRule && !L.compact) full(`${fmt(m.rememberedUnderRule)} memories carry the tag ${c.law.key}`, { fg: TEXT }, 22);
  full(L.compact ? 'memory fades unless recalled. the record never does.' : 'memory fades unless it is recalled. the record never fades.', { fg: SOFT }, 26);
  if (f >= 30) {
    if (L.compact) { if (y < L.oy + L.ch) cv.put(L.ox, y++, 'ask your agent', { fg: SOFT }); if (y < L.oy + L.ch) cv.put(L.ox, y++, fit(c.recall, L.cw), { fg: theme.accent, bold: true }); }
    else if (y < L.oy + L.ch) { const end = cv.put(L.ox, y, 'ask your agent  ', { fg: SOFT }); cv.put(end, y, fit(c.recall, L.ox + L.cw - end), { fg: theme.accent, bold: true }); }
  }
}

// ── station 4: THE PRECEDENT ───────────────────────────────────────────────────────────────────
const PRECEDENT_FRAMES = 42;
function renderPrecedent(cv, L, ctx, f, tick) {
  const theme = THEMES.precedent;
  const c = ctx.c;
  const law = c.law;
  const chartW = L.compact ? 0 : 34;
  const textX = L.compact ? L.ox : L.ox + chartW + 4;
  const textW = L.compact ? L.cw : L.cw - chartW - 4;
  prepare(cv, L);
  pad(cv, L.ox, L.oy, L.ox + L.cw - 1, L.oy + L.ch - 1);
  backdrop(cv, theme, 61, tick, { density: 0.6 });
  chrome(cv, L, ctx, { station: 3, keys: f >= PRECEDENT_FRAMES - 1 ? KEYS_STATION : ['any key skips'] });
  cv.put(L.ox, L.oy, 'THE PRECEDENT', { fg: theme.accent, bold: true });
  cv.put(L.ox, L.oy + 1, fit(STATIONS[3].line, L.cw), { fg: SOFT });
  const kind = law.rule.mode === 'LAW' ? 'owner law' : `${law.rule.mode === 'SHADOW' ? 'shadow rule' : 'rule'}`;
  cv.put(L.ox, L.oy + 3, fit(`${law.key}${law.rule.name && law.rule.name !== law.key ? ' · ' + law.rule.name : ''} · ${kind}`, L.cw), { fg: WHITE, bold: true });
  // the rule in its own words: two lines at most
  const ruleLines = law.rule.text ? Math.min(2, wrap(`"${law.rule.text}"`, L.cw).length) : 1;
  const dy = !L.compact && ruleLines > 1 ? 1 : 0;
  if (law.rule.text && f >= 4) {
    const words = wrap(`"${law.rule.text}"`, L.cw);
    const shown = words.length > 2 ? [words[0], fit(words.slice(1).join(' '), L.cw)] : L.compact ? [fit(words.join(' '), L.cw)] : words;
    shown.forEach((w, i) => cv.put(L.ox, L.oy + 4 + i, w, { fg: TEXT }));
  }

  if (!L.compact) {
    // the docket: cases per day, this case's day lit
    const caseDay = new Date(c.story.ts).toISOString().slice(0, 10);
    const counts = new Map(law.days);
    const days = Array.from({ length: 10 }, (_v, k) => { const d = new Date(Date.parse(caseDay + 'T00:00:00Z') - (9 - k) * 86400000).toISOString().slice(0, 10); return [d, counts.get(d) || 0]; });
    const max = Math.max(1, ...days.map((d) => d[1]));
    const height = 4 - dy;
    const x0 = L.ox + 5;
    const yBase = L.oy + 11;
    cv.put(L.ox, L.oy + 6 + dy, 'cases per day', { fg: SOFT });
    cv.put(L.ox, yBase - height, String(max).padStart(3), { fg: SOFT });
    cv.put(L.ox + 3, yBase - height, ' ┤', { fg: FAINT });
    for (let yy = yBase - height + 1; yy < yBase; yy++) cv.put(L.ox + 4, yy, '│', { fg: FAINT });
    cv.put(L.ox + 2, yBase, '0 └' + '─'.repeat(days.length * 3), { fg: FAINT });
    days.forEach(([day, n], i) => {
      const grow = clamp01((f - 4 - i * 1.5) / 8);
      const hgt = Math.max(1, Math.round((n / max) * height * grow));
      const isCase = day === caseDay;
      const col = isCase ? mix(theme.accent, WHITE, 0.25 + 0.25 * Math.sin(tick * 0.6)) : mix(theme.top, theme.accent, 0.6);
      for (let k = 0; k < (n ? hgt : 0) && grow > 0; k++) cv.put(x0 + i * 3, yBase - 1 - k, '██', { fg: col, bold: isCase });
      cv.put(x0 + i * 3, yBase + 1, day.slice(8, 10), { fg: isCase ? WHITE : n ? SOFT : FAINT, bold: isCase });
    });
    if (days.length) cv.put(x0 + days.length * 3, yBase + 1, MONTHS[Number(days[days.length - 1][0].slice(5, 7)) - 1], { fg: FAINT });
  }

  let y = L.oy + 6 + dy;
  const fact = (label, value, at, style = {}) => {
    if (y >= L.oy + 13) return;
    if (f >= at) {
      cv.put(textX, y, fit(label, 14), { fg: SOFT });
      cv.put(textX + 14, y, fit(value, textW - 14), { fg: TEXT, bold: true, ...style });
    }
    y++;
  };
  fact('this case', `${fmt(law.position)} of ${fmt(law.count)} under this rule`, 8, { fg: theme.accent });
  fact('the first', law.first.isThis ? 'this one: the landmark case' : `${monthDay(law.first.ts)} · ${law.first.source}`, 12);
  // as many agents as fit, then how many more
  const agentWords = (() => { const room = textW - 14; let out = ''; let shownN = 0; for (const [src, n] of law.sources) { const next = (out ? out + ' · ' : '') + `${src} ${fmt(n)}`; const more = law.sources.length - shownN - 1; if (chars(next).length + (more ? 5 : 0) > room) break; out = next; shownN++; } const rest = law.sources.length - shownN; return rest ? `${out} · +${rest}` : out; })();
  fact('agents', agentWords, 16);
  fact('same action', law.sameAction > 1 ? `stopped ${fmt(law.sameAction)} times` : 'stopped once: this time', 20);
  const p = c.permit;
  fact('permit', p ? `${p.status} · granted ${p.granted ? monthDay(p.granted * 1000) : '?'} by ${p.by || 'the owner'}` : 'none for this action', 24, { fg: p ? AMBER : TEXT });
  if (c.permitsUsedForRule && !L.compact) fact('yeses used', `${fmt(c.permitsUsedForRule)} permit${c.permitsUsedForRule === 1 ? '' : 's'}, once each`, 26, { fg: SOFT, bold: false });

  // the doctrine and the call, anchored to the bottom of the content
  const tail = [
    ['history, not permission.', { fg: theme.accent, bold: true }, 28],
    ...(L.compact ? [] : [['every permit is single-use: an old yes never covers a new attempt.', { fg: SOFT }, 30], ['', {}, 0]]),
    ['ask your agent', { fg: SOFT }, 33],
    [typed(c.precedent.call, f, 34, 8, PRECEDENT_FRAMES - 1), { fg: WHITE, bold: true }, 34],
    [`→ "${c.precedent.answer}"`, { fg: theme.accent }, 38],
  ];
  // a call to copy never gets cut: long ones wrap at a space
  const lines = [];
  for (const [text, style, at] of tail) { if (chars(text).length > L.cw) for (const w of wrap(text, L.cw)) lines.push([w, style, at]); else lines.push([text, style, at]); }
  y = Math.max(y + 1, L.oy + L.ch - lines.length);
  for (const [text, style, at] of lines) { if (text && f >= at && y < L.oy + L.ch) cv.put(L.ox, y, fit(text, L.cw), style); y++; }
}

// ── station 5: THE AFTERMATH ───────────────────────────────────────────────────────────────────
const AFTERMATH_FRAMES = 44;
const SHORT_TOOL = { bash: 'Bash', read: 'Read', edit: 'Edit', multiedit: 'Edit', write: 'Write', grep: 'Grep', glob: 'Glob', task: 'Agent', agent: 'Agent', webfetch: 'Web', websearch: 'Web', toolsearch: 'Tools', todowrite: 'Todo', exec: 'exec', notebookedit: 'Notes', skill: 'Skill', workflow: 'Flow' };
const toolShort = (tool) => { const t = String(tool || ''); if (/^mcp:/.test(t)) return 'MCP'; return SHORT_TOOL[t.toLowerCase()] || fit(t, 5).replace('…', ''); };
function renderAftermath(cv, L, ctx, f, tick) {
  const theme = THEMES.aftermath;
  const a = ctx.c.aftermath;
  prepare(cv, L);
  pad(cv, L.ox, L.oy, L.ox + L.cw - 1, L.oy + L.ch - 1);
  backdrop(cv, theme, 71, tick, { density: 0.6 });
  chrome(cv, L, ctx, { station: 4, keys: f >= AFTERMATH_FRAMES - 1 ? KEYS_STATION : ['any key skips'] });
  cv.put(L.ox, L.oy, 'THE AFTERMATH', { fg: theme.accent, bold: true });
  cv.put(L.ox, L.oy + 1, fit(STATIONS[4].line, L.cw), { fg: SOFT });
  let y = L.oy + 3;
  const s = ctx.c.story;
  cv.put(L.ox, y, '✗ STOP', { fg: RED, bold: true });
  cv.put(L.ox + 8, y, fit(`${s.commitment} · ${clock(s.ts)}`, L.cw - 8), { fg: TEXT });
  y++;
  if (!a.session8) { cv.put(L.ox, y + 1, 'this receipt carries no session id, so its aftermath cannot be followed', { fg: SOFT }); return; }
  cv.put(L.ox, y, fit(`then, in the same session (${s.source} · ${a.session8})`, L.cw), { fg: SOFT });
  y += 2;
  const tileW = 8; // a seven-cell tile and a one-cell gap
  const perRow = Math.max(4, Math.floor(L.cw / tileW));
  const rowsOfTiles = L.compact ? 1 : 2;
  const shown = a.next.slice(0, perRow * rowsOfTiles);
  shown.forEach((call, i) => {
    const appear = 4 + i * 0.9;
    if (f < appear) return;
    const tx = L.ox + (i % perRow) * tileW;
    const ty = y + Math.floor(i / perRow) * 4;
    const stop = call.decision === 'STOP';
    const flagged = call.decision === 'SHADOW_STOP' || call.decision === 'SHADOW_TASTE';
    const col = stop ? RED : flagged ? AMBER : call.same ? AMBER : mix(theme.top, theme.accent, 0.75);
    const mark = stop ? '✗' : flagged ? '!' : '✔';
    cv.put(tx, ty, '╭─────╮', { fg: col });
    cv.put(tx, ty + 1, '│', { fg: col });
    cv.put(tx + 1, ty + 1, toolShort(call.tool).padEnd(5).slice(0, 5), { fg: TEXT, bold: stop });
    cv.put(tx + 6, ty + 1, '│', { fg: col });
    cv.put(tx, ty + 2, '╰──', { fg: col });
    cv.put(tx + 3, ty + 2, mark, { fg: stop ? RED : flagged ? AMBER : GREEN, bold: true });
    cv.put(tx + 4, ty + 2, '──╯', { fg: col });
  });
  y += rowsOfTiles * 4;
  const more = a.calls - shown.length;
  if (f >= 30) {
    const verdict = a.retried
      ? `it tried the same action ${fmt(a.retried)} more time${a.retried === 1 ? '' : 's'}${a.retriedStopped === a.retried ? ', and was stopped every time' : ''}`
      : a.calls ? 'it never tried that action again' : 'the session ended there';
    const count = `${fmt(a.calls)} more call${a.calls === 1 ? '' : 's'}${more > 0 ? ` (${fmt(more)} not shown)` : ''}`;
    const style = { fg: a.retried ? AMBER : GREEN, bold: true };
    if (chars(`${count} · ${verdict}`).length <= L.cw) cv.put(L.ox, y, `${count} · ${verdict}`, style);
    else { cv.put(L.ox, y, fit(count, L.cw), { fg: TEXT }); y++; cv.put(L.ox, y, fit(verdict, L.cw), style); }
  }
  y += 2;
  if (f >= 36 && y < L.oy + L.ch) {
    const later = a.laterCount
      ? `the same action came back in ${fmt(a.laterSessions)} later session${a.laterSessions === 1 ? '' : 's'}: ` + a.later.slice(-3).map((l) => `${monthDay(l.ts)} ${l.source} ${l.decision === 'STOP' ? 'stopped' : l.decision.toLowerCase()}`).join(' · ')
      : 'no later session has tried this exact action';
    cv.put(L.ox, y, fit(later, L.cw), { fg: TEXT });
  }
  y++;
  if (f >= 40 && y < L.oy + L.ch) cv.put(L.ox, y, "a new session forgets. precedent doesn't.", { fg: theme.accent, bold: true });
}

// ── station 6: THE VERDICT ─────────────────────────────────────────────────────────────────────
const VERDICT_FRAMES = 24;
function renderVerdict(cv, L, ctx, f, tick) {
  const theme = THEMES.verdict;
  const s = ctx.c.story;
  prepare(cv, L);
  pad(cv, L.ox, L.oy, L.ox + L.cw - 1, L.oy + L.ch - 1);
  backdrop(cv, theme, 81, tick, { density: 0.8 });
  chrome(cv, L, ctx, { station: 5, keys: f >= VERDICT_FRAMES - 1 ? ['enter close the loop', 's text', 'i image', 'g gif', 'c code', '← back', 'q leave'] : ['any key skips'] });
  const cardW = booking.STAGE_COLS;
  if (!L.compact && L.cw >= cardW) {
    const grid = booking.bookingGrid(s, { cols: cardW }); // the replay command is printed whole below it
    const x = L.ox + Math.floor((L.cw - cardW) / 2);
    const reveal = clamp01(f / 14);
    const rows = Math.round(grid.rows * ease(reveal));
    const top = L.oy;
    for (let r = 0; r < rows; r++) for (let col = 0; col < grid.cols; col++) {
      const src = grid.cells[r * grid.cols + col];
      const dst = cv.at(x + col, top + r);
      if (!dst) continue;
      dst.ch = stage.safeChar(src.ch);
      // the card's own words have no colour of their own; give them the station's light ink
      dst.fg = src.fg || (src.dim ? SOFT : TEXT);
      dst.bold = src.bold;
      dst.dim = false;
      dst.bg = mix(theme.bottom, [0, 0, 0], 0.35);
    }
    if (f >= 16) {
      // the replay command, whole: whoever runs it sees this stop in their own terminal
      const cmd = `${booking.INSTALL} play ${ctx.code}`;
      // rows to the key bar, so a taller window shows the whole command
      const room = L.bottom - 2 - (top + grid.rows + 1) + 1;
      const lines = [];
      for (let k = 0; k < chars(cmd).length; k += cardW) lines.push(chars(cmd).slice(k, k + cardW).join(''));
      let y = top + grid.rows + 1;
      if (lines.length + 1 <= room) { cv.put(x, y++, 'replay it anywhere', { fg: SOFT }); }
      if (lines.length <= Math.max(room, 0)) lines.forEach((line) => cv.put(x, y++, line, { fg: GREEN }));
      else cv.put(x, y, fit('c copies the replay command', cardW), { fg: SOFT });
    }
    return;
  }
  const out = [];
  booking.compactCard(s, {}).map((line, i) => (i === 0 ? line.replace('REM got booked', 'vestige · booked') : line)).forEach((line, i) => {
    if (line === booking.INSTALL) return;
    wrap(line, L.cw).forEach((w) => out.push([w, i === 0 ? { fg: theme.accent, bold: true } : i === 2 ? { fg: RED, bold: true } : { fg: TEXT }]));
  });
  out.push(['', {}]);
  out.push(['replay it anywhere:', { fg: SOFT }]);
  const cmd = `${booking.INSTALL} play ${ctx.code}`;
  for (let k = 0; k < chars(cmd).length; k += L.cw) out.push([chars(cmd).slice(k, k + L.cw).join(''), { fg: GREEN }]);
  out.push(['', {}]);
  out.forEach(([line, style], i) => { if (f >= i && L.oy + i < L.oy + L.ch) cv.put(L.ox, L.oy + i, fit(line, L.cw), style); });
}

const RENDER = [renderStop, renderRecord, renderMemory, renderPrecedent, renderAftermath, renderVerdict];
const FRAMES = [STOP_FRAMES, RECORD_FRAMES, MEMORY_FRAMES, PRECEDENT_FRAMES, AFTERMATH_FRAMES, VERDICT_FRAMES];

function renderTooSmall(cv, ctx) {
  cv.reserve(0, Math.floor(cv.rows / 2) - 3, cv.cols - 1, Math.floor(cv.rows / 2) + 2);
  backdrop(cv, THEMES.map, 3, 0, { density: 0.2 });
  const lines = [['the window is a little small', true], [`${cv.cols}×${cv.rows} now · needs 60×20`, false], ['make it bigger and the journey comes back', false], ['q leaves', false]]
    .flatMap(([text, strong]) => wrap(text, Math.max(8, cv.cols - 2)).map((w) => [w, strong]));
  const top = Math.max(0, Math.floor((cv.rows - lines.length) / 2));
  lines.forEach(([text, strong], i) => cv.put(Math.max(0, Math.floor((cv.cols - chars(text).length) / 2)), top + i, fit(text, cv.cols), { fg: strong ? WHITE : SOFT, bold: strong }));
}

function renderHelp(cv, L) {
  if (cv.cols < 30 || cv.rows < 18) return; // no room for a panel; the too-small screen says so
  const lines = [
    'KEYS',
    '',
    'enter / space / →   next (or skip the animation)',
    '← / backspace       back one station',
    'l / h               next / back (vi keys)',
    '1 - 6               jump to a station',
    'm                   the map',
    'f                   forge a receipt (at THE RECORD)',
    's i g c             share (at THE VERDICT)',
    'r                   play this station again',
    'n                   another case (title, finale)',
    'q / esc             leave; progress is kept',
    'ctrl-c              quit at once',
    '',
    'any key closes this',
  ];
  const w = Math.min(54, cv.cols - 8);
  const x = Math.floor((cv.cols - w) / 2);
  const y = Math.max(1, Math.floor((cv.rows - lines.length - 2) / 2));
  const panel = [30, 32, 64];
  for (let yy = y - 1; yy <= y + lines.length; yy++) for (let xx = x - 3; xx <= x + w + 2; xx++) { const c = cv.at(xx, yy); if (c) Object.assign(c, { ch: ' ', fg: null, bg: panel, bold: false, dim: false, rev: false }); }
  const edge = { fg: BRAND_B, bg: panel };
  cv.put(x - 3, y - 1, '╭' + '─'.repeat(w + 4) + '╮', edge);
  cv.put(x - 3, y + lines.length, '╰' + '─'.repeat(w + 4) + '╯', edge);
  for (let yy = y; yy < y + lines.length; yy++) { cv.put(x - 3, yy, '│', edge); cv.put(x + w + 2, yy, '│', edge); }
  lines.forEach((text, i) => {
    const line = fit(text, w).padEnd(w);
    const keyPart = i > 1 && text.length > 20 ? line.slice(0, 20) : '';
    if (keyPart) { cv.put(x, y + i, keyPart, { fg: WHITE, bold: true, bg: panel }); cv.put(x + 20, y + i, line.slice(20), { fg: TEXT, bg: panel }); }
    else cv.put(x, y + i, line, { fg: i === 0 ? BRAND_B : i === lines.length - 1 ? SOFT : TEXT, bold: i === 0, bg: panel });
  });
}

// ── progress, kept between visits ──────────────────────────────────────────────────────────────
const journeyPath = (env = process.env) => path.join(path.dirname(story.statePath(env)), 'journey.json');
// Only a file of the right shape is used; entries that are not { stations: [...] } are dropped.
function loadJourney(env = process.env) {
  const fresh = { v: 1, cases: {}, lastVisit: null, lastTotal: null, lastStops: null };
  try {
    const j = JSON.parse(fs.readFileSync(journeyPath(env), 'utf8'));
    if (!j || typeof j !== 'object' || Array.isArray(j) || !j.cases || typeof j.cases !== 'object' || Array.isArray(j.cases)) return fresh;
    const cases = {};
    for (const [d, e] of Object.entries(j.cases)) {
      if (!/^[a-f0-9]{16}$/.test(d) || !e || typeof e !== 'object' || !Array.isArray(e.stations)) continue;
      cases[d] = { stations: e.stations.filter((n) => Number.isInteger(n) && n >= 0 && n < 6), finished: e.finished === true, first: Number(e.first) || null, at: Number(e.at) || null };
    }
    const num = (v) => (Number.isFinite(Number(v)) && v !== null ? Number(v) : null);
    return { v: 1, cases, lastVisit: num(j.lastVisit), lastTotal: num(j.lastTotal), lastStops: num(j.lastStops) };
  } catch { return fresh; }
}
function saveJourney(j, env = process.env) {
  const file = journeyPath(env);
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const draft = `${file}.${process.pid}.draft`;
    fs.writeFileSync(draft, JSON.stringify(j));
    fs.renameSync(draft, file);
    return true;
  } catch { return false; }
}

// ── building the context ───────────────────────────────────────────────────────────────────────
function pickCase(cases, journey, token) {
  if (token) {
    if (/^\d{1,6}$/.test(token)) return cases.find((k) => k.number === Number(token)) || null;
    const hits = cases.filter((k) => k.story.digest.startsWith(String(token).toLowerCase()));
    return hits.length === 1 ? hits[0] : null;
  }
  return cases.find((k) => !(journey.cases[k.story.digest] && journey.cases[k.story.digest].finished)) || cases[0] || null;
}

function buildContext({ env = process.env, token = null } = {}) {
  const home = precedent.operatorHome(env);
  const log = precedent.readLog(home);
  const journey = loadJourney(env);
  const cases = precedent.eligibleCases(log).map((k) => ({ ...k, number: 0 }));
  // case numbers count every STOP on this machine, oldest first
  let n = 0;
  const numberAt = new Map();
  log.records.forEach((r, i) => { if (r.decision === 'STOP') numberAt.set(i, ++n); });
  cases.forEach((k) => { k.number = numberAt.get(k.index) || 0; });
  const pick = pickCase(cases, journey, token);
  const pool = log.records.slice(-48).map((r) => String(r.digest || '')).filter((d) => /^[a-f0-9]{64}$/.test(d));
  const ctx = {
    env, home, log, journey, cases, pool: pool.length ? pool : ['0123456789abcdef'],
    c: null, visited: new Set(), chainProgress: null, chainFinal: null, memory: null, toast: null, code: '', forge: null,
    since: '', empty: log.records.length ? 'Operator has not stopped anything on this machine yet' : 'no Operator receipts on this machine yet',
  };
  if (journey.lastVisit && journey.lastTotal !== null) {
    const newReceipts = Math.max(0, log.records.length - journey.lastTotal);
    const newStops = Math.max(0, n - (journey.lastStops || 0));
    if (newReceipts) ctx.since = `since your last visit: ${fmt(newReceipts)} receipts · ${fmt(newStops)} new stop${newStops === 1 ? '' : 's'}`;
  }
  ctx.stopCount = n;
  if (pick) setCase(ctx, pick);
  return ctx;
}

function setCase(ctx, pick) {
  const catalog = ctx.catalog || new Map();
  ctx.c = precedent.buildCase(ctx.log, pick.index, { catalog, permits: ctx.permits || precedent.loadPermits(ctx.home) });
  ctx.c.number = pick.number;
  ctx.code = (() => { try { return replay.encode(ctx.c.story); } catch { return ''; } })();
  const saved = ctx.journey.cases[ctx.c.story.digest];
  ctx.visited = new Set(saved && Array.isArray(saved.stations) ? saved.stations : []);
  ctx.memory = null;
  ctx.stopFrames = null;
  ctx.forge = null;
}

function startLoads(ctx) {
  // The chain re-check and the rule list run while the title plays.
  ctx.chainPromise = precedent.verifyChain(ctx.log, { onProgress: (p) => { ctx.chainProgress = p; } }).then((r) => { ctx.chainFinal = r; return r; });
  ctx.catalogPromise = new Promise((resolve) => setImmediate(() => resolve(precedent.ruleCatalog(ctx.home)))).then((cat) => {
    ctx.catalog = cat;
    if (ctx.c) { const law = ctx.c.law; const rule = cat.get(law.key); if (rule) law.rule = rule; }
    return cat;
  });
  if (ctx.c) ctx.memoryPromise = precedent.memoryOf(ctx.c, { env: ctx.env }).then((m) => { ctx.memory = m; return m; });
}

function remember(ctx, station, { finished = false } = {}) {
  if (!ctx.c) return;
  const d = ctx.c.story.digest;
  const entry = ctx.journey.cases[d] || { stations: [], finished: false, first: Date.now() };
  if (station !== null && !entry.stations.includes(station)) entry.stations.push(station);
  if (finished) entry.finished = true;
  entry.at = Date.now();
  ctx.journey.cases[d] = entry;
  ctx.journey.lastVisit = Date.now();
  ctx.journey.lastTotal = ctx.log.records.length;
  ctx.journey.lastStops = ctx.stopCount;
  saveJourney(ctx.journey, ctx.env);
}

// ── sharing at the verdict ─────────────────────────────────────────────────────────────────────
function share(ctx, key) {
  const s = ctx.c.story;
  const cinema = require('./vestige-cinema.js');
  const noClipboard = ctx.env.VESTIGE_NO_CLIPBOARD === '1';
  try {
    if (key === 's' || key === 'c') {
      const text = key === 's' ? booking.textStrip(s, { code: ctx.code }) : `${booking.INSTALL} play ${ctx.code}`;
      if (noClipboard) return { text: '✔ ready to copy (clipboard off here)', ok: true, copied: text };
      // OSC 52 reaches the clipboard through SSH and tmux; the system command covers the rest.
      process.stdout.write(cinema.clipboardPayload(text));
      const copied = cinema.copyCommand(text);
      return { text: key === 's' ? '✔ text copied: paste it anywhere' : '✔ replay command copied', ok: copied !== false };
    }
    const episodes = require('./vestige-episodes.js');
    const sharer = require('./vestige-share.js');
    const file = episodes.exportImage(s, key === 'g' ? 'gif' : 'png');
    const copied = noClipboard ? { ok: false } : key === 'g' ? sharer.copyFile(file) : sharer.copyImage(file);
    return { text: `✔ saved ${path.basename(file)}${copied.ok ? ' · on the clipboard' : ''}`, ok: true, file };
  } catch (error) {
    return { text: `✗ could not share: ${error.message}`, ok: false, bad: true };
  }
}

// ── the player ─────────────────────────────────────────────────────────────────────────────────
async function play(term, ctx, renderFn, frames, { reduced, startFrame = 0, onKey } = {}) {
  // Plays frames; any key skips to the last one. Returns the key that ended the hold.
  let f = reduced ? frames - 1 : startFrame;
  let tick = 0;
  const draw = () => {
    const { cols, rows } = term.size();
    const cv = new Canvas(cols, rows);
    if (tooSmall(cols, rows)) renderTooSmall(cv, ctx);
    else {
      renderFn(cv, layout(cols, rows), ctx, f, tick);
      if (ctx.help) renderHelp(cv, layout(cols, rows));
    }
    term.draw(cv);
  };
  // A key during the animation skips to its end. Enter, space and → only skip; any other key
  // (f, s, m, a digit, ←, ?) skips and then does its job, so a hint on screen always works.
  let carried = null;
  while (f < frames - 1) {
    draw();
    const key = await term.next(FRAME_MS);
    if (key && key !== 'resize') {
      if (ctx.help) { ctx.help = false; continue; }
      if (['q', 'esc', 'ctrl-d'].includes(key)) return key;
      f = frames - 1;
      if (!['enter', 'space', 'right', 'l'].includes(key)) carried = key;
      break;
    }
    f++;
  }
  for (;;) {
    draw();
    let key;
    if (carried) { key = carried; carried = null; } else key = await term.next(reduced ? undefined : HOLD_TICK_MS);
    if (key === null) { tick++; continue; }
    if (key === 'resize') continue;
    if (ctx.help) { ctx.help = false; continue; }
    if (key === '?') { ctx.help = true; continue; }
    if (onKey) {
      const result = await onKey(key, { redraw: draw });
      if (result === 'stay') continue;
      if (result) return result;
    }
    return key;
  }
}

async function travel(term, ctx, from, to, reduced) {
  if (reduced) return null;
  const steps = 13;
  for (let k = 0; k <= steps; k++) {
    const { cols, rows } = term.size();
    const cv = new Canvas(cols, rows);
    if (tooSmall(cols, rows)) renderTooSmall(cv, ctx);
    else renderMap(cv, layout(cols, rows), ctx, { from, to, t: k / steps, tick: k, travelling: true });
    term.draw(cv);
    const key = await term.next(FRAME_MS);
    if (key && key !== 'resize') {
      if (key === 'q' || key === 'ctrl-d') return key;
      // any key ends the flight; one that means something at the station is kept for it
      if (!['enter', 'space', 'right', 'esc'].includes(key)) term.pushBack(key);
      break;
    }
  }
  return null;
}

// The map: arrows move the highlight round the ring, enter goes there, a digit jumps, esc or m closes it.
async function mapScreen(term, ctx, at, reduced, finale = false) {
  let selected = at;
  for (;;) {
    const k = await play(term, ctx, (cv, L, c, f, tick) => renderMap(cv, L, c, { to: selected, tick, finale }), 1, {
      reduced,
      onKey: async (key) => {
        if (finale) return ['enter', 'space', 'right', 'n', 'r', 'q', 'esc', 'ctrl-d'].includes(key) ? null : 'stay';
        if (['right', 'down', 'l', 'j', 'tab'].includes(key)) { selected = (selected + 1) % 6; return 'stay'; }
        if (['left', 'up', 'h', 'k', 'shift-tab'].includes(key)) { selected = (selected + 5) % 6; return 'stay'; }
        if (['enter', 'space', 'q', 'esc', 'm', 'ctrl-d'].includes(key) || /^[1-6]$/.test(key)) return null;
        return 'stay';
      },
    });
    if (finale) return k;
    if (k === 'enter' || k === 'space') return String(selected + 1);
    if (k === 'esc' || k === 'm') return 'close';
    return k;
  }
}

async function forgeScene(term, ctx, reduced) {
  const r = precedent.forge(ctx.log, ctx.c.index);
  ctx.forge = { active: true, f: 0, result: r };
  let key = null;
  const total = FORGE_FRAMES;
  for (let k = reduced ? total - 1 : 0; k < total; k++) {
    ctx.forge.f = k;
    const { cols, rows } = term.size();
    const cv = new Canvas(cols, rows);
    if (tooSmall(cols, rows)) renderTooSmall(cv, ctx); else renderRecord(cv, layout(cols, rows), ctx, RECORD_FRAMES - 1, k);
    term.draw(cv);
    key = await term.next(FRAME_MS);
    if (key && key !== 'resize') {
      // f or esc closes the forge at once; q leaves; any other key skips to the end
      if (key === 'f' || key === 'esc') { ctx.forge = null; return 'closed'; }
      if (key === 'q' || key === 'ctrl-d') return key;
      if (k >= total - 2) { term.pushBack(key); break; }
      k = total - 2;
    }
  }
  ctx.forge.f = total - 1;
  return null;
}

const LEAVE = ['q', 'esc', 'ctrl-d'];
const NEXT = ['enter', 'space', 'right', 'l'];
const BACK = ['left', 'backspace', 'h'];

async function journeyLoop(term, ctx, reduced) {
  // the title: enter follows the case, n picks another, q leaves; any other key waits
  const key = await play(term, ctx, renderTitle, TITLE_FRAMES, {
    reduced,
    onKey: async (k) => {
      if (k === 'n' && ctx.cases.length > 1) return 'next-case';
      return NEXT.includes(k) || LEAVE.includes(k) ? null : 'stay';
    },
  });
  if (LEAVE.includes(key)) return 'left';
  if (!ctx.c) return 'empty';
  if (key === 'next-case') {
    const at = ctx.cases.findIndex((k) => k.story.digest === ctx.c.story.digest);
    setCase(ctx, ctx.cases[(at + 1) % ctx.cases.length]);
    ctx.memoryPromise = precedent.memoryOf(ctx.c, { env: ctx.env }).then((m) => { ctx.memory = m; });
  }
  let station = 0;
  let from = null;
  let skipTravel = false;
  let resume = false; // back from the map: the station returns settled, not replayed
  for (;;) {
    if (!skipTravel) {
      const travelled = await travel(term, ctx, from, station, reduced);
      if (LEAVE.includes(travelled)) return 'left';
    }
    skipTravel = false;
    remember(ctx, station);
    ctx.visited.add(station);
    if (station === 1 && !ctx.chainFinal) await ctx.chainPromise;
    if (station === 2 && !ctx.memory && ctx.memoryPromise) await Promise.race([ctx.memoryPromise, stage.sleep(1600)]);
    if (station === 3 && !ctx.catalog) await Promise.race([ctx.catalogPromise, stage.sleep(2500)]);
    if (station === 0) story.markSeen(ctx.c.story.digest, ctx.env);
    const startFrame = resume ? FRAMES[station] - 1 : 0;
    resume = false;
    const k = await play(term, ctx, RENDER[station], FRAMES[station], {
      reduced,
      startFrame,
      onKey: async (key2) => {
        if (station === 1 && key2 === 'f') {
          if (ctx.forge && ctx.forge.active) { ctx.forge = null; return 'stay'; }
          const end = await forgeScene(term, ctx, reduced);
          if (end === 'q' || end === 'ctrl-d') return end;
          return 'stay';
        }
        if (station === 1 && key2 === 'esc' && ctx.forge && ctx.forge.active) { ctx.forge = null; return 'stay'; }
        if (station === 5 && ['s', 'i', 'g', 'c'].includes(key2)) {
          ctx.toast = { text: key2 === 'g' ? 'saving the gif…' : key2 === 'i' ? 'saving the image…' : '…', until: Date.now() + 60000 };
          await stage.sleep(10);
          const result = share(ctx, key2);
          ctx.toast = { ...result, until: Date.now() + 4000 };
          return 'stay';
        }
        if (key2 === 'r') return 'replay';
        // only these keys move; every other key is ignored
        if (LEAVE.includes(key2) || NEXT.includes(key2) || BACK.includes(key2) || ['m', 'tab'].includes(key2) || /^[1-6]$/.test(key2)) return null;
        return 'stay';
      },
    });
    ctx.forge = null;
    if (LEAVE.includes(k)) return 'left';
    if (k === 'replay') { skipTravel = true; continue; }
    if (k === 'm' || k === 'tab') {
      const m = await mapScreen(term, ctx, station, reduced);
      if (m === 'q' || m === 'ctrl-d') return 'left';
      if (/^[1-6]$/.test(m) && Number(m) - 1 !== station) { from = station; station = Number(m) - 1; continue; }
      skipTravel = true;
      resume = true;
      continue;
    }
    if (/^[1-6]$/.test(k)) {
      if (Number(k) - 1 === station) { skipTravel = true; resume = true; continue; }
      from = station; station = Number(k) - 1; continue;
    }
    if (BACK.includes(k)) {
      if (station === 0) { skipTravel = true; resume = true; continue; }
      from = station; station -= 1; continue;
    }
    if (station === 5) {
      remember(ctx, 5, { finished: true });
      const back = await travel(term, ctx, 5, 0, reduced);
      if (LEAVE.includes(back)) return 'finished';
      const fin = await mapScreen(term, ctx, 0, reduced, true);
      if (fin === 'r') { from = null; station = 0; ctx.visited = new Set(); continue; }
      if (NEXT.includes(fin) || fin === 'n') {
        const next = ctx.cases.find((k2) => !(ctx.journey.cases[k2.story.digest] && ctx.journey.cases[k2.story.digest].finished));
        if (!next) return 'finished';
        setCase(ctx, next);
        ctx.memoryPromise = precedent.memoryOf(ctx.c, { env: ctx.env }).then((m) => { ctx.memory = m; });
        from = null;
        station = 0;
        continue;
      }
      return 'finished';
    }
    from = station;
    station++;
  }
}

// How this user starts the journey again: the package dispatcher names its own command.
const entry = (env = process.env) => (/^[\w@/ .-]{1,60}$/.test(env.VESTIGE_ENTRY || '') ? env.VESTIGE_ENTRY : 'node vestige-journey.js');

// What stays in scrollback keeps to ASCII when the terminal's locale is not UTF-8.
// Lines wrap at word boundaries to the terminal's width, continuing under the value column.
function fitLines(text, width) {
  if (!width || width < 40) return text;
  return text.split('\n').map((line) => {
    if (chars(line).length <= width) return line;
    const indent = /^\d \S/.test(line) || /^ {18}/.test(line) ? 18 : 2;
    const out = [];
    let rest = line;
    while (chars(rest).length > width) {
      const head = chars(rest).slice(0, width).join('');
      let cut = head.lastIndexOf(' ');
      if (cut <= indent) cut = width;
      out.push(rest.slice(0, cut).trimEnd());
      rest = ' '.repeat(indent) + rest.slice(cut).trimStart();
    }
    out.push(rest);
    return out.join('\n');
  }).join('\n');
}
const plainText = (text, env) => fitLines(stage.asciiOnly(env) ? [...text].map(stage.asciiOf).join('') : text, process.stdout.isTTY ? process.stdout.columns : 0);

async function main(argv = process.argv.slice(2), env = process.env) {
  const want = (flag) => argv.includes(flag);
  const valueOf = (flag) => { const i = argv.indexOf(flag); return i >= 0 ? argv[i + 1] : null; };
  if (want('--help') || want('-h')) {
    console.log(`usage: ${entry(env)} [--case <number|receipt prefix>] [--plain] [--still]\n\n  follow one real Operator stop around the loop: the stop, the record, the memory,\n  the precedent, the aftermath, the verdict. keys: enter next · ← back · m map · ? keys · q leave`);
    return 0;
  }
  const ctx = buildContext({ env, token: valueOf('--case') });
  if (valueOf('--case') && !ctx.c) { console.error(`no case matches ${valueOf('--case')}`); return 1; }
  const interactive = process.stdout.isTTY && process.stdin.isTTY && !want('--plain') && env.TERM !== 'dumb';
  if (!interactive) {
    if (!ctx.c) { console.log(ctx.empty); return 0; }
    ctx.catalog = precedent.ruleCatalog(ctx.home);
    setCase(ctx, ctx.cases.find((k) => k.story.digest === ctx.c.story.digest));
    const chain = await precedent.verifyChain(ctx.log);
    const memory = await precedent.memoryOf(ctx.c, { env });
    console.log(plainText(precedent.caseFile(ctx.c, chain, memory, ctx.code), env));
    return 0;
  }
  const reduced = stage.reducedMotion(env, process.argv);
  const term = new stage.Term({ env, onInterrupt: () => { try { remember(ctx, null); } catch {} } });
  startLoads(ctx);
  term.open();
  let outcome;
  try {
    outcome = await journeyLoop(term, ctx, reduced);
  } finally {
    term.restore();
  }
  if (ctx.c) {
    remember(ctx, null);
    const chain = ctx.chainFinal || (await ctx.chainPromise);
    const memory = ctx.memory || (await Promise.race([ctx.memoryPromise, stage.sleep(1500)]));
    console.log(plainText(precedent.caseFile(ctx.c, chain, memory, ctx.code), env));
    const left = ctx.cases.filter((k) => !(ctx.journey.cases[k.story.digest] && ctx.journey.cases[k.story.digest].finished)).length;
    console.log(`\n${outcome === 'finished' ? 'the loop is closed.' : 'progress kept.'} ${left ? `${fmt(left)} case${left === 1 ? '' : 's'} not followed yet · ` : ''}again: ${entry(env)}`);
  } else {
    console.log(ctx.empty);
  }
  return 0;
}

if (require.main === module) main().then((code) => process.exit(code), (error) => { console.error(`vestige journey: ${error && error.stack ? error.stack : error}`); process.exit(1); });

module.exports = {
  main, buildContext, setCase, startLoads, layout, tooSmall, STATIONS, THEMES, RENDER, FRAMES, TITLE_FRAMES, FORGE_FRAMES,
  renderTitle, renderMap, renderStop, renderRecord, renderMemory, renderPrecedent, renderAftermath, renderVerdict, renderTooSmall, renderHelp,
  loadJourney, saveJourney, journeyPath, pickCase, journeyLoop, play, share,
};
