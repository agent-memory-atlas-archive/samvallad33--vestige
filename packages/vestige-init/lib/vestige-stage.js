// The stage the v5 Precedent journey plays on: a cell canvas, a colour ladder (truecolor, 256, 16,
// none), a terminal session that always gives the terminal back, and a key reader that never drops
// keys. Zero dependencies.

const SYNC_ON = '\x1b[?2026h';
const SYNC_OFF = '\x1b[?2026l';

const mix = (a, b, t) => [0, 1, 2].map((i) => Math.round(a[i] + (b[i] - a[i]) * Math.max(0, Math.min(1, t))));
const chars = (text) => [...String(text)];

// Characters that some terminal draws two cells wide (emoji, CJK, wide symbols) would push the rest of
// the row out of place, so the canvas never holds one.
const WIDE = /[ᄀ-ᅟ〈〉⌚⌛⏩-⏬⏰⏳◽◾☔☕♈-♓♿⚓⚡⚪⚫⚽⚾⛄⛅⛎⛔⛪⛲-⛵⛺⛽✅✊✋✨❌❎❓-❕❗➕-➗➰➿⬛⬜⭐⭕⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹏＀-｠￠-￦]|[\ud83c-\ud83e][\udc00-\udfff]|️/;
const safeChar = (ch) => (ch.length === 1 && ch >= ' ' && !WIDE.test(ch) ? ch : (WIDE.test(ch) || ch < ' ' ? '?' : ch));

class Canvas {
  constructor(cols, rows) {
    this.cols = cols;
    this.rows = rows;
    this.cells = Array.from({ length: cols * rows }, () => ({ ch: ' ', fg: null, bg: null, bold: false, dim: false, rev: false }));
    this.reserved = new Uint8Array(cols * rows);
  }
  at(x, y) { return x >= 0 && y >= 0 && x < this.cols && y < this.rows ? this.cells[y * this.cols + x] : null; }
  put(x, y, text, style = {}) {
    let cx = x;
    for (const raw of chars(text)) {
      if (raw === '\ufe0f' || raw === '\u200d') continue;
      const cell = this.at(cx++, y);
      if (!cell) continue;
      cell.ch = safeChar(raw);
      cell.fg = style.fg || null;
      cell.bold = Boolean(style.bold);
      cell.dim = Boolean(style.dim);
      cell.rev = Boolean(style.rev);
      if (style.bg !== undefined) cell.bg = style.bg;
    }
    return cx;
  }
  gradient(x, y, text, from, to, style = {}) {
    const list = chars(text);
    list.forEach((ch, i) => this.put(x + i, y, ch, { ...style, fg: mix(from, to, list.length > 1 ? i / (list.length - 1) : 0) }));
    return x + list.length;
  }
  fill(x0, y0, x1, y1, bg) {
    for (let y = Math.max(0, y0); y <= Math.min(this.rows - 1, y1); y++) for (let x = Math.max(0, x0); x <= Math.min(this.cols - 1, x1); x++) this.cells[y * this.cols + x].bg = bg;
  }
  // Words and pictures each get their own rectangle; decoration may only use cells nobody reserved.
  reserve(x0, y0, x1, y1) {
    for (let y = Math.max(0, y0); y <= Math.min(this.rows - 1, y1); y++) for (let x = Math.max(0, x0); x <= Math.min(this.cols - 1, x1); x++) this.reserved[y * this.cols + x] = 1;
  }
  free(x, y) { return x >= 0 && y >= 0 && x < this.cols && y < this.rows && !this.reserved[y * this.cols + x]; }
  blit(lines, x, y) { // a booking Grid's cells, copied in place
    for (let r = 0; r < lines.rows; r++) for (let c = 0; c < lines.cols; c++) {
      const src = lines.cells[r * lines.cols + c];
      const dst = this.at(x + c, y + r);
      if (!dst) continue;
      Object.assign(dst, { ch: safeChar(src.ch), fg: src.fg, bold: src.bold, dim: src.dim, rev: false });
      if (src.bg) dst.bg = src.bg;
    }
  }
  text() { // plain rows, for tests and screen readers
    const out = [];
    for (let y = 0; y < this.rows; y++) out.push(this.cells.slice(y * this.cols, (y + 1) * this.cols).map((c) => c.ch).join(''));
    return out;
  }
}

