#!/usr/bin/env node

// Terminal frames (arrays of ANSI-coloured lines) to PNG and GIF bytes, with Node built-ins only.

const zlib = require('zlib');
const { FONT, GLYPH_TOP, CELL_WIDTH, CELL_HEIGHT } = require('./vestige-font.js');

const CARD = { cols: 72, rows: 17, scale: 3, padX: 24, padY: 46 };
const STAGE = { cols: 72, rows: 22, scale: 2, padX: 18, padY: 6 };
const DEFAULT_FG = [214, 220, 235];
const DEFAULT_BG = [12, 12, 28];
const BASIC = [[0, 0, 0], [205, 49, 49], [13, 188, 121], [229, 229, 16], [36, 114, 200], [188, 63, 188], [17, 168, 205], [229, 229, 229],
  [102, 102, 102], [241, 76, 76], [35, 209, 139], [245, 245, 67], [59, 142, 234], [214, 112, 214], [41, 184, 219], [255, 255, 255]];

function color256(n) {
  if (n < 16) return BASIC[n];
  if (n < 232) { const k = n - 16; const level = (v) => (v ? 55 + v * 40 : 0); return [level(Math.floor(k / 36)), level(Math.floor(k / 6) % 6), level(k % 6)]; }
  const gray = 8 + (n - 232) * 10;
  return [gray, gray, gray];
}

function parseAnsi(lines, { cols, rows } = {}) {
  const height = rows || lines.length;
  const parsed = [];
  for (let y = 0; y < height; y++) {
    const text = String(lines[y] || '').replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '');
    const row = [];
    let style = { fg: null, bg: null, bold: false, dim: false, reverse: false };
    const pattern = /\x1b\[([0-9;?]*)([@-~])|([^\x1b])/gu;
    let match;
    while ((match = pattern.exec(text)) !== null) {
      if (match[3] !== undefined) {
        const [fg, bg] = style.reverse ? [style.bg || DEFAULT_BG, style.fg || DEFAULT_FG] : [style.fg, style.bg];
        row.push({ ch: match[3], fg, bg, bold: style.bold, dim: style.dim });
        continue;
      }
      if (match[2] !== 'm') continue;
      const params = match[1] === '' ? [0] : match[1].split(';').map((p) => Number(p) || 0);
      for (let i = 0; i < params.length; i++) {
        const p = params[i];
        if (p === 0) style = { fg: null, bg: null, bold: false, dim: false, reverse: false };
        else if (p === 1) style.bold = true;
        else if (p === 2) style.dim = true;
        else if (p === 22) { style.bold = false; style.dim = false; } else if (p === 7) style.reverse = true;
        else if (p === 27) style.reverse = false;
        else if (p === 39) style.fg = null;
        else if (p === 49) style.bg = null;
        else if (p >= 30 && p <= 37) style.fg = BASIC[p - 30];
        else if (p >= 90 && p <= 97) style.fg = BASIC[p - 82];
        else if (p >= 40 && p <= 47) style.bg = BASIC[p - 40];
        else if (p >= 100 && p <= 107) style.bg = BASIC[p - 92];
        else if (p === 38 || p === 48) {
          let rgb = null;
          if (params[i + 1] === 5) { rgb = color256(Math.max(0, Math.min(255, params[i + 2] || 0))); i += 2; } else if (params[i + 1] === 2) { rgb = [params[i + 2], params[i + 3], params[i + 4]].map((v) => Math.max(0, Math.min(255, v || 0))); i += 4; }
          if (rgb) style[p === 38 ? 'fg' : 'bg'] = rgb;
        }
      }
    }
    parsed.push(row);
  }
  const width = cols || Math.max(1, ...parsed.map((row) => row.length));
  const cells = [];
  for (const row of parsed) for (let x = 0; x < width; x++) cells.push(row[x] || { ch: ' ', fg: null, bg: null, bold: false, dim: false });
  return { cols: width, rows: height, cells };
}

