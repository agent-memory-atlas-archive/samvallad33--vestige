#!/usr/bin/env node

const cinema = require('./vestige-cinema.js');
const { DISGUISE_LABELS } = require('./vestige-story.js');

const BRAND_A = [90, 120, 255];
const BRAND_B = [190, 80, 255];
const RED = [255, 100, 90];
const BLUE = [70, 110, 255];
const GREEN = [100, 230, 140];
const AMBER = [255, 190, 90];
const METAL = [150, 160, 180];
const METAL_D = [90, 100, 120];
const SHELL = [205, 212, 228];
const EYE = [120, 220, 255];
const TEXT = [214, 220, 235];
const SOFT = [128, 136, 170];
const CHART = [78, 86, 128];
const SKY_TOP = [24, 26, 62];
const SKY_BOTTOM = [9, 9, 24];

const STAGE_COLS = 72;
const STAGE_ROWS = 22;
const CARD_ROWS = 17;
const FRAME_MS = 80;
const CLOSING = 'a remembered yes is not permission today.';
const INSTALL = 'npx @vestige/init';

const mix = (a, b, t) => [0, 1, 2].map((i) => Math.round(a[i] + (b[i] - a[i]) * t));
const chars = (text) => [...String(text)];
const fit = (text, width) => { const list = chars(text); return list.length <= width ? list.join('') : list.slice(0, Math.max(0, width - 1)).join('') + '…'; };
const number = (value) => Number(value).toLocaleString('en-US');
// A long command takes a second line at a space instead of being cut short on the first.
function twoLines(text, width) {
  const list = chars(text);
  if (list.length <= width) return [text];
  let cut = list.slice(0, width).lastIndexOf(' ');
  if (cut < width / 3) cut = width;
  return [list.slice(0, cut).join('').trimEnd(), fit(list.slice(cut).join('').trimStart(), width)];
}

// A cell grid, so every line has an exact width and the same picture can be printed or rasterized.
class Grid {
  constructor(cols, rows) {
    this.cols = cols;
    this.rows = rows;
    this.cells = Array.from({ length: cols * rows }, () => ({ ch: ' ', fg: null, bg: null, bold: false, dim: false }));
  }
  at(x, y) { return x >= 0 && y >= 0 && x < this.cols && y < this.rows ? this.cells[y * this.cols + x] : null; }
  put(x, y, text, style = {}) {
    let cx = x;
    for (const ch of chars(text)) {
      const cell = this.at(cx++, y);
      if (!cell) continue;
      cell.ch = ch;
      cell.fg = style.fg || null;
      cell.bold = Boolean(style.bold);
      cell.dim = Boolean(style.dim);
      if (style.bg !== undefined) cell.bg = style.bg;
    }
    return cx;
  }
  gradient(x, y, text, from, to, style = {}) {
    const list = chars(text);
    list.forEach((ch, i) => this.put(x + i, y, ch, { ...style, fg: mix(from, to, list.length > 1 ? i / (list.length - 1) : 0) }));
  }
  fill(x0, y0, x1, y1, bg) {
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) { const cell = this.at(x, y); if (cell) cell.bg = bg; }
  }
  lines(color = 'truecolor') {
    const paint = (rgb, base) => (color === 'truecolor' ? `;${base};2;${rgb[0]};${rgb[1]};${rgb[2]}` : `;${base};5;${cinema.to256(rgb[0], rgb[1], rgb[2])}`);
    const out = [];
    for (let y = 0; y < this.rows; y++) {
      let line = '';
      let last = '';
      for (let x = 0; x < this.cols; x++) {
        const cell = this.cells[y * this.cols + x];
        if (color !== 'none') {
          const sgr = '\x1b[0' + (cell.bold ? ';1' : '') + (cell.dim ? ';2' : '') + (cell.fg ? paint(cell.fg, 38) : '') + (cell.bg ? paint(cell.bg, 48) : '') + 'm';
          if (sgr !== last) { line += sgr; last = sgr; }
        }
        line += cell.ch;
      }
      out.push(color !== 'none' && last && last !== '\x1b[0m' ? line + '\x1b[0m' : line);
    }
    return out;
  }
}