// The 16 colours as xterm draws them, for terminals that have no more.
const ANSI16 = [[0, 0, 0], [205, 0, 0], [0, 205, 0], [205, 205, 0], [0, 0, 238], [205, 0, 205], [0, 205, 205], [229, 229, 229],
  [127, 127, 127], [255, 0, 0], [0, 255, 0], [255, 255, 0], [92, 92, 255], [255, 0, 255], [0, 255, 255], [255, 255, 255]];
// 16 colours: a saturated colour keeps its hue (red stays red, violet stays magenta), so meaning
// survives; greys and near-greys pick black, grey or white by brightness.
function nearest16(rgb) {
  const [r, g, b] = rgb;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const light = (max + min) / 2;
  if (max - min < 48) return light < 50 ? 0 : light < 140 ? 8 : light < 215 ? 7 : 15;
  let h;
  if (max === r) h = ((g - b) / (max - min) + 6) % 6;
  else if (max === g) h = (b - r) / (max - min) + 2;
  else h = (r - g) / (max - min) + 4;
  h *= 60;
  // hue sectors to the six ANSI hues: red 1, yellow 3, green 2, cyan 6, blue 4, magenta 5
  const base = h < 25 || h >= 330 ? 1 : h < 70 ? 3 : h < 160 ? 2 : h < 200 ? 6 : h < 255 ? 4 : 5;
  if (max < 70) return 0;
  return max >= 160 ? base + 8 : base;
}

// 256 colours: the nearest of the 6x6x6 cube and the 24-step grey ramp, by distance.
const CUBE = [0, 95, 135, 175, 215, 255];
function to256(r, g, b) {
  const level = (v) => { let best = 0; for (let i = 1; i < 6; i++) if (Math.abs(CUBE[i] - v) < Math.abs(CUBE[best] - v)) best = i; return best; };
  const [lr, lg, lb] = [level(r), level(g), level(b)];
  const cube = [CUBE[lr], CUBE[lg], CUBE[lb]];
  const avg = (r + g + b) / 3;
  const gi = Math.max(0, Math.min(23, Math.round((avg - 8) / 10)));
  const grey = 8 + gi * 10;
  const d = (c) => (c[0] - r) ** 2 + (c[1] - g) ** 2 + (c[2] - b) ** 2;
  return d([grey, grey, grey]) < d(cube) ? 232 + gi : 16 + 36 * lr + 6 * lg + lb;
}

function sgrFor(cell, mode) {
  if (mode === 'none') return '\x1b[0' + (cell.bold ? ';1' : '') + (cell.rev ? ';7' : '') + 'm';
  let s = '\x1b[0' + (cell.bold ? ';1' : '') + (cell.dim ? ';2' : '') + (cell.rev ? ';7' : '');
  const paint = (rgb, base) => {
    if (mode === 'truecolor') return `;${base};2;${rgb[0]};${rgb[1]};${rgb[2]}`;
    if (mode === '256') return `;${base};5;${base === 48 && Math.max(...rgb) < 64 ? 233 : to256(rgb[0], rgb[1], rgb[2])}`;
    // 16 colours: dark backgrounds read as black, so a near-black sky stays black, not blue.
    const n = nearest16(rgb);
    if (base === 48) return `;${n < 8 ? 40 + n : 100 + n - 8}`;
    return `;${n < 8 ? 30 + n : 90 + n - 8}`;
  };
  if (cell.fg) s += paint(cell.fg, 38);
  if (cell.bg) s += mode === '16' && Math.max(...cell.bg) < 70 ? ';40' : paint(cell.bg, 48);
  return s + 'm';
}