// Box drawing as four arms from the cell centre: [up, right, down, left], 1 light, 2 heavy.
const BOX = {
  '─': [0, 1, 0, 1], '│': [1, 0, 1, 0], '┌': [0, 1, 1, 0], '┐': [0, 0, 1, 1], '└': [1, 1, 0, 0], '┘': [1, 0, 0, 1],
  '├': [1, 1, 1, 0], '┤': [1, 0, 1, 1], '┬': [0, 1, 1, 1], '┴': [1, 1, 0, 1], '┼': [1, 1, 1, 1],
  '╭': [0, 1, 1, 0], '╮': [0, 0, 1, 1], '╰': [1, 1, 0, 0], '╯': [1, 0, 0, 1],
  '━': [0, 2, 0, 2], '┃': [2, 0, 2, 0], '┏': [0, 2, 2, 0], '┓': [0, 0, 2, 2], '┗': [2, 2, 0, 0], '┛': [2, 0, 0, 2],
  '═': [0, 2, 0, 2], '║': [2, 0, 2, 0], '╔': [0, 2, 2, 0], '╗': [0, 0, 2, 2], '╚': [2, 2, 0, 0], '╝': [2, 0, 0, 2],
};
const DASHED = { '┄': 'h', '╌': 'h', '┈': 'h', '┆': 'v', '╎': 'v', '┊': 'v' };
const QUADRANTS = { '▖': 4, '▗': 8, '▘': 1, '▙': 13, '▚': 9, '▛': 7, '▜': 11, '▝': 2, '▞': 6, '▟': 14 };

function glyphKnown(ch) {
  const code = ch.codePointAt(0);
  return ch === ' ' || FONT.has(ch) || BOX[ch] || DASHED[ch] || QUADRANTS[ch] || (code >= 0x2580 && code <= 0x2595) || (code >= 0x2800 && code <= 0x28ff);
}

function missingGlyphs(grid) {
  return [...new Set(grid.cells.map((cell) => cell.ch).filter((ch) => !glyphKnown(ch)))];
}

