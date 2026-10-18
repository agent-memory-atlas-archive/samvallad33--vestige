const readline = require('readline');

const IS_TTY = process.stdout.isTTY && process.env.TERM !== 'dumb';
const NO_COLOR = Boolean(process.env.NO_COLOR);
const REDUCED = process.env.VESTIGE_REDUCE_MOTION === '1' || process.env.CI || !IS_TTY;

function colorDepth() {
  const ct = process.env.COLORTERM || '';
  if (ct.includes('truecolor') || ct.includes('24bit')) return 24;
  if (/xterm-kitty|ghostty|wezterm|alacritty|vte/.test(process.env.TERM || '')) return 24;
  if ((process.env.TERM || '').includes('256color')) return 8;
  return IS_TTY ? 4 : 1;
}

const TRUECOLOR = colorDepth() >= 24 && !NO_COLOR;
const COLOR = IS_TTY && !NO_COLOR;

const SYNC_ON = COLOR ? '\x1b[?2026h' : '';
const SYNC_OFF = COLOR ? '\x1b[?2026l' : '';

function rgb(r, g, b) {
  if (!COLOR) return '';
  if (TRUECOLOR) return `\x1b[38;2;${r | 0};${g | 0};${b | 0}m`;
  const gray = Math.abs(r - g) < 8 && Math.abs(g - b) < 8;
  const n = gray ? 232 + Math.round((r + g + b) / 30) : 16 + 36 * Math.round(r / 51) + 6 * Math.round(g / 51) + Math.round(b / 51);
  return `\x1b[38;5;${n}m`;
}

const RESET = COLOR ? '\x1b[0m' : '';
const BOLD = COLOR ? '\x1b[1m' : '';
const DIM = COLOR ? '\x1b[2m' : '';
const c = {
  green: COLOR ? '\x1b[38;5;84m' : '',
  red: COLOR ? '\x1b[38;5;203m' : '',
  amber: COLOR ? '\x1b[38;5;222m' : '',
  cyan: COLOR ? '\x1b[38;5;51m' : '',
  violet: COLOR ? '\x1b[38;5;141m' : '',
  magenta: COLOR ? '\x1b[38;5;213m' : '',
  dim: DIM,
  bold: BOLD,
};

function gradient(text, from, to, phase = 0) {
  if (!COLOR) return text;
  const chars = [...text];
  const n = chars.length;
  return chars.map((ch, i) => {
    if (ch === ' ') return ch;
    const t = (i / Math.max(1, n - 1) + phase) % 1;
    const r = from[0] + (to[0] - from[0]) * t;
    const g = from[1] + (to[1] - from[1]) * t;
    const b = from[2] + (to[2] - from[2]) * t;
    return rgb(r, g, b) + ch + RESET;
  }).join('');
}

function glowText(text, phase = 0) {
  return gradient(text, [90, 120, 255], [190, 80, 255], phase / 24);
}