// For terminals whose locale is not UTF-8: every drawing character has a plain ASCII stand-in.
const ASCII = {
  '─': '-', '━': '-', '┄': '-', '│': '|', '┃': '|', '┌': '+', '┐': '+', '└': '+', '┘': '+', '├': '+', '┤': '+', '┬': '+', '┴': '+', '┼': '+',
  '╭': '+', '╮': '+', '╰': '+', '╯': '+', '═': '=', '║': '|', '╔': '+', '╗': '+', '╚': '+', '╝': '+', '█': '#', '▀': '#', '▄': '#', '▌': '#',
  '▐': '#', '░': ':', '▒': ':', '▓': '#', '▁': '_', '●': 'o', '○': '.', '•': '*', '·': '.', '…': '.', '←': '<', '→': '>', '↑': '^', '↓': 'v',
  '✔': 'v', '✗': 'x', '×': 'x', '◉': '@', '◎': '@', '▸': '>', '◂': '<', '›': '>', '»': '>', '«': '<', '⋆': '*', '✦': '*', '✧': '*', '˙': "'",
};
const asciiOf = (ch) => (ch <= '~' ? ch : ASCII[ch] || '?');

function renderRow(canvas, y, mode, ascii = false) {
  let line = '';
  let last = '';
  for (let x = 0; x < canvas.cols; x++) {
    const cell = canvas.cells[y * canvas.cols + x];
    const sgr = mode === 'plain' ? '' : sgrFor(cell, mode);
    if (sgr !== last) { line += sgr; last = sgr; }
    line += ascii ? asciiOf(cell.ch) : cell.ch;
  }
  return mode === 'plain' ? line : line + '\x1b[0m';
}

// An explicit non-UTF-8 locale (C, POSIX, ISO-8859-x…) gets ASCII drawing. Unset means UTF-8.
function asciiOnly(env = process.env) {
  if (env.VESTIGE_ASCII === '1') return true;
  if (env.VESTIGE_ASCII === '0') return false;
  const loc = env.LC_ALL || env.LC_CTYPE || env.LANG || '';
  return Boolean(loc) && !/utf-?8/i.test(loc) && (/^(C|POSIX)$/i.test(loc) || /\./.test(loc));
}

// truecolor, 256, 16 or none. VESTIGE_COLOR forces one; NO_COLOR and TERM=dumb always win.
function colorMode(env = process.env, stream = process.stdout) {
  if (!stream || !stream.isTTY) return 'none';
  if (env.NO_COLOR !== undefined && env.NO_COLOR !== '') return 'none';
  if (env.TERM === 'dumb' || env.FORCE_COLOR === '0') return 'none';
  if (['truecolor', '256', '16', 'none'].includes(env.VESTIGE_COLOR)) return env.VESTIGE_COLOR;
  if (/truecolor|24bit/i.test(env.COLORTERM || '')) return 'truecolor';
  if (env.WT_SESSION || /^(iTerm\.app|WezTerm|ghostty|vscode|WarpTerminal|Hyper|Tabby|rio)$/.test(env.TERM_PROGRAM || '')) return 'truecolor';
  if (/kitty|ghostty|wezterm|alacritty|foot|contour|rio/.test(env.TERM || '')) return 'truecolor';
  let depth = 4;
  try { depth = typeof stream.getColorDepth === 'function' ? stream.getColorDepth(env) : 4; } catch {}
  if (depth >= 24) return 'truecolor';
  if (depth >= 8 || /256/.test(env.TERM || '')) return '256';
  return depth >= 4 ? '16' : 'none';
}

const reducedMotion = (env = process.env, argv = process.argv) => env.VESTIGE_REDUCE_MOTION === '1' || env.NO_MOTION !== undefined || Boolean(env.CI) || argv.includes('--still');