function rasterize(grid, { scale = 3, padX = 24, padY = 24, background = DEFAULT_BG, defaultFg = DEFAULT_FG } = {}) {
  const cw = CELL_WIDTH * scale;
  const ch = CELL_HEIGHT * scale;
  const width = grid.cols * cw + padX * 2;
  const height = grid.rows * ch + padY * 2;
  const rgb = new Uint8Array(width * height * 3);
  const set = (x, y, c) => { if (x >= 0 && y >= 0 && x < width && y < height) { const i = (y * width + x) * 3; rgb[i] = c[0]; rgb[i + 1] = c[1]; rgb[i + 2] = c[2]; } };
  const rect = (x0, y0, x1, y1, c) => { for (let y = Math.max(0, y0); y < Math.min(height, y1); y++) for (let x = Math.max(0, x0); x < Math.min(width, x1); x++) { const i = (y * width + x) * 3; rgb[i] = c[0]; rgb[i + 1] = c[1]; rgb[i + 2] = c[2]; } };
  const get = (x, y) => { const i = (y * width + x) * 3; return [rgb[i], rgb[i + 1], rgb[i + 2]]; };

  const flat = Array.isArray(background);
  for (let y = 0; y < height; y++) {
    const base = flat ? background : [0, 1, 2].map((k) => background.top[k] + (background.bottom[k] - background.top[k]) * (y / Math.max(1, height - 1)));
    for (let x = 0; x < width; x++) {
      let shade = 1;
      if (!flat && background.vignette) {
        const dx = (x / width - 0.5) * 2;
        const dy = (y / height - 0.5) * 2;
        shade = 1 - background.vignette * Math.min(1, (dx * dx + dy * dy) / 2);
      }
      set(x, y, [base[0] * shade, base[1] * shade, base[2] * shade]);
    }
  }
  const blank = (cx, cy) => { if (cx < 0 || cy < 0 || cx >= grid.cols || cy >= grid.rows) return true; const cell = grid.cells[cy * grid.cols + cx]; return cell.ch === ' ' && !cell.bg; };
  if (!flat && background.stars) {
    let seed = background.stars >>> 0;
    const next = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 4294967296; };
    for (let i = 0; i < Math.round((width * height) / 9000); i++) {
      const x = Math.floor(next() * width);
      const y = Math.floor(next() * height);
      const glow = 70 + Math.floor(next() * 110);
      const cx = Math.floor((x - padX) / cw);
      const cy = Math.floor((y - padY) / ch);
      let open = true;
      for (let dy = -1; dy <= 1 && open; dy++) for (let dx = -1; dx <= 1; dx++) if (!blank(cx + dx, cy + dy)) { open = false; break; }
      if (open) rect(x, y, x + scale, y + scale, [glow, glow + 8, Math.min(255, glow + 60)]);
    }
  }

  for (let cy = 0; cy < grid.rows; cy++) {
    for (let cx = 0; cx < grid.cols; cx++) {
      const cell = grid.cells[cy * grid.cols + cx];
      const x0 = padX + cx * cw;
      const y0 = padY + cy * ch;
      if (cell.bg) rect(x0, y0, x0 + cw, y0 + ch, cell.bg);
      if (cell.ch === ' ') continue;
      const under = cell.bg || get(x0, y0);
      let ink = cell.fg || defaultFg;
      if (cell.bold) ink = ink.map((v) => Math.round(v + (255 - v) * 0.2));
      if (cell.dim) ink = ink.map((v, k) => Math.round(v * 0.5 + under[k] * 0.5));
      const part = (fx0, fy0, fx1, fy1) => rect(x0 + Math.round(fx0 * cw), y0 + Math.round(fy0 * ch), x0 + Math.round(fx1 * cw), y0 + Math.round(fy1 * ch), ink);
      const code = cell.ch.codePointAt(0);
      const glyph = FONT.get(cell.ch);
      if (glyph) {
        glyph.forEach((row, gy) => row.forEach((on, gx) => { if (on) rect(x0 + gx * scale, y0 + (GLYPH_TOP + gy) * scale, x0 + (gx + 1) * scale, y0 + (GLYPH_TOP + gy + 1) * scale, ink); }));
      } else if (BOX[cell.ch]) {
        const [up, right, down, left] = BOX[cell.ch];
        const mx = x0 + Math.floor(cw / 2);
        const my = y0 + Math.floor(ch / 2);
        const half = (weight) => Math.max(1, Math.round((scale * weight) / 2));
        const reach = half(Math.max(up, right, down, left));
        if (up) rect(mx - half(up), y0, mx + half(up), my + reach, ink);
        if (down) rect(mx - half(down), my - reach, mx + half(down), y0 + ch, ink);
        if (left) rect(x0, my - half(left), mx + reach, my + half(left), ink);
        if (right) rect(mx - reach, my - half(right), x0 + cw, my + half(right), ink);
      } else if (DASHED[cell.ch]) {
        const t = Math.max(1, Math.round(scale / 2));
        const mx = x0 + Math.floor(cw / 2);
        const my = y0 + Math.floor(ch / 2);
        if (DASHED[cell.ch] === 'h') for (const f of [0.08, 0.58]) rect(x0 + Math.round(f * cw), my - t, x0 + Math.round((f + 0.34) * cw), my + t, ink);
        else for (const f of [0.06, 0.4, 0.74]) rect(mx - t, y0 + Math.round(f * ch), mx + t, y0 + Math.round((f + 0.2) * ch), ink);
      } else if (QUADRANTS[cell.ch]) {
        const bits = QUADRANTS[cell.ch];
        if (bits & 1) part(0, 0, 0.5, 0.5);
        if (bits & 2) part(0.5, 0, 1, 0.5);
        if (bits & 4) part(0, 0.5, 0.5, 1);
        if (bits & 8) part(0.5, 0.5, 1, 1);
      } else if (code === 0x2580) part(0, 0, 1, 0.5);
      else if (code >= 0x2581 && code <= 0x2588) part(0, 1 - (code - 0x2580) / 8, 1, 1);
      else if (code >= 0x2589 && code <= 0x258f) part(0, 0, (0x2590 - code) / 8, 1);
      else if (code === 0x2590) part(0.5, 0, 1, 1);
      else if (code >= 0x2591 && code <= 0x2593) {
        const density = code - 0x2590;
        for (let py = 0; py < CELL_HEIGHT; py++) for (let px = 0; px < CELL_WIDTH; px++) {
          const on = density === 2 ? (px + py) % 2 === 0 : density === 1 ? px % 2 === 0 && py % 2 === (px / 2) % 2 : !(px % 2 === 0 && py % 2 === (px / 2) % 2);
          if (on) rect(x0 + px * scale, y0 + py * scale, x0 + (px + 1) * scale, y0 + (py + 1) * scale, ink);
        }
      } else if (code === 0x2594) part(0, 0, 1, 1 / 8);
      else if (code === 0x2595) part(7 / 8, 0, 1, 1);
      else if (code >= 0x2800 && code <= 0x28ff) {
        const dots = [[0, 0, 1], [0, 1, 2], [0, 2, 4], [1, 0, 8], [1, 1, 16], [1, 2, 32], [0, 3, 64], [1, 3, 128]];
        for (const [col, row, bit] of dots) if ((code - 0x2800) & bit) rect(x0 + (1 + col * 3) * scale, y0 + (1 + row * 3) * scale, x0 + (2 + col * 3) * scale, y0 + (2 + row * 3) * scale, ink);
      } else {
        rect(x0 + scale, y0 + 3 * scale, x0 + 5 * scale, y0 + 4 * scale, ink);
        rect(x0 + scale, y0 + 8 * scale, x0 + 5 * scale, y0 + 9 * scale, ink);
        rect(x0 + scale, y0 + 3 * scale, x0 + 2 * scale, y0 + 9 * scale, ink);
        rect(x0 + 4 * scale, y0 + 3 * scale, x0 + 5 * scale, y0 + 9 * scale, ink);
      }
    }
  }
  return { width, height, rgb };
}