function frame(rows, width, title) {
  const inner = Math.max(10, width - 4);
  const head = title ? `─ ${title} ` : '';
  const top = `╭${head}${'─'.repeat(Math.max(0, inner - [...head].length + 1))}╮`;
  const body = rows.map((line) => {
    const plain = line.replace(/\x1b\[[0-9;]*m/g, '');
    const visible = [...plain].length;
    const cut = visible > inner ? line.slice(0, inner) : line;
    const cutVisible = [...cut.replace(/\x1b\[[0-9;]*m/g, '')].length;
    return `│ ${cut}${' '.repeat(Math.max(0, inner - cutVisible))} │`;
  });
  return [top, ...body, `╰${'─'.repeat(inner + 2)}╯`];
}

function meter(used, total, width = 24, hue = 'auto') {
  const f = total ? Math.min(1, used / total) : 0;
  const n = Math.round(f * width);
  let color = c.green;
  if (hue === 'auto') color = f > 0.9 ? c.red : f > 0.7 ? c.amber : c.green;
  else if (hue === 'cyan') color = c.cyan;
  return `${color}${'█'.repeat(n)}${RESET}${DIM}${'░'.repeat(width - n)}${RESET}`;
}

const SPARK_CHARS = '▁▂▃▄▅▆▇█';
function sparkline(values, width = 40) {
  const data = values.slice(-width);
  const lo = Math.min(...data, 0);
  const hi = Math.max(...data, 1);
  return data.map((v) => {
    const h = (v - lo) / (hi - lo || 1);
    const color = rgb(90 + h * 100, 120 + h * 120, 255 - h * 130);
    return color + SPARK_CHARS[Math.round(h * 7)] + RESET;
  }).join('');
}

function coverageBars(categories, total, width = 44) {
  const palette = [[51, 181, 229], [141, 111, 245], [84, 220, 140], [222, 178, 90], [213, 120, 200], [90, 200, 190]];
  const lines = [`${BOLD}catalogue coverage${RESET}  ${total.toLocaleString('en-US')} platforms · ${categories.length} categories`];
  categories.forEach(([name, count], i) => {
    const share = count / total;
    const bar = Math.max(1, Math.round(share * width));
    const col = palette[i % palette.length];
    const cc = rgb(col[0], col[1], col[2]);
    lines.push(`  ${name.padEnd(12)}${cc}${'━'.repeat(bar)}${RESET}${DIM}${'·'.repeat(width - bar)}${RESET} ${(share * 100).toFixed(1).padStart(5)}% ${BOLD}${count}${RESET}`);
  });
  return lines;
}

function fingerprint(bytes) {
  const { createHash } = require('crypto');
  return 'SHA256:' + createHash('sha256').update(bytes).digest('base64').replace(/=+$/, '');
}

function randomart(bytes, width = 17, height = 9) {
  const CH = ' .o+=*BOX@%&#/';
  const field = Array.from({ length: height }, () => Array(width).fill(0));
  let x = 8;
  let y = 4;
  for (const byte of bytes) {
    for (let s = 0; s < 4; s++) {
      const step = (byte >> s * 2) & 3;
      x = Math.max(0, Math.min(width - 1, x + (step & 1 ? 1 : -1)));
      y = Math.max(0, Math.min(height - 1, y + (step & 2 ? 1 : -1)));
      field[y][x]++;
    }
  }
  return field.map((row) => `${DIM}[+${RESET}` + row.map((n) => (COLOR ? rgb(80 + Math.min(14, n) * 12, 140, 255 - Math.min(14, n) * 10) : '') + CH[Math.min(14, n)] + (COLOR ? RESET : '')).join('') + `${DIM}+]${RESET}`).join('\n');
}

function hyperlink(text, uri) {
  if (!IS_TTY || process.env.TERM_PROGRAM === 'Apple_Terminal' || (process.env.TERM || '').startsWith('screen')) return `${text} ${DIM}(${uri})${RESET}`;
  return `\x1b]8;;${uri}\x07${text}\x1b]8;;\x07`;
}

function clipboardPayload(text) {
  return '\x1b]52;c;' + Buffer.from(text, 'utf8').toString('base64') + '\x07';
}

function copyCommand(text) {
  const { execFileSync } = require('child_process');
  try {
    if (process.platform === 'darwin') { execFileSync('pbcopy', { input: text, stdio: ['pipe', 'ignore', 'ignore'] }); return true; }
    if (process.platform === 'win32') { execFileSync('clip.exe', { input: text, stdio: ['pipe', 'ignore', 'ignore'] }); return true; }
    execFileSync('wl-copy', { input: text, stdio: ['pipe', 'ignore', 'ignore'] }); return true;
  } catch {
    try { execFileSync('xclip', ['-selection', 'clipboard'], { input: text, stdio: ['pipe', 'ignore', 'ignore'] }); return true; }
    catch { return false; }
  }
}

const ease = {
  outCubic: (t) => 1 - (1 - t) ** 3,
  outQuint: (t) => 1 - (1 - t) ** 5,
};

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function cascade(checks, { animate = !REDUCED } = {}) {
  const started = Date.now();
  let failures = 0;
  for (const check of checks) {
    const t0 = Date.now();
    let outcome;
    try { outcome = await check.run(); } catch (error) { outcome = { ok: false, note: error.message }; }
    const elapsed = Date.now() - t0;
    if (animate) {
      const frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
      const budget = 60 + Math.random() * 60;
      let i = 0;
      const spinStart = Date.now();
      while (Date.now() - spinStart < Math.max(0, budget - elapsed)) {
        process.stdout.write(`\r  ${c.cyan}${frames[i++ % frames.length]}${RESET} ${check.name}`);
        await sleep(80);
      }
    }
    const glyph = outcome.ok ? `${c.green}✔${RESET}` : outcome.warn ? `${c.amber}▲${RESET}` : `${c.red}✗${RESET}`;
    const note = outcome.note ? `${DIM}${outcome.note}${RESET}` : '';
    const ms = DIM + ' ' + (elapsed > 0 ? `${elapsed}ms` : '') + RESET;
    process.stdout.write(`\r  ${glyph} ${BOLD}${check.name.padEnd(14)}${RESET} ${note}${ms}${' '.repeat(8)}\n`);
    if (!outcome.ok && !outcome.warn) failures++;
  }
  if (failures) process.stdout.write(`  ${c.red}${BOLD}boot halted${RESET} ${DIM}${failures} check${failures === 1 ? '' : 's'} failed${RESET}\n`);
  else process.stdout.write(`  ${c.green}${BOLD}boot ok${RESET} ${DIM}all checks passed in ${((Date.now() - started) / 1000).toFixed(2)}s${RESET}\n`);
  return failures === 0;
}

async function glitchResolve(text, { ms = 45, colorFrom = [0, 255, 102], colorTo = [120, 200, 255] } = {}) {
  if (REDUCED) { process.stdout.write(gradient(text, colorFrom, colorTo) + '\n'); return; }
  const pool = '!<>-_\\/[]{}=+*^?#@$%';
  for (let f = 0; f < text.length * 2 + 10; f++) {
    let out = '';
    for (let i = 0; i < text.length; i++) {
      const settle = f - i * 2;
      out += settle > 8 ? text[i] : pool[(Math.random() * pool.length) | 0];
    }
    process.stdout.write(`\r\x1b[2K${gradient(out, colorFrom, colorTo)}`);
    await sleep(ms);
  }
  process.stdout.write(`\r\x1b[2K${gradient(text, colorFrom, colorTo)}\n`);
}

async function countUp(target, label, { ms = 800 } = {}) {
  const t0 = Date.now();
  if (REDUCED) { process.stdout.write(`${BOLD}${target.toLocaleString('en-US')}${RESET} ${DIM}${label}${RESET}\n`); return; }
  for (;;) {
    const p = Math.min(1, (Date.now() - t0) / ms);
    const value = Math.round(target * ease.outCubic(p));
    process.stdout.write(`\r${BOLD}${value.toLocaleString('en-US')}${RESET} ${DIM}${label}${RESET}  `);
    if (p >= 1) break;
    await sleep(33);
  }
  process.stdout.write('\n');
}

async function typewriter(text, { perChar = [12, 40], ms = null } = {}) {
  if (REDUCED || ms === 0) { process.stdout.write(text + '\n'); return; }
  for (let i = 0; i < text.length; i++) {
    const t = i / text.length;
    const delay = ms ? ms / text.length : perChar[0] + (perChar[1] - perChar[0]) * t;
    process.stdout.write(text[i]);
    await sleep(delay);
  }
  process.stdout.write('\n');
}

function receiptCard({ index, action, chain, payload, hash, prev, verified }) {
  const dimHash = (h) => `${DIM}${h.slice(0, 4)}${RESET}${BOLD}${h.slice(4, 20)}${RESET}${DIM}${h.slice(20, 26)}…${RESET}`;
  const row = (k, v) => `│ ${DIM}${k.padEnd(11)}${RESET} ${v}`;
  const title = ` receipt #${index} `;
  const width = 72;
  return [
    `${DIM}┌${'─'.repeat(width - title.length - 2)}${title}┐${RESET}`,
    row('action', action),
    row('timestamp', `${DIM}${new Date().toISOString()}${RESET}`),
    row('chain', `${DIM}block #${index - 1}${RESET} ${c.cyan}→${RESET} ${BOLD}#${index}${RESET}`),
    payload ? row('payload', payload) : null,
    hash ? row('hash', dimHash(hash)) : null,
    prev ? row('prev', `${DIM}${prev.slice(0, 26)}…${RESET}`) : null,
    row('verified', verified ? `${c.green}✔ chain intact${RESET}` : `${c.amber}unverified${RESET}`),
    `${DIM}└${'─'.repeat(width)}┘${RESET}`,
  ].filter(Boolean).join('\n');
}

function shareCard({ title, stats, catalogCategories, catalogTotal, fingerprintLine, randomartBytes, links, shareText }) {
  const width = Math.min(72, process.stdout.columns || 72);
  const lines = [];
  lines.push('');
  lines.push(gradient(`  ${title}`, [90, 120, 255], [190, 80, 255]));
  lines.push('');
  for (const [label, value] of stats) {
    lines.push(`  ${BOLD}${String(value).padEnd(10)}${RESET} ${DIM}${label}${RESET}`);
  }
  if (catalogCategories) {
    lines.push('');
    lines.push(...coverageBars(catalogCategories, catalogTotal, Math.max(20, width - 28)).map((l) => '  ' + l));
  }
  if (fingerprintLine) {
    lines.push('');
    lines.push(`  ${DIM}fingerprint${RESET}  ${fingerprintLine}`);
    if (randomartBytes) lines.push(...randomart(randomartBytes).split('\n').map((l) => '  ' + l));
  }
  if (links && links.length) {
    lines.push('');
    for (const [labelText, uri] of links) lines.push(`  ${c.cyan}▸${RESET} ${hyperlink(labelText, uri)}`);
  }
  const card = frame(lines.map((l) => l.replace(/^  /, '')), width, 'vestige · local proof');
  process.stdout.write(card.join('\n') + '\n');
  if (shareText) {
    process.stdout.write(`\n  ${DIM}press S to copy the share text · any other key continues${RESET}\n`);
  }
  return shareText || null;
}

async function promptShareCopy(shareText) {
  if (!shareText || !IS_TTY) return false;
  return new Promise((resolve) => {
    const wasRaw = process.stdin.isRaw;
    const done = (result) => {
      process.stdin.off('data', onData);
      clearInterval(blinkTimer);
      try { process.stdin.setRawMode(wasRaw); } catch {}
      process.stdin.pause();
      resolve(result);
    };
    let blink = 0;
    const blinkTimer = setInterval(() => { blink++; }, 400);
    const onData = (buffer) => {
      const key = buffer.toString('utf8');
      if (key.toLowerCase() === 's') {
        const via52 = IS_TTY;
        if (via52) process.stdout.write(clipboardPayload(shareText));
        const viaCommand = copyCommand(shareText);
        process.stdout.write(`\r  ${c.green}✔${RESET} share text copied to clipboard${' '.repeat(20)}\n`);
        done(true);
      } else done(false);
    };
    try {
      process.stdin.setRawMode(true);
      process.stdin.resume();
      process.stdin.once('data', onData);
    } catch { done(false); }
  });
}

function score(query, target) {
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  if (!q) return 1;
  let qi = 0;
  let scoreValue = 0;
  let streak = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      streak++;
      scoreValue += 1 + streak * 2 + (ti === 0 || t[ti - 1] === ' ' || t[ti - 1] === '-' || t[ti - 1] === '/' ? 4 : 0);
      qi++;
    } else streak = 0;
  }
  return qi === q.length ? scoreValue / (q.length * 4 + t.length * 0.1) : -1;
}

