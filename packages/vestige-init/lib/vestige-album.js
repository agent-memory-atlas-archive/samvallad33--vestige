#!/usr/bin/env node

const { DISGUISE_LABELS } = require('./vestige-story.js');
const { Grid, FIXTURES } = require('./vestige-booking.js');

const BRAND_A = [90, 120, 255];
const BRAND_B = [190, 80, 255];
const AMBER = [255, 190, 90];
const TILE = 22;
const GAP = 2;
const MAX_TILES = 24;

const fit = (text, width) => { const list = [...String(text)]; return list.length <= width ? list.join('') : list.slice(0, width - 1).join('') + '…'; };

// Rarity is how often this kind of stop has happened on this machine. Nothing here is rolled.
function rarity(story, stats) {
  const count = stats.byCommitment[story.commitment] || 1;
  if (stats.firstTs[story.commitment] === story.ts) return { tier: 'first', label: `first ${story.commitment} here` };
  return { tier: count / Math.max(1, stats.total) < 0.05 ? 'rare' : 'common', label: `${count} of ${stats.total} here` };
}

function shortDate(ts) {
  const when = new Date(ts);
  return `${when.getDate()} ${['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'][when.getMonth()]}`;
}

function drawTile(grid, x, y, { lines, ghost, tier }) {
  const flat = tier === 'rare' ? { fg: AMBER } : tier === 'canon' ? { fg: BRAND_A } : { dim: true };
  const edge = (cx, cy, ch, t) => grid.put(cx, cy, ch, tier === 'first' ? { fg: [0, 1, 2].map((k) => Math.round(BRAND_A[k] + (BRAND_B[k] - BRAND_A[k]) * t)) } : flat);
  const [h, v] = ghost ? ['┄', '┆'] : ['─', '│'];
  for (let i = 0; i < TILE; i++) {
    const t = i / (TILE - 1);
    edge(x + i, y, i === 0 ? '┌' : i === TILE - 1 ? '┐' : h, t);
    edge(x + i, y + 5, i === 0 ? '└' : i === TILE - 1 ? '┘' : h, t);
  }
  for (let row = 1; row < 5; row++) { edge(x, y + row, v, 0); edge(x + TILE - 1, y + row, v, 1); }
  lines.forEach(([text, style], i) => grid.put(x + 2, y + 1 + i, fit(text, TILE - 4), style || {}));
}

function albumView({ stories = [], canon = [], seen = new Set(), stats = { total: 0, byCommitment: {}, firstTs: {} }, cols = 100, color = 'truecolor' } = {}) {
  const unseen = stories.filter((s) => !seen.has(s.digest)).length;
  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
  const header = [`${plural(stories.length, 'true story', 'true stories')} · ${plural(canon.length, 'canon episode', 'canon episodes')} · ${unseen} not yet seen`];
  if (!stories.length) header.push('true stories appear here after Operator stops something one of your agents tried');
  const tiles = [];
  for (const story of stories.slice(0, MAX_TILES)) {
    if (!seen.has(story.digest)) { tiles.push({ ghost: true, tier: 'ghost', lines: [['not yet seen', { dim: true }], [shortDate(story.ts), { dim: true }]] }); continue; }
    const rare = rarity(story, stats);
    tiles.push({
      tier: rare.tier,
      lines: [
        [story.commitment, { bold: true }],
        [`${story.source} · ${shortDate(story.ts)}`, {}],
        [rare.label, rare.tier === 'common' ? { dim: true } : { fg: AMBER }],
        [story.disguises.length ? story.disguises.map((name) => DISGUISE_LABELS[name]).join(' ') : story.digest.slice(0, 8), { dim: true }],
      ],
    });
  }
  for (const episode of canon) {
    tiles.push({ tier: 'canon', lines: [[`EPISODE ${episode.number}`, { bold: true }], [episode.title, {}], [episode.rule, { dim: true }], [episode.version, { dim: true }]] });
  }
  const perRow = Math.max(1, Math.floor((Math.max(TILE, cols) + GAP) / (TILE + GAP)));
  const rows = Math.ceil(tiles.length / perRow);
  const grid = new Grid(Math.min(Math.max(TILE, cols), perRow * (TILE + GAP) - GAP), Math.max(1, rows * 7 - 1));
  tiles.forEach((tile, i) => drawTile(grid, (i % perRow) * (TILE + GAP), Math.floor(i / perRow) * 7, tile));
  const more = stories.length > MAX_TILES ? [`… and ${stories.length - MAX_TILES} older true stories · --show <number> plays one`] : [];
  return [...header.map((line) => fit(line, cols)), '', ...(tiles.length ? grid.lines(color) : []), ...more.map((line) => fit(line, cols))];
}

if (require.main === module) {
  const now = Date.parse('2026-10-12T03:52:00Z');
  const make = (i, commitment, extra = {}) => ({ ...FIXTURES.b, commitment, digest: (i + 1).toString(16).padStart(2, '0').repeat(8), ts: now - i * 7200000, ...extra });
  const stories = [make(0, 'OP-001'), make(1, 'recall-before-grep', { disguises: [] }), make(2, 'OP-001'), make(3, 'OP-018', { disguises: ['heredoc-extracted'] }), make(4, 'OP-001'), make(5, 'OP-001')];
  const stats = { total: 6, byCommitment: { 'OP-001': 4, 'recall-before-grep': 1, 'OP-018': 1 }, firstTs: { 'OP-001': stories[5].ts, 'recall-before-grep': stories[1].ts, 'OP-018': stories[3].ts } };
  const canon = [{ number: 1, version: 'v4.2.0', title: 'the deletion', rule: 'OP-001' }, { number: 2, version: 'v4.2.0', title: 'the hidden hand', rule: 'OP-001' }, { number: 3, version: 'v4.2.0', title: 'the ledger', rule: 'OP-000' }];
  const seen = new Set(stories.slice(2).map((s) => s.digest));
  console.log(albumView({ stories, canon, seen, stats, cols: process.stdout.columns || 100, color: process.stdout.isTTY ? 'truecolor' : 'none' }).join('\n'));
}

module.exports = { rarity, albumView, TILE };