const CRC_TABLE = Array.from({ length: 256 }, (_v, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) crc = CRC_TABLE[(crc ^ byte) & 255] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function encodePNG({ width, height, rgb }) {
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc32(body), body.length + 4);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  const raw = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++) raw.set(rgb.subarray(y * width * 3, (y + 1) * width * 3), y * (width * 3 + 1) + 1);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header), chunk('IDAT', zlib.deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

function lzw(minCodeSize, indices) {
  const clear = 1 << minCodeSize;
  const end = clear + 1;
  const out = [];
  let size = minCodeSize + 1;
  let next = end + 1;
  let table = new Map();
  let hold = 0;
  let bits = 0;
  const emit = (code) => {
    hold |= code << bits;
    bits += size;
    while (bits >= 8) { out.push(hold & 255); hold >>>= 8; bits -= 8; }
  };
  emit(clear);
  let prefix = indices[0];
  for (let i = 1; i < indices.length; i++) {
    const k = indices[i];
    const key = prefix * 256 + k;
    const found = table.get(key);
    if (found !== undefined) { prefix = found; continue; }
    emit(prefix);
    if (next === 4096) {
      emit(clear);
      table = new Map();
      next = end + 1;
      size = minCodeSize + 1;
    } else {
      if (next >= 1 << size) size++;
      table.set(key, next++);
    }
    prefix = k;
  }
  emit(prefix);
  emit(end);
  if (bits > 0) out.push(hold & 255);
  return out;
}

function encodeGIF(frames, { loop = 0 } = {}) {
  const { width, height } = frames[0];
  const counts = new Map();
  for (const frame of frames) for (let i = 0; i < frame.rgb.length; i += 3) { const key = (frame.rgb[i] << 16) | (frame.rgb[i + 1] << 8) | frame.rgb[i + 2]; counts.set(key, (counts.get(key) || 0) + 1); }
  const palette = [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 256).map(([key]) => key);
  const index = new Map(palette.map((key, i) => [key, i]));
  const nearest = (key) => {
    let best = 0;
    let bestDistance = Infinity;
    for (let i = 0; i < palette.length; i++) {
      const dr = ((palette[i] >> 16) & 255) - ((key >> 16) & 255);
      const dg = ((palette[i] >> 8) & 255) - ((key >> 8) & 255);
      const db = (palette[i] & 255) - (key & 255);
      const distance = dr * dr + dg * dg + db * db;
      if (distance < bestDistance) { bestDistance = distance; best = i; }
    }
    index.set(key, best);
    return best;
  };
  let depth = 1;
  while (1 << depth < palette.length) depth++;
  const minCode = Math.max(2, depth);
  const out = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61, width & 255, width >> 8, height & 255, height >> 8, 0xf0 | (depth - 1), 0, 0];
  for (let i = 0; i < 1 << depth; i++) { const key = palette[i] || 0; out.push((key >> 16) & 255, (key >> 8) & 255, key & 255); }
  out.push(0x21, 0xff, 11, ...Buffer.from('NETSCAPE2.0', 'ascii'), 3, 1, loop & 255, loop >> 8, 0);

  let previous = null;
  for (const frame of frames) {
    const pixels = new Uint8Array(width * height);
    for (let p = 0, i = 0; p < pixels.length; p++, i += 3) {
      const key = (frame.rgb[i] << 16) | (frame.rgb[i + 1] << 8) | frame.rgb[i + 2];
      const at = index.get(key);
      pixels[p] = at === undefined ? nearest(key) : at;
    }
    let x0 = 0, y0 = 0, x1 = width - 1, y1 = height - 1;
    if (previous) {
      x0 = width; y0 = height; x1 = -1; y1 = -1;
      for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) if (pixels[y * width + x] !== previous[y * width + x]) { if (x < x0) x0 = x; if (x > x1) x1 = x; if (y < y0) y0 = y; if (y > y1) y1 = y; }
      if (x1 < 0) { x0 = 0; y0 = 0; x1 = 0; y1 = 0; }
    }
    const w = x1 - x0 + 1;
    const h = y1 - y0 + 1;
    const region = new Uint8Array(w * h);
    for (let y = 0; y < h; y++) region.set(pixels.subarray((y0 + y) * width + x0, (y0 + y) * width + x0 + w), y * w);
    const delay = Math.max(2, Math.round((frame.delayMs || 80) / 10));
    out.push(0x21, 0xf9, 4, 0x04, delay & 255, delay >> 8, 0, 0);
    out.push(0x2c, x0 & 255, x0 >> 8, y0 & 255, y0 >> 8, w & 255, w >> 8, h & 255, h >> 8, 0, minCode);
    const data = lzw(minCode, region);
    for (let i = 0; i < data.length; i += 255) { const block = data.slice(i, i + 255); out.push(block.length, ...block); }
    out.push(0);
    previous = pixels;
  }
  out.push(0x3b);
  return Buffer.from(out);
}