async function fuzzySearch(items, { placeholder = 'type to search · ↑↓ move · Enter select · Esc cancel', previewWidth = 36, renderPreview } = {}) {
  if (!IS_TTY) return null;
  return new Promise((resolve) => {
    const wasRaw = process.stdin.isRaw;
    let query = '';
    let cursor = 0;
    let offset = 0;
    let phase = 0;
    let timer;
    const resultsFor = () => {
      if (!query) return items.slice(0, 200).map((item, index) => ({ item, s: 1 - index / 1000 }));
      return items.map((item) => ({ item, s: score(query, (item.searchText || item.name || String(item))) }))
        .filter((r) => r.s >= 0).sort((a, b) => b.s - a.s).slice(0, 200);
    };
    const cleanup = (value) => {
      clearInterval(timer);
      process.stdin.off('keypress', onKey);
      process.stdout.off('resize', draw);
      try { process.stdin.setRawMode(wasRaw); } catch {}
      process.stdin.pause();
      process.stdout.write('\x1b[0m\x1b[?25h\x1b[?1049l');
      resolve(value);
    };
    const draw = () => {
      const width = Math.max(40, process.stdout.columns || 80);
      const height = Math.max(12, process.stdout.rows || 24);
      const results = resultsFor();
      if (cursor >= results.length) cursor = Math.max(0, results.length - 1);
      const rowsAvailable = height - 7;
      if (cursor < offset) offset = cursor;
      if (cursor >= offset + rowsAvailable) offset = cursor - rowsAvailable + 1;
      const focused = results[cursor];
      const leftWidth = width >= 90 ? Math.floor(width * 0.45) : width - 2;
      const listRows = results.slice(offset, offset + rowsAvailable).map((result, i) => {
        const index = offset + i;
        const label = (result.item.label || result.item.name || String(result.item)).slice(0, leftWidth - 6);
        const verdict = result.item.verdict ? result.item.verdict : '';
        const verdictColor = { 'SEAM-VERIFIED': c.green, 'SEAM-REPORTED': c.cyan, 'MCP-CLIENT': c.violet, 'SANDBOX': c.amber }[verdict] || DIM;
        const active = index === cursor;
        return `${active ? c.magenta + '›' + RESET : ' '} ${active ? BOLD + label + RESET : label}${' '.repeat(Math.max(1, leftWidth - 6 - label.length))}${verdictColor}${verdict.slice(0, 13)}${RESET}`;
      });
      const panelBody = listRows.length ? listRows : [DIM + `no match for "${query}"` + RESET];
      const counter = `${results.length.toLocaleString('en-US')} match${results.length === 1 ? '' : 'es'}`;
      const left = frame(panelBody, leftWidth, `${counter}`);
      let right = [];
      if (focused && renderPreview) {
        right = renderPreview(focused.item, width >= 90 ? width - leftWidth - 6 : leftWidth).slice(0, rowsAvailable + 2);
      }
      const searchLine = `  ${c.cyan}search${RESET} ${BOLD}${query}${RESET}${COLOR ? '\x1b[5m▏\x1b[25m' : ''}${DIM}  ${placeholder}${RESET}`;
      const body = width >= 90 ? left.map((line, i) => line + '   ' + (right[i] || '')) : left;
      const out = [searchLine, '', ...body];
      if (width < 90 && focused && renderPreview) {
        out.push('', `${c.violet}${focused.item.name}${RESET}`, ...renderPreview(focused.item, leftWidth).slice(0, 6));
      }
      process.stdout.write(SYNC_ON + '\x1b[H' + out.map((line) => (line.replace(/\x1b\[[0-9;]*m/g, '').length > width - 1 ? line.slice(0, width - 1) : line) + '\x1b[K').join('\n') + '\x1b[J' + SYNC_OFF);
    };
    const onKey = (_value, key = {}) => {
      if (key.ctrl && key.name === 'c') return cleanup(null);
      if (key.name === 'escape') return cleanup(null);
      if (key.name === 'return' || key.name === 'enter') {
        const results = resultsFor();
        return cleanup(results[cursor] ? results[cursor].item : null);
      }
      if (key.name === 'up' || key.name === 'k') cursor = Math.max(0, cursor - 1);
      else if (key.name === 'down' || key.name === 'j') cursor = cursor + 1;
      else if (key.name === 'backspace' || key.name === 'delete') query = query.slice(0, -1);
      else if (key.name === 'space' || (key.sequence && key.sequence.length === 1 && /[\w .\/+-]/.test(key.sequence))) {
        query += key.name === 'space' ? ' ' : key.sequence;
        cursor = 0;
        offset = 0;
      }
      draw();
    };
    process.stdout.write('\x1b[?1049h\x1b[?25l');
    readline.emitKeypressEvents(process.stdin);
    process.stdin.setRawMode(true);
    process.stdin.resume();
    process.stdin.on('keypress', onKey);
    process.stdout.on('resize', draw);
    timer = setInterval(() => { phase = (phase + 1) % 1000; }, 200);
    draw();
  });
}

const tone = {
  green: (v) => c.green + v + RESET,
  red: (v) => c.red + v + RESET,
  amber: (v) => c.amber + v + RESET,
  cyan: (v) => c.cyan + v + RESET,
  violet: (v) => c.violet + v + RESET,
  magenta: (v) => c.magenta + v + RESET,
  electric: (v) => c.cyan + BOLD + v + RESET,
  dim: (v) => DIM + v + RESET,
  bold: (v) => BOLD + v + RESET,
};

function hsl2rgb(h, s, l) {
  h = ((h % 360) + 360) % 360;
  const a = s * Math.min(l, 1 - l);
  const f = (n) => {
    const k = (n + h / 30) % 12;
    return Math.max(0, Math.min(255, Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))))));
  };
  return [f(0), f(8), f(4)];
}