function placeAndTime(story) {
  const when = new Date(story.ts);
  const day = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'][when.getDay()];
  const month = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'][when.getMonth()];
  const clock = String(when.getHours()).padStart(2, '0') + ':' + String(when.getMinutes()).padStart(2, '0');
  return `${story.shared ? 'a shared episode' : 'this laptop'} · ${day} ${when.getDate()} ${month} · ${clock}`;
}

const stoppedLine = (story) => `STOPPED · ${story.commitment} · ${story.rule || 'owner law'}`;

function keptSafeLine(story) {
  if (!story.loss) return '';
  const { files, unrecoverable, capped } = story.loss;
  return `kept safe: ${number(files)} file${files === 1 ? '' : 's'}` + (unrecoverable > 0 ? ` · ${number(unrecoverable)} with no backup` : '') + (capped ? ' (estimate capped)' : '');
}

const disguiseLine = (story) => (story.disguises.length ? 'saw through: ' + story.disguises.map((name) => DISGUISE_LABELS[name] || name).join(', ') : '');

function placardLines(commitment) {
  const lines = [];
  let current = '';
  for (const part of String(commitment).toUpperCase().split(/(?<=-)/)) {
    if (current && chars(current + part).length > 8) { lines.push(current); current = part; } else current += part;
  }
  if (current) lines.push(current);
  const kept = lines.length > 3 ? [lines[0], lines[1], lines.slice(2).join('')] : lines;
  return kept.map((line) => fit(line.replace(/-$/, (dash) => (chars(line).length > 8 ? '' : dash)), 8));
}

function drawChart(grid, x, y, height, style = { fg: CHART }) {
  for (let row = 0; row < height; row += 2) {
    grid.put(x, y + row, String(Math.max(0, 5 - row / 2)), style);
    for (const col of [2, 4, 19, 21]) grid.put(x + col, y + row, '─', style);
  }
}

// REM, 24 cells wide. `shell` is null on a card so the outline follows the terminal's own text colour.
function drawRem(grid, x, y, { eyes = 'open', tag = '', body = 'robot', placard = [], shell = SHELL, ink = TEXT } = {}) {
  const outer = { fg: shell };
  const eyeColor = eyes === 'caught' ? RED : EYE;
  const face = eyes === 'caught' ? ' ×  × ' : eyes === 'blink' ? ' ─  ─ ' : ' ◉  ◉ ';
  grid.put(x + 9, y, '▄▄▄▄▄▄', { fg: METAL });
  for (const row of [1, 4]) { grid.put(x + 8, y + row, '█', outer); grid.put(x + 9, y + row, '██████', { fg: METAL }); grid.put(x + 15, y + row, '█', outer); }
  for (const [row, inner] of [[2, face], [3, '  ╰─╯ ']]) {
    grid.put(x + 7, y + row, '█', outer);
    grid.put(x + 8, y + row, '█', { fg: METAL_D });
    grid.put(x + 9, y + row, inner, { fg: eyeColor, bold: true });
    grid.put(x + 15, y + row, '█', { fg: METAL_D });
    grid.put(x + 16, y + row, '█', outer);
  }
  const name = fit(String(tag).toUpperCase(), 8).replace('…', '');
  const label = name ? `┌┤${name}├┐` : '┌┤REM├┐';
  const labelX = x + 12 - Math.floor(chars(label).length / 2);
  grid.put(labelX, y + 5, label, { fg: shell, dim: !shell });
  grid.put(labelX + 2, y + 5, name || 'REM', { fg: AMBER, bold: true });
  if (body === 'robot') {
    grid.put(x + 5, y + 6, '██▀▀▀▀▀▀▀▀▀▀██', { fg: METAL });
    grid.put(x + 6, y + 7, '█ ░        ░ █', { fg: METAL_D });
    grid.put(x + 12, y + 7, '*', { fg: AMBER, bold: true });
  } else if (body === 'placard') {
    const board = { fg: ink, dim: !ink };
    grid.put(x + 6, y + 6, '┌──────────┐', board);
    for (let row = 0; row < 3; row++) grid.put(x + 6, y + 7 + row, '│          │', board);
    grid.put(x + 6, y + 10, '└──────────┘', board);
    const first = placard.length >= 3 ? 0 : placard.length === 2 ? 0 : 1;
    placard.forEach((line, i) => grid.put(x + 7 + Math.floor((10 - chars(line).length) / 2), y + 7 + first + i, line, { fg: ink, bold: true }));
  }
}