function linesToPNG(lines, options = {}) {
  return encodePNG(rasterize(parseAnsi(lines, options), options));
}

function framesToGIF(frames, options = {}) {
  return encodeGIF(frames.map((frame) => ({ ...rasterize(parseAnsi(frame.lines, options), options), delayMs: frame.ms })), options);
}

if (require.main === module && process.argv.includes('--specimen')) {
  const fs = require('fs');
  const path = require('path');
  const out = path.join(__dirname, 'probes', 'out', 'raster');
  fs.mkdirSync(out, { recursive: true });
  const c = (r, g, b) => `\x1b[38;2;${r};${g};${b}m`;
  const lines = [
    ' !"#$%&\'()*+,-./0123456789:;<=>?@',
    'ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`',
    'abcdefghijklmnopqrstuvwxyz{|}~',
    'The quick brown fox jumps over the lazy dog. 0123456789',
    'a remembered yes is not permission today.  npx @vestige/init',
    '\x1b[1mbold text sample\x1b[0m  \x1b[2mdim text sample\x1b[0m  \x1b[7m reverse \x1b[0m',
    `${c(90, 120, 255)}indigo ${c(190, 80, 255)}violet ${c(255, 100, 90)}red ${c(100, 230, 140)}green ${c(255, 190, 90)}amber ${c(120, 220, 255)}cyan\x1b[0m`,
    '· • … ← → ↑ ↓ × ✗ ✔ ◉ ● ○ ◌ ⚡ ✦ ✧ ▸ ◂ › » « ⋆',
    '╭──┬──╮ ┌──┐ ┏━━┓ ╔══╗ ┄┄┄┄ ▁▂▃▄▅▆▇█ ▏▎▍▌▋▊▉ ░▒▓ ▖▗▘▙▚▛▜▝▞▟ ⠁⠃⠇⡇⣇⣧⣷⣿',
    '│  │  │ │  │ ┃  ┃ ║  ║ ┆  ┆ ▀▀▀▀▄▄▄▄ ▐▌',
    '├──┼──┤ └──┘ ┗━━┛ ╚══╝',
    '╰──┴──╯',
    '      ▄▄▄▄▄▄',
    '     ████████',
    `    ██${c(120, 220, 255)} ◉  ◉ \x1b[0m██   ${c(255, 100, 90)}██ ×  × ██\x1b[0m`,
    `    ██${c(120, 220, 255)}  ╰─╯ \x1b[0m██   ${c(255, 100, 90)}██  ╰─╯ ██\x1b[0m`,
    '     ████████',
  ];
  const grid = parseAnsi(lines, { cols: 72 });
  fs.writeFileSync(path.join(out, 'specimen.png'), encodePNG(rasterize(grid, { background: { top: [24, 26, 62], bottom: [9, 9, 24], vignette: 0.35, stars: 7 } })));
  console.log('missing glyphs:', JSON.stringify(missingGlyphs(grid)));
  console.log(path.join(out, 'specimen.png'));
}

module.exports = { CARD, STAGE, parseAnsi, rasterize, missingGlyphs, encodePNG, encodeGIF, linesToPNG, framesToGIF, crc32, lzw, color256 };