function to256(r, g, b) {
  if (Math.abs(r - g) < 8 && Math.abs(g - b) < 8) return 232 + Math.round((r + g + b) / 30.05);
  return 16 + 36 * Math.round(r / 51) + 6 * Math.round(g / 51) + Math.round(b / 51);
}

const PAINTED = COLOR && colorDepth() >= 8 && !REDUCED;

class CellScreen {
  constructor(width, height) {
    this.width = width;
    this.height = height;
    this.ch = new Array(width * height).fill(' ');
    this.fg = new Array(width * height).fill('');
    this.bg = new Array(width * height).fill(null);
    this.prevSig = new Array(height).fill(null);
  }
  idx(x, y) { return y * this.width + x; }
  setCell(x, y, ch, fg, bg) {
    if (x < 0 || y < 0 || x >= this.width || y >= this.height) return;
    const i = this.idx(x, y);
    this.ch[i] = ch;
    if (fg !== undefined) this.fg[i] = fg;
    if (bg !== undefined) this.bg[i] = bg;
  }
  rowBg(y, x0, x1, rgbArr) {
    if (y < 0 || y >= this.height) return;
    for (let x = Math.max(0, x0); x < Math.min(this.width, x1); x++) this.bg[this.idx(x, y)] = rgbArr;
  }
  ansiLine(text, y, opts = {}) {
    if (y < 0 || y >= this.height) return;
    let x = opts.x || 0;
    let fg = '';
    const boldRe = /\x1b\[([0-9;]*)m/g;
    let rest = text;
    let match;
    let consumed = 0;
    boldRe.lastIndex = 0;
    let segment = '';
    const flush = () => {
      for (const chr of segment) {
        if (x >= this.width) return;
        const i = this.idx(x, y);
        this.ch[i] = chr;
        this.fg[i] = fg;
        if (opts.bg !== undefined) this.bg[i] = opts.bg;
        x++;
      }
      segment = '';
    };
    while ((match = boldRe.exec(text)) !== null) {
      segment = text.slice(consumed, match.index);
      flush();
      if (x >= this.width) break;
      consumed = match.index + match[0].length;
      const params = match[1];
      if (params === '0' || params === '') fg = opts.baseFg || '';
      else if (params === '1' || params === '2' || params.startsWith('38')) {
        if (params.startsWith('38')) fg = '\x1b[' + params + 'm';
        else fg = (fg && fg.includes(';5;') ? fg : '') + (params === '1' ? '\x1b[1m' : '\x1b[2m');
      } else if (params.includes(';')) {
        if (params.split(';').includes('38')) fg = '\x1b[' + params + 'm';
        else fg = '\x1b[' + params + 'm';
      } else fg = '\x1b[' + params + 'm';
    }
    segment = text.slice(consumed);
    flush();
  }
  serialize() {
    const out = [];
    for (let y = 0; y < this.height; y++) {
      let sig = '';
      const base = y * this.width;
      for (let x = 0; x < this.width; x++) sig += this.ch[base + x] + '|' + this.fg[base + x] + '|' + (this.bg[base + x] || '').join(',') + ';';
      if (sig === this.prevSig[y]) continue;
      this.prevSig[y] = sig;
      let line = `\x1b[${y + 1};1H`;
      let lastFg = null;
      let lastBg = null;
      for (let x = 0; x < this.width; x++) {
        const i = base + x;
        const bgKey = this.bg[i] ? this.bg[i].join(',') : '';
        const fgKey = this.fg[i];
        if (fgKey !== lastFg) {
          if (fgKey) line += fgKey;
          else if (lastFg) { line += '\x1b[0m'; lastBg = null; }
          lastFg = fgKey;
        }
        if (bgKey !== lastBg) {
          if (this.bg[i]) {
            const [r, g, b] = this.bg[i];
            line += TRUECOLOR ? `\x1b[48;2;${r};${g};${b}m` : `\x1b[48;5;${to256(r, g, b)}m`;
          } else line += '\x1b[49m';
          lastBg = bgKey;
        }
        line += this.ch[i];
      }
      if (lastFg || lastBg) line += '\x1b[0m';
      out.push(line);
    }
    return out.join('');
  }
}

const STAR_LAYERS = [
  { n: 26, speed: 0.05, glyphs: ['·'], lum: 0.5 },
  { n: 12, speed: 0.3, glyphs: ['˙', '⋆'], lum: 0.68 },
  { n: 6, speed: 1.0, glyphs: ['✦', '✧'], lum: 0.95 },
];

function makeStars(width, height) {
  const layers = [];
  for (const spec of STAR_LAYERS) {
    const stars = [];
    for (let i = 0; i < spec.n; i++) stars.push({ x: (i * 37.7 + 11) % width, y: (i * 53.3 + 7) % height, tw: i * 2.399963 });
    layers.push({ ...spec, stars });
  }
  return layers;
}

const BLOBS = [
  { x: 0.2, y: 0.28, r: 9, hue: 280, dx: 0.009 },
  { x: 0.66, y: 0.6, r: 13, hue: 200, dx: -0.006 },
  { x: 0.48, y: 0.86, r: 10, hue: 320, dx: 0.004 },
];

function paintBackdrop(screen, phase, opts = {}) {
  const { width, height } = screen;
  const baseHue = 250 + phase * 0.45;
  const horizon = Math.floor(height * 0.78);
  const blend = opts.panelBlend || 0;
  for (let y = 0; y < height; y++) {
    const f = Math.min(1, y / horizon);
    const [r, g, b] = hsl2rgb(baseHue + 42 * f, 0.62, 0.13 - 0.045 * f);
    screen.rowBg(y, 0, width, [r, g, b]);
  }
  for (const blob of BLOBS) {
    blob.x = (blob.x + blob.dx + 1) % 1;
    const bx = blob.x * width;
    const by = blob.y * height + 2 * Math.sin(phase * 0.09 + blob.hue);
    for (let y = Math.max(0, by - blob.r | 0); y < Math.min(height, by + blob.r); y++) {
      for (let x = Math.max(0, bx - 2 * blob.r | 0); x < Math.min(width, bx + 2 * blob.r); x++) {
        const d = Math.hypot((x - bx) / (2 * blob.r), (y - by) / blob.r);
        const v = Math.max(0, Math.cos(d * Math.PI / 2));
        if (v < 0.1) continue;
        const i = screen.idx(x, y);
        const [br, bg2, bb] = hsl2rgb(blob.hue + phase * 0.3, 0.75, 0.32);
        const cur = screen.bg[i] || [10, 8, 24];
        screen.bg[i] = [cur[0] + (br - cur[0]) * v * 0.5 | 0, cur[1] + (bg2 - cur[1]) * v * 0.5 | 0, cur[2] + (bb - cur[2]) * v * 0.5 | 0];
      }
    }
  }
  const stars = opts.stars || (opts.stars = makeStars(width, height));
  for (const layer of stars) {
    for (const star of layer.stars) {
      star.x -= layer.speed * 0.35;
      if (star.x < 0) star.x += width;
      const tw = 0.7 + 0.3 * Math.sin(phase * 0.35 + star.tw);
      const [r, g, b] = hsl2rgb(baseHue + 140, 0.5, layer.lum * tw);
      screen.setCell(star.x | 0, star.y | 0, layer.glyphs[(star.tw + (phase / 6 | 0)) % layer.glyphs.length | 0], TRUECOLOR ? `\x1b[38;2;${r};${g};${b}m` : `\x1b[38;5;${to256(r, g, b)}m`);
    }
  }
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < 4; x++) {
      for (const i of [screen.idx(x, y), screen.idx(width - 1 - x, y)]) {
        const bg = screen.bg[i];
        if (!bg) continue;
        const falloff = 1 - 0.4 * (x / 4);
        screen.bg[i] = [bg[0] * falloff | 0, bg[1] * falloff | 0, bg[2] * falloff | 0];
      }
    }
  }
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = screen.idx(x, y);
      const bg = screen.bg[i];
      if (!bg) continue;
      const lift = 1 + blend;
      screen.bg[i] = [Math.min(255, bg[0] * lift) | 0, Math.min(255, bg[1] * lift) | 0, Math.min(255, (bg[2] + 6 * blend) | 0)];
    }
  }
}