// Ctrl-Z for a full-screen program, as a shell expects:
// - under npm/npx, which catch SIGTSTP and would keep the job running (the shell then waits on a job
//   that never stops), the whole group gets SIGSTOP, which nothing can catch;
// - otherwise the group gets SIGTSTP with its default action, the stop a terminal sends itself;
// - with no job-control shell above (an orphaned group: exec from a login shell, a tmux window,
//   setsid) the kernel discards SIGTSTP, so the program takes the screen back instead of freezing.
// The shell's fg sends SIGCONT to the group, and the caller's resume takes the screen again.
function parentCatchesStop() {
  try {
    const { execFileSync } = require('child_process');
    const comm = execFileSync('ps', ['-o', 'command=', '-p', String(process.ppid)], { encoding: 'utf8', timeout: 1000 }).trim();
    return /(^|\/)(npm|npx|node|pnpm|yarn|bun)(\s|$)|npm exec|npm-cli|npx-cli/.test(comm);
  } catch { return false; }
}
function stopJob(resumeIfNotStopped) {
  const asked = Date.now();
  if (parentCatchesStop()) {
    try { process.kill(0, 'SIGSTOP'); } catch { try { process.kill(process.pid, 'SIGSTOP'); } catch {} }
    return;
  }
  const listeners = process.listeners('SIGTSTP');
  for (const l of listeners) process.off('SIGTSTP', l);
  try { process.kill(0, 'SIGTSTP'); } catch {}
  for (const l of listeners) process.on('SIGTSTP', l);
  // still running a moment later and no time passed stopped: nobody can resume us, so come back
  setTimeout(() => { if (Date.now() - asked < 500) resumeIfNotStopped(); }, 60);
}

// One terminal session. open() takes the screen, restore() always gives it back, on any exit path.
class Term {
  constructor({ out = process.stdout, input = process.stdin, env = process.env, onInterrupt } = {}) {
    this.out = out;
    this.input = input;
    this.mode = colorMode(env, out);
    this.ascii = asciiOnly(env);
    this.sync = this.mode !== 'none';
    this.queue = [];
    this.waiters = [];
    this.shown = [];
    this.resized = false;
    this.opened = false;
    this.onInterrupt = onInterrupt || (() => {});
    this.onData = (buffer) => this.feed(buffer);
    this.onResize = () => { this.resized = true; this.shown = []; this.wake('resize'); };
    this.onExit = () => this.restore();
    this.onSignal = (signal) => { this.suspended = false; this.restore(); process.exit(signal === 'SIGTERM' ? 143 : signal === 'SIGHUP' ? 129 : 130); };
    this.onCrash = (error) => { this.restore(); process.stderr.write(`vestige journey: ${error && error.stack ? error.stack : error}\n`); process.exit(1); };
    // ctrl-z, or a stop signal from outside: give the terminal back, stop, and take it again on resume
    this.onStop = () => this.suspend();
    this.onCont = () => { if (this.suspended) { this.suspended = false; this.open(); } this.shown = []; this.resized = true; this.wake('resize'); };
  }
  size() { return { cols: this.out.columns || 80, rows: this.out.rows || 24 }; }
  open() {
    if (this.opened) return;
    this.opened = true;
    this.wasRaw = Boolean(this.input.isRaw);
    try { if (this.input.isTTY) this.input.setRawMode(true); } catch {}
    this.input.resume();
    this.input.on('data', this.onData);
    this.out.on('resize', this.onResize);
    if (!this.lifeSignals) {
      this.lifeSignals = true;
      process.on('exit', this.onExit);
      for (const s of ['SIGTERM', 'SIGHUP', 'SIGINT']) process.on(s, this.onSignal);
      process.on('uncaughtException', this.onCrash);
      process.on('unhandledRejection', this.onCrash);
    }
    if (process.platform !== 'win32' && !this.jobSignals) { this.jobSignals = true; process.on('SIGTSTP', this.onStop); process.on('SIGCONT', this.onCont); }
    // alternate screen, hidden cursor, no auto-wrap (a full last column must not scroll the screen)
    this.out.write('\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J\x1b[H');
  }
  restore() {
    if (!this.opened) return;
    this.opened = false;
    try { this.out.write('\x1b[0m' + SYNC_OFF + '\x1b[?7h\x1b[?25h\x1b[?1049l'); } catch {}
    try { if (this.input.isTTY) this.input.setRawMode(this.wasRaw); } catch {}
    this.input.off('data', this.onData);
    this.input.pause();
    this.out.off('resize', this.onResize);
    if (this.suspended) return; // stopped, not finished: keep the signal handlers for the way back
    this.lifeSignals = false;
    process.off('exit', this.onExit);
    for (const s of ['SIGTERM', 'SIGHUP', 'SIGINT']) process.off(s, this.onSignal);
    process.off('uncaughtException', this.onCrash);
    process.off('unhandledRejection', this.onCrash);
    if (this.jobSignals) { this.jobSignals = false; process.off('SIGTSTP', this.onStop); process.off('SIGCONT', this.onCont); }
  }
  suspend() {
    if (!this.opened || process.platform === 'win32') return;
    this.suspended = true;
    this.restore();
    stopJob(() => this.onCont());
  }
  draw(canvas) {
    if (!this.opened) return 0;
    let buffer = '';
    if (this.resized) { buffer += '\x1b[0m\x1b[2J'; this.resized = false; }
    for (let y = 0; y < canvas.rows; y++) {
      const line = renderRow(canvas, y, this.mode, this.ascii);
      if (line !== this.shown[y]) { buffer += `\x1b[${y + 1};1H${line}`; this.shown[y] = line; }
    }
    if (buffer) this.out.write((this.sync ? SYNC_ON : '') + buffer + (this.sync ? SYNC_OFF : ''));
    return buffer.length;
  }
  feed(buffer) {
    const text = buffer.toString('utf8');
    for (const key of parseKeys(text)) {
      if (key === 'ctrl-c') { this.onInterrupt(); this.restore(); process.exit(130); }
      if (key === 'ctrl-z') { this.suspend(); continue; }
      this.queue.push(key);
    }
    this.wake();
  }
  wake(reason) { const waiters = this.waiters; this.waiters = []; for (const w of waiters) w(reason); }
  hasKey() { return this.queue.length > 0; }
  // a key that arrived during something that ends (a flight, an animation) goes back for what follows
  pushBack(key) { this.queue.unshift(key); }
  takeKey() { return this.queue.shift() || null; }
  // Resolves with the next key, 'resize', or null after ms.
  next(ms) {
    if (this.queue.length) return Promise.resolve(this.queue.shift());
    return new Promise((resolve) => {
      let timer = null;
      const done = (reason) => { if (timer) clearTimeout(timer); resolve(this.queue.length ? this.queue.shift() : reason === 'resize' ? 'resize' : null); };
      this.waiters.push(done);
      if (ms !== undefined && ms !== Infinity) timer = setTimeout(() => { this.waiters = this.waiters.filter((w) => w !== done); resolve(null); }, ms);
    });
  }
}