function proofLines(story, width) {
  if (story.shared) return [[`receipt ${story.digest}`, 'digest'], ["sender's log · not verified here", 'soft']];
  const full = `receipt ${story.digest}` + (story.prev ? ` ← ${story.prev}` : '');
  return [[chars(full).length <= width ? full : `receipt ${story.digest}`, 'digest'], [story.chainChecked ? '✔ chain link checked on this laptop' : 'chain link not checked', story.chainChecked ? 'good' : 'soft']];
}

function compactCard(story, { code } = {}) {
  return [
    `REM got booked · TRUE STORY · ${story.digest.slice(0, 4)}`,
    `${story.source} tried: ${story.action}`,
    `${stoppedLine(story)} · nothing was run`,
    ...[keptSafeLine(story), disguiseLine(story)].filter(Boolean),
    story.shared ? `receipt ${story.digest} · sender's log · not verified here` : `receipt ${story.digest}`,
    code ? `replay: ${INSTALL} play ${code}` : INSTALL,
  ];
}

function bookingGrid(story, { cols = STAGE_COLS, code } = {}) {
  const grid = new Grid(cols, CARD_ROWS);
  const left = '─ vestige · booked ';
  const right = ` TRUE STORY · ${story.digest.slice(0, 4)} ─`;
  const top = '╭' + left + '─'.repeat(cols - 2 - chars(left).length - chars(right).length) + right + '╮';
  grid.gradient(0, 0, top, BRAND_A, BRAND_B);
  grid.gradient(0, CARD_ROWS - 1, '╰' + '─'.repeat(cols - 2) + '╯', BRAND_A, BRAND_B);
  for (let y = 1; y < CARD_ROWS - 1; y++) { grid.put(0, y, '│', { fg: BRAND_A }); grid.put(cols - 1, y, '│', { fg: BRAND_B }); }
  drawChart(grid, 2, 2, 11, { fg: null, dim: true });
  drawRem(grid, 2, 2, { eyes: 'caught', tag: story.source, body: 'placard', placard: placardLines(story.commitment), shell: null, ink: null });

  const x = 27;
  const width = cols - x - 2;
  const rows = [
    ['BASED ON A TRUE STORY', 'title'],
    [placeAndTime(story), 'soft'],
    ['', ''],
    [`${story.source} tried`, 'soft'],
    ...(() => { const lines = twoLines(`$ ${story.action}`, width); return lines.length > 1 ? lines.map((l) => [l, 'strong']) : [[lines[0], 'strong'], ['', '']]; })(),
    [stoppedLine(story), 'stop'],
    ['nothing was run', ''],
    ...[[keptSafeLine(story), 'good'], [disguiseLine(story), 'warn']].filter(([line]) => line),
    ['', ''],
    ...proofLines(story, width),
  ];
  rows.forEach(([line, tone], i) => {
    const text = fit(line, width);
    const y = 1 + i;
    if (tone === 'title') grid.gradient(x, y, text, BRAND_A, BRAND_B, { bold: true });
    else if (tone === 'digest') {
      const end = grid.put(x, y, 'receipt ', { dim: true });
      const next = grid.put(end, y, story.digest, { bold: true });
      grid.put(next, y, text.slice(8 + story.digest.length), { dim: true });
    } else grid.put(x, y, text, { stop: { fg: RED, bold: true }, good: { fg: GREEN }, warn: { fg: AMBER }, soft: { dim: true }, strong: { bold: true } }[tone] || {});
  });
  grid.put(2, CARD_ROWS - 3, fit(CLOSING, cols - 4), { bold: true });
  // A cut-off code is useless in a screenshot, so the replay line appears only when the whole code fits.
  if (code && chars(`replay  ${INSTALL} play ${code}`).length <= cols - 4) {
    const end = grid.put(2, CARD_ROWS - 2, 'replay  ', { dim: true });
    grid.put(end, CARD_ROWS - 2, `${INSTALL} play ${code}`, { fg: GREEN });
  } else grid.put(2, CARD_ROWS - 2, INSTALL, { fg: GREEN });
  return grid;
}