function tickerLine(facts, width, tick) {
  const joined = facts.join('  ·  ') + '  ·  ';
  const doubled = joined + joined;
  const offset = tick % joined.length;
  return '» ' + doubled.slice(offset, offset + Math.max(0, width - 3));
}

function paintedFrame(rows, opts) {
  const { width, height, phase, telemetry, tickerFacts } = opts;
  const screen = new CellScreen(width, height);
  if (!opts.stars) opts.stars = makeStars(width, height);
  paintBackdrop(screen, phase, { stars: opts.stars, panelBlend: 0.16 });
  rows.forEach((line, y) => {
    const isContent = line.trim().length > 0;
    if (isContent) {
      for (let x = 0; x < width; x++) {
        const i = screen.idx(x, y);
        const bg = screen.bg[i];
        if (bg) screen.bg[i] = [Math.min(255, bg[0] * 1.5 + 14) | 0, Math.min(255, bg[1] * 1.5 + 12) | 0, Math.min(255, bg[2] * 1.55 + 20) | 0];
      }
    }
    screen.ansiLine(line, y);
  });
  if (telemetry) {
    const top = `${telemetry.left}`.slice(0, Math.floor(width / 2) - 2);
    const topRight = `${telemetry.right}`.slice(0, Math.floor(width / 2) - 3);
    screen.ansiLine('', 0);
    screen.ansiLine(`${c.cyan}${top}${RESET}`, 0);
    const rightCol = width - 1 - topRight.length;
    for (let k = 0; k < topRight.length; k++) {
      const i = screen.idx(rightCol + k, 0);
      screen.ch[i] = topRight[k];
      screen.fg[i] = DIM;
    }
  }
  if (tickerFacts && tickerFacts.length) {
    const line = tickerLine(tickerFacts, width - 2, Math.floor(phase / 2));
    screen.ansiLine(`${DIM}${line}${RESET}`, height - 1);
  }
  const body = screen.serialize();
  process.stdout.write(SYNC_ON + body + SYNC_OFF + `\x1b[${height};1H`);
}

module.exports = {
  IS_TTY, REDUCED, COLOR, TRUECOLOR, PAINTED, SYNC_ON, SYNC_OFF,
  rgb, c, tone, RESET, BOLD, DIM, gradient, glowText, frame, meter, sparkline, coverageBars,
  fingerprint, randomart, hyperlink, clipboardPayload, copyCommand,
  ease, sleep, cascade, glitchResolve, countUp, typewriter, receiptCard, shareCard, promptShareCopy,
  score, fuzzySearch, hsl2rgb, to256, CellScreen, makeStars, paintBackdrop, tickerLine, paintedFrame,
};