function parseKeys(text) {
  const keys = [];
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    const seq = rest.match(/^\x1b(?:\[[0-9;]*[A-Za-z~]|O[A-Za-z])/);
    if (seq) {
      const tail = seq[0].slice(-1);
      keys.push({ A: 'up', B: 'down', C: 'right', D: 'left', H: 'home', F: 'end', Z: 'shift-tab' }[tail] || (seq[0] === '\x1b[3~' ? 'delete' : 'other'));
      i += seq[0].length;
      continue;
    }
    const ch = text[i];
    i++;
    if (ch === '\x03') keys.push('ctrl-c');
    else if (ch === '\x04') keys.push('ctrl-d');
    else if (ch === '\x1a') keys.push('ctrl-z');
    else if (ch === '\r' || ch === '\n') keys.push('enter');
    else if (ch === ' ') keys.push('space');
    else if (ch === '\t') keys.push('tab');
    else if (ch === '\x1b') keys.push('esc');
    else if (ch === '\x7f' || ch === '\b') keys.push('backspace');
    else if (ch >= ' ') keys.push(ch.toLowerCase());
  }
  return keys;
}

// A seeded random so every frame of a scene is the same on every run (tests and GIFs agree).
function seeded(seed) {
  let s = seed >>> 0 || 1;
  return () => { s ^= s << 13; s >>>= 0; s ^= s >>> 17; s ^= s << 5; s >>>= 0; return s / 4294967296; };
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

module.exports = { stopJob, parentCatchesStop, Canvas, mix, chars, safeChar, renderRow, asciiOnly, asciiOf, sgrFor, colorMode, reducedMotion, Term, parseKeys, seeded, sleep, to256, nearest16, SYNC_ON, SYNC_OFF };
