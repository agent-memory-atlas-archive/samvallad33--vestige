const test = require('node:test');
const assert = require('node:assert/strict');
const zlib = require('zlib');
const raster = require('../lib/vestige-raster.js');
const booking = require('../lib/vestige-booking.js');

function pngChunks(png) {
  assert.deepEqual([...png.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  const chunks = [];
  for (let at = 8; at < png.length;) {
    const length = png.readUInt32BE(at);
    const body = png.subarray(at + 4, at + 8 + length);
    assert.equal(png.readUInt32BE(at + 8 + length), raster.crc32(body), 'chunk checksum');
    chunks.push({ type: body.subarray(0, 4).toString('ascii'), data: body.subarray(4) });
    at += 12 + length;
  }
  return chunks;
}

// A plain GIF LZW decoder, written separately from the encoder, to prove the stream round trips.
function unlzw(minCode, bytes, count) {
  const clear = 1 << minCode;
  const end = clear + 1;
  let size = minCode + 1;
  let table = [];
  const reset = () => { table = Array.from({ length: clear + 2 }, (_v, i) => (i < clear ? [i] : [])); size = minCode + 1; };
  reset();
  const out = [];
  let hold = 0, bits = 0, at = 0, previous = null;
  while (out.length < count) {
    while (bits < size) { hold |= bytes[at++] << bits; bits += 8; }
    const code = hold & ((1 << size) - 1);
    hold >>>= size;
    bits -= size;
    if (code === clear) { reset(); previous = null; continue; }
    if (code === end) break;
    let entry;
    if (code < table.length) entry = table[code];
    else entry = previous.concat(previous[0]);
    out.push(...entry);
    if (previous) { table.push(previous.concat(entry[0])); if (table.length === 1 << size && size < 12) size++; }
    previous = entry;
  }
  return out;
}

function gifFrames(gif) {
  assert.equal(gif.subarray(0, 6).toString('ascii'), 'GIF89a');
  const width = gif.readUInt16LE(6);
  const height = gif.readUInt16LE(8);
  const tableSize = 1 << ((gif[10] & 7) + 1);
  let at = 13 + tableSize * 3;
  const frames = [];
  let loops = null;
  let delay = 0;
  while (gif[at] !== 0x3b) {
    if (gif[at] === 0x21 && gif[at + 1] === 0xff) { loops = gif.readUInt16LE(at + 16); at += 19; } else if (gif[at] === 0x21 && gif[at + 1] === 0xf9) { delay = gif.readUInt16LE(at + 4); at += 8; } else if (gif[at] === 0x2c) {
      const frame = { x: gif.readUInt16LE(at + 1), y: gif.readUInt16LE(at + 3), w: gif.readUInt16LE(at + 5), h: gif.readUInt16LE(at + 7), delay, minCode: gif[at + 10], data: [] };
      at += 11;
      while (gif[at]) { frame.data.push(...gif.subarray(at + 1, at + 1 + gif[at])); at += 1 + gif[at]; }
      at++;
      frames.push(frame);
    } else assert.fail('unexpected block ' + gif[at]);
  }
  return { width, height, tableSize, loops, frames, palette: gif.subarray(13, 13 + tableSize * 3) };
}

test('parseAnsi reads the colour forms the bundle uses', () => {
  const grid = raster.parseAnsi(['\x1b[1;38;2;10;20;30mA\x1b[0mB\x1b[38;5;196mC\x1b[48;5;21mD\x1b[0;2mE\x1b[0;31;44mF\x1b[0;7mG\x1b[27mH\x1b]8;;http://x\x07L\x1b]8;;\x07\x1b[5;9HZ\x1b[91mI'], { cols: 14 });
  const [a, b, c, d, e, f, g, h, l, z, i, pad] = grid.cells;
  assert.deepEqual([a.ch, a.fg, a.bold], ['A', [10, 20, 30], true]);
  assert.deepEqual([b.fg, b.bg, b.bold], [null, null, false]);
  assert.deepEqual(c.fg, [255, 0, 0]);
  assert.deepEqual(d.bg, [0, 0, 255]);
  assert.equal(e.dim, true);
  assert.deepEqual([f.fg, f.bg], [[205, 49, 49], [36, 114, 200]]);
  assert.ok(g.bg && g.fg, 'reverse swaps in the defaults');
  assert.deepEqual([h.fg, h.bg], [null, null]);
  assert.equal(l.ch + z.ch, 'LZ', 'link text is kept, other sequences are ignored');
  assert.deepEqual(i.fg, [241, 76, 76]);
  assert.equal(pad.ch, ' ');
  assert.equal(raster.parseAnsi(['abcdef'], { cols: 3 }).cells.length, 3);
});

test('PNG is well formed and holds the pixels', () => {
  const image = raster.rasterize(raster.parseAnsi(['\x1b[48;2;200;10;10m  \x1b[0m █'], { cols: 4 }), { scale: 2, padX: 3, padY: 5, background: [1, 2, 3] });
  assert.equal(image.width, 4 * 12 + 6);
  assert.equal(image.height, 24 + 10);
  const chunks = pngChunks(raster.encodePNG(image));
  assert.deepEqual(chunks.map((c) => c.type), ['IHDR', 'IDAT', 'IEND']);
  assert.equal(chunks[0].data.readUInt32BE(0), image.width);
  assert.equal(chunks[0].data.readUInt32BE(4), image.height);
  assert.deepEqual([...chunks[0].data.subarray(8)], [8, 2, 0, 0, 0]);
  const raw = zlib.inflateSync(chunks[1].data);
  assert.equal(raw.length, (image.width * 3 + 1) * image.height);
  const pixel = (x, y) => [...raw.subarray(y * (image.width * 3 + 1) + 1 + x * 3, y * (image.width * 3 + 1) + 4 + x * 3)];
  assert.deepEqual(pixel(0, 0), [1, 2, 3], 'padding is background');
  assert.deepEqual(pixel(3, 5), [200, 10, 10], 'cell background');
  assert.deepEqual(pixel(3 + 3 * 12 + 6, 5 + 12), [214, 220, 235], 'full block in the default ink');
});

test('GIF is well formed and its LZW stream round trips', () => {
  const make = (lines) => ({ ...raster.rasterize(raster.parseAnsi(lines, { cols: 6, rows: 2 }), { scale: 1, padX: 2, padY: 2 }), delayMs: 120 });
  const frames = [make(['\x1b[31m█▀▄\x1b[0m ab', 'xyz']), make(['\x1b[31m█▀▄\x1b[0m ab', 'xyQ']), make(['\x1b[31m█▀▄\x1b[0m ab', 'xyQ'])];
  const gif = gifFrames(raster.encodeGIF(frames));
  assert.equal(gif.width, frames[0].width);
  assert.equal(gif.height, frames[0].height);
  assert.equal(gif.loops, 0);
  assert.equal(gif.frames.length, 3);
  assert.equal(gif.frames[0].delay, 12);
  const first = gif.frames[0];
  assert.deepEqual([first.x, first.y, first.w, first.h], [0, 0, gif.width, gif.height]);
  const indices = unlzw(first.minCode, first.data, first.w * first.h);
  assert.equal(indices.length, first.w * first.h);
  for (const p of [0, 7, 100, indices.length - 1]) {
    assert.deepEqual([...gif.palette.subarray(indices[p] * 3, indices[p] * 3 + 3)], [...frames[0].rgb.subarray(p * 3, p * 3 + 3)]);
  }
  const second = gif.frames[1];
  assert.ok(second.w < gif.width && second.h < gif.height, 'only the changed rectangle is stored');
  assert.equal(unlzw(second.minCode, second.data, second.w * second.h).length, second.w * second.h);
  assert.deepEqual([gif.frames[2].w, gif.frames[2].h], [1, 1], 'an unchanged frame keeps its delay with a 1x1 patch');
});

test('a long LZW stream with table resets round trips', () => {
  let seed = 42;
  const next = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed >>> 24; };
  const indices = Array.from({ length: 60000 }, (_v, i) => (i % 97 < 60 ? next() % 16 : (i >> 5) % 16));
  assert.deepEqual(unlzw(4, raster.lzw(4, indices), indices.length), indices);
});

test('the card and the episode export at their preset sizes with every glyph known', () => {
  const card = booking.bookingCard(booking.FIXTURES.b, { color: 'truecolor' });
  assert.deepEqual(raster.missingGlyphs(raster.parseAnsi(card, raster.CARD)), []);
  const png = raster.linesToPNG(card, { ...raster.CARD, background: { top: [24, 26, 62], bottom: [9, 9, 24], vignette: 0.35, stars: 7 } });
  const header = pngChunks(png)[0].data;
  assert.deepEqual([header.readUInt32BE(0), header.readUInt32BE(4)], [1344, 704]);
  const frames = booking.storyFrames(booking.FIXTURES.b);
  for (const frame of frames) assert.deepEqual(raster.missingGlyphs(raster.parseAnsi(frame.lines, raster.STAGE)), []);
  const t0 = Date.now();
  raster.rasterize(raster.parseAnsi(frames[30].lines, raster.STAGE), raster.STAGE);
  const oneFrame = Date.now() - t0;
  const t1 = Date.now();
  const gif = raster.framesToGIF(frames, raster.STAGE);
  const encode = Date.now() - t1;
  const parsed = gifFrames(gif);
  assert.deepEqual([parsed.width, parsed.height, parsed.frames.length], [900, 540, 50]);
  console.log(`card png ${png.length} bytes · one frame rasterized in ${oneFrame} ms · 50-frame gif ${gif.length} bytes in ${encode} ms`);
  assert.ok(gif.length < 2 * 1024 * 1024, 'gif under 2 MB');
  assert.ok(encode < 8000, 'gif encodes in under 8 s');
});