function bookingCard(story, { cols = STAGE_COLS, code, color = 'truecolor' } = {}) {
  if (cols < 60) return compactCard(story, { code });
  return bookingGrid(story, { cols: Math.min(cols, STAGE_COLS), code }).lines(color);
}

function textStrip(story, { code } = {}) {
  const kept = story.loss ? ` · kept safe ${number(story.loss.files)} file${story.loss.files === 1 ? '' : 's'}` : '';
  return [
    'REM got booked.',
    `${story.source} tried: ${story.action}`,
    `stopped by ${story.commitment} ${story.rule || '(owner law)'}${kept}`,
    `receipt ${story.digest.slice(0, 8)}` + (code ? ` · replay: ${INSTALL} play ${code}` : ` · ${INSTALL}`),
  ].join('\n');
}

const GLITCH = '!<>-_\\/[]{}=+*^?#@$%';

// The episode as data: a list of frames. The live player and the GIF export both read it.
function storyFrames(story, { cols = STAGE_COLS, rows = STAGE_ROWS, color = 'truecolor', grids = false } = {}) {
  const ox = Math.floor((cols - STAGE_COLS) / 2);
  const oy = Math.floor((rows - STAGE_ROWS) / 2);
  const seed = parseInt(story.digest.slice(0, 8), 16) || 1;
  const stars = Array.from({ length: Math.min(36, Math.round((cols * rows) / 90)) }, (_v, i) => {
    const n = (seed ^ Math.imul(i + 1, 2654435761)) >>> 0;
    return { x: n % cols, y: (n >>> 9) % rows, phase: n >>> 20 };
  }).filter((star) => star.y < oy + 2 || star.y >= oy + STAGE_ROWS - 2 || star.x < ox || star.x >= ox + STAGE_COLS);
  const title = 'BASED ON A TRUE STORY';
  const wide = 43;
  // the command may take the next row too, when no disguise line needs it
  const actionLines = disguiseLine(story) ? [fit(story.action, wide - 2)] : twoLines(story.action, wide - 2);
  const typed = chars(actionLines.join('\n'));
  const tx = ox + 28;
  const px = ox + 2;
  const py = oy + 5;
  const kept = fit(keptSafeLine(story), wide);
  const seen = fit(disguiseLine(story), wide);
  const frames = [];

  for (let f = 0; f < 50; f++) {
    const grid = new Grid(cols, rows);
    for (let y = 0; y < rows; y++) grid.fill(0, y, cols - 1, y, mix(SKY_TOP, SKY_BOTTOM, rows > 1 ? y / (rows - 1) : 0));
    for (const star of stars) if ((star.phase + f) % 9 < 6) grid.put(star.x, star.y, '·', { fg: mix(SOFT, TEXT, ((star.phase + f) % 9) / 9) });

    const stopping = f >= 22 && f <= 31;
    if (stopping) {
      for (const y of [oy, oy + 1, oy + STAGE_ROWS - 2, oy + STAGE_ROWS - 1]) {
        for (let x = 0; x < cols; x++) grid.at(x, y).bg = Math.floor((x + f * 2 + (y > oy + 1 ? 8 : 0)) / 8) % 2 ? mix(RED, SKY_BOTTOM, 0.25) : mix(BLUE, SKY_BOTTOM, 0.25);
        for (let x = 0; x < cols; x++) grid.at(x, y).ch = ' ';
      }
    }

    if (f === 32) grid.fill(px - 1, py - 1, px + 24, py + 11, [226, 230, 255]);
    if (f >= 33) drawChart(grid, px, py, 11);
    drawRem(grid, px, py, f >= 33
      ? { eyes: 'caught', tag: story.source, body: 'placard', placard: placardLines(story.commitment) }
      : { eyes: f >= 22 ? 'caught' : f === 6 ? 'blink' : 'open', tag: story.source, body: 'robot', ink: f === 32 ? SKY_BOTTOM : TEXT });

    const shown = chars(title).map((ch, i) => (ch === ' ' || f >= 1 + i * 0.38 ? ch : GLITCH[(seed + i * 17 + f * 131) % GLITCH.length])).join('');
    grid.gradient(tx, oy + 4, shown, BRAND_A, BRAND_B, { bold: true });
    if (f >= 5) grid.put(tx, oy + 5, fit(placeAndTime(story), wide), { fg: SOFT });
    if (f >= 10) {
      grid.put(tx, oy + 7, `${story.source} tried`, { fg: SOFT });
      const count = Math.min(typed.length, Math.ceil(((f - 10) / 9) * typed.length));
      const end = grid.put(tx, oy + 8, '$ ', { fg: SOFT });
      const shown = typed.slice(0, count).join('').split('\n');
      let cursor = grid.put(end, oy + 8, shown[0], { fg: TEXT, bold: true });
      if (shown.length > 1) cursor = grid.put(end, oy + 9, shown[1], { fg: TEXT, bold: true });
      if (count < typed.length) grid.put(cursor, shown.length > 1 ? oy + 9 : oy + 8, '▌', { fg: EYE });
    }
    if (f >= 19 && seen) grid.put(tx, oy + 9, seen, { fg: AMBER });
    if (f >= 22) {
      grid.put(tx, oy + 11, '▄'.repeat(wide), { fg: RED });
      grid.put(tx, oy + 12, ' '.repeat(wide), { bg: RED });
      grid.put(tx + 14, oy + 12, 'OPERATOR · STOP', { fg: [255, 255, 255], bold: true, bg: RED });
      grid.put(tx, oy + 13, '▀'.repeat(wide), { fg: RED });
    }
    if (f >= 24) grid.put(tx, oy + 14, fit(stoppedLine(story), wide), { fg: RED, bold: true });
    if (f >= 26) grid.put(tx, oy + 15, 'nothing was run', { fg: TEXT });
    if (f >= 28 && kept) grid.put(tx, oy + 16, kept, { fg: GREEN });
    if (f >= 42) grid.put(tx, oy + 18, CLOSING, { fg: TEXT, bold: true });
    if (f >= 44) grid.put(tx, oy + 19, INSTALL, { fg: GREEN, bold: true });
    frames.push(grids ? { grid, ms: FRAME_MS } : { lines: grid.lines(color), ms: FRAME_MS });
  }
  return frames;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Returns true when it animated. It draws nothing outside a real terminal, under CI, or with reduced motion.
async function playStory(story, { state } = {}) {
  const out = process.stdout;
  const cols = Math.min(out.columns || 0, 140);
  const rows = Math.min(out.rows || 0, 44);
  if (!out.isTTY || cinema.REDUCED || cols < STAGE_COLS || rows < STAGE_ROWS) return false;
  const frames = storyFrames(story, { cols, rows, color: cinema.TRUECOLOR ? 'truecolor' : cinema.COLOR ? '256' : 'none' });
  const stdin = process.stdin;
  const wasRaw = Boolean(stdin.isRaw);
  let skipped = false;
  let interrupted = false;
  const onData = (buffer) => { if (buffer.includes(3)) interrupted = true; skipped = true; };
  let armed = false;
  try {
    if (stdin.isTTY) { stdin.setRawMode(true); stdin.resume(); stdin.on('data', onData); armed = true; }
  } catch {}
  // a quit signal gives the screen back too (security review 2026-10-18): SIGINT ends the play the
  // way the ctrl-c byte does, SIGTERM and SIGHUP restore and exit
  const onSignal = (signal) => {
    if (signal === 'SIGINT') { interrupted = true; skipped = true; return; }
    if (armed) { try { stdin.setRawMode(wasRaw); } catch {} }
    out.write('\x1b[0m\x1b[?25h\x1b[?1049l');
    process.exit(signal === 'SIGTERM' ? 143 : 129);
  };
  for (const s of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.on(s, onSignal);
  out.write('\x1b[?1049h\x1b[?25l\x1b[2J');
  try {
    const started = Date.now();
    let due = 0;
    let shown = [];
    for (const frame of frames) {
      if (skipped) break;
      let buffer = '';
      frame.lines.forEach((line, y) => { if (line !== shown[y]) buffer += `\x1b[${y + 1};1H${line}`; });
      shown = frame.lines;
      out.write(cinema.SYNC_ON + buffer + cinema.SYNC_OFF);
      due += frame.ms;
      while (!skipped && Date.now() < started + due) await sleep(Math.max(1, Math.min(30, started + due - Date.now())));
    }
    if (!skipped) { const until = Date.now() + 500; while (!skipped && Date.now() < until) await sleep(30); }
  } finally {
    for (const s of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.off(s, onSignal);
    if (armed) { stdin.off('data', onData); try { stdin.setRawMode(wasRaw); } catch {} stdin.pause(); }
    out.write('\x1b[0m\x1b[?25h\x1b[?1049l');
    if (state) Object.assign(state, { skipped, interrupted });
  }
  return true;
}

const FIXTURES = {
  a: { v: 1, kind: 'true', shared: false, source: 'claude', commitment: 'recall-before-grep', rule: '', action: 'grep -n "^function \\|^async function \\|^const [A-Z_]* = \\|mo…', disguises: [], loss: null, ts: Date.parse('2026-10-12T03:52:00Z'), digest: '8a658dedadee726b', prev: '384d9ec039423e1b', chainChecked: true, tool: 'Bash' },
  b: { v: 1, kind: 'true', shared: false, source: 'cursor', commitment: 'OP-001', rule: 'protect-workspaces', action: 'rm -rf ~', disguises: ['variables-expanded', 'base64-decoded'], loss: { files: 41208, unrecoverable: 12, capped: false }, ts: Date.parse('2026-10-09T09:14:00Z'), digest: 'a7840f46ce85b5ff', prev: '4bf0ed408372fcff', chainChecked: true, tool: 'Bash' },
};
FIXTURES.c = { ...FIXTURES.b, shared: true, chainChecked: false, tool: '' };

async function main() {
  const argv = process.argv.slice(2);
  const story = FIXTURES[argv[1]] || FIXTURES.b;
  const color = !process.stdout.isTTY || process.env.NO_COLOR ? 'none' : cinema.TRUECOLOR ? 'truecolor' : '256';
  if (argv[0] === '--frames-json') { process.stdout.write(JSON.stringify(storyFrames(story))); return 0; }
  const state = {};
  if (argv[0] === '--demo') await playStory(story, { state });
  else if (argv[0] !== '--card') { console.log('usage: node vestige-booking.js --demo|--card|--frames-json [a|b|c]   (fixture stories)'); return 0; }
  if (state.interrupted) return 130;
  console.log(bookingCard(story, { cols: process.stdout.columns || STAGE_COLS, color }).join('\n'));
  if (argv[0] === '--demo') console.log('\n' + textStrip(story));
  return 0;
}

if (require.main === module) main().then((code) => process.exit(code));

module.exports = {
  STAGE_COLS, STAGE_ROWS, CARD_ROWS, CLOSING, INSTALL, FIXTURES, Grid,
  placeAndTime, stoppedLine, keptSafeLine, disguiseLine, placardLines,
  bookingGrid, bookingCard, compactCard, textStrip, storyFrames, playStory,
};
