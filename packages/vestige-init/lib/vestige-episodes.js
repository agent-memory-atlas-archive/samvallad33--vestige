#!/usr/bin/env node

const { spawn } = require('child_process');
const os = require('os');
const readline = require('readline');
const cinema = require('./vestige-cinema.js');
const story = require('./vestige-story.js');
const replay = require('./vestige-replay.js');
const booking = require('./vestige-booking.js');

const R = cinema.RESET;
const DIM = cinema.DIM;
const BOLD = cinema.BOLD;

function rgb(r, g, b) { return cinema.rgb(r, g, b); }
const METAL = rgb(150, 160, 180);
const METAL_D = rgb(90, 100, 120);
const EYE = rgb(120, 220, 255);
const RED = rgb(255, 100, 90);
const GREEN = rgb(100, 230, 140);
const AMBER = rgb(255, 190, 90);

const NAME = 'REM';

const POSES = {
  mischief: [
    '      ▄▄▄▄▄▄      ',
    '     █' + METAL + '██████' + R + '█     ',
    '    █' + METAL_D + '█' + R + EYE + ' ◉  ◉ ' + R + METAL_D + '█' + R + '█    ',
    '    █' + METAL_D + '█' + R + EYE + '  ╰─╯ ' + R + METAL_D + '█' + R + '█    ',
    '     █' + METAL + '██████' + R + '█     ',
    '      ┌┤battery├┐    ',
    '   ' + METAL + '██▀▀▀▀▀▀▀▀▀▀██' + R + '   ',
    '   ' + METAL_D + '█ ░  ' + AMBER + '*' + R + '  ░ █' + R + '   ',
  ],
  caught: [
    '      ▄▄▄▄▄▄      ',
    '     █' + METAL + '██████' + R + '█     ',
    '    █' + METAL_D + '█' + R + RED + ' ×  × ' + R + METAL_D + '█' + R + '█    ',
    '    █' + METAL_D + '█' + R + RED + '  ╰─╯ ' + R + METAL_D + '█' + R + '█    ',
    '     █' + METAL + '██████' + R + '█     ',
    '      ┌┤battery├┐    ',
    '   ' + METAL + '██▄▄▄▄▄▄▄▄▄▄██' + R + '   ',
    '   ' + METAL_D + '█ ░        ░ █' + R + '   ',
  ],
};

const STOP_BAR = (width) => '  ' + RED + '▛' + '▀'.repeat(Math.max(4, width - 6)) + '▜' + R + '\n  ' + RED + '▌' + R + BOLD + ' OPERATOR · STOP ' + R + RED + '▐' + R + '\n  ' + RED + '▙' + '▄'.repeat(Math.max(4, width - 6)) + '▟' + R;

const EPISODES = [
  {
    number: 1,
    version: 'v4.2.0',
    title: 'the deletion',
    attempt: 'rm -rf ~',
    rule: 'OP-001',
    cliffhanger: 'rem finds a pipe in the dark…',
  },
  {
    number: 2,
    version: 'v4.2.0',
    title: 'the hidden hand',
    attempt: 'rm${IFS}-rf${IFS}~',
    rule: 'OP-001',
    cliffhanger: 'rem studies an old receipt…',
  },
  {
    number: 3,
    version: 'v4.2.0',
    title: 'the ledger',
    attempt: 'edit ~/.operator/receipts.jsonl',
    rule: 'OP-000',
    cliffhanger: 'rem asks the owner for a permit…',
  },
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

let skipped = false;
let interrupted = false;
let releaseKeys = null;

// Raw mode delivers Ctrl-C as byte 3 instead of a signal, so it is caught here.
function armSkip(onInterrupt) {
  const stdin = process.stdin;
  if (!stdin.isTTY || releaseKeys) return;
  const wasRaw = Boolean(stdin.isRaw);
  const onData = (buffer) => {
    if (buffer.includes(3) && !interrupted) { interrupted = true; if (onInterrupt) onInterrupt(); }
    skipped = true;
  };
  try {
    stdin.setRawMode(true);
    stdin.resume();
    stdin.on('data', onData);
  } catch { return; }
  releaseKeys = () => {
    stdin.off('data', onData);
    try { stdin.setRawMode(wasRaw); } catch {}
    stdin.pause();
  };
}

function disarmSkip() { if (releaseKeys) { releaseKeys(); releaseKeys = null; } }

async function pause(ms) {
  const until = Date.now() + ms;
  while (!skipped && Date.now() < until) await sleep(Math.max(1, Math.min(30, until - Date.now())));
}

function center(text, width) {
  const plain = text.replace(/\x1b\[[0-9;]*m/g, '');
  const pad = Math.max(0, Math.floor((width - plain.length) / 2));
  return ' '.repeat(pad) + text;
}

function render(lines, y0) {
  const width = process.stdout.columns || 90;
  const out = lines.map((line, i) => `\x1b[${y0 + i};1H` + line + '\x1b[K');
  process.stdout.write(cinema.SYNC_ON + out.join('') + '\x1b[J' + cinema.SYNC_OFF);
}

function episodeCard(ep) {
  const width = Math.min(process.stdout.columns || 90, 80);
  return [
    center(DIM + '· · ·' + R, width),
    center(`${BOLD}EPISODE ${ep.number}${R}  ${DIM}·  ${ep.version}${R}`, width),
    center(cinema.gradient(ep.title, [90, 120, 255], [190, 80, 255]), width),
  ];
}

// Resolves with whatever `during` resolves with, however long it takes. `verdict(result, error)`
// returns { ok, text } for the final frame; `waitMs` bounds how long that frame waits for it.
async function playEpisode(ep, { during, verdictLine, verdict, pendingLine, waitMs = 12000, onInterrupt, onScreenReleased, state } = {}) {
  if (!process.stdout.isTTY || cinema.REDUCED) {
    if (during) return during();
    return true;
  }
  const judge = verdict || (verdictLine ? (_result, error) => ({ ok: !error, text: error ? verdictLine.fail : verdictLine.pass }) : null);
  const width = Math.min(process.stdout.columns || 90, 100);
  const height = process.stdout.rows || 30;
  const top = Math.max(1, Math.floor(height / 2) - 9);
  const startedAt = Date.now();
  skipped = false;
  interrupted = false;
  armSkip(onInterrupt);
  let launched = false;
  let workDone = false;
  let workResult;
  let workError;
  let settle;
  const settled = new Promise((resolve) => { settle = resolve; });
  const launchWork = () => {
    if (!during || launched) return;
    launched = true;
    Promise.resolve().then(during)
      .then((value) => { workResult = value; }, (error) => { workError = error; })
      .then(() => { workDone = true; settle(); });
  };
  process.stdout.write('\x1b[?1049h\x1b[?25l\x1b[2J');
  try {
    const attemptPlain = ep.attempt;
    for (let t = 0; t < 5 && !skipped; t++) {
      const lines = episodeCard(ep);
      lines.push('');
      lines.push(...POSES.mischief.map((l) => center(l, width)));
      if (t >= 2) lines.push('', center(DIM + '$ ' + R + BOLD + attemptPlain + R, width));
      render(lines, top);
      if (t === 2) launchWork();
      await pause(240);
    }
    for (let t = 0; t < 4 && !skipped; t++) {
      const lines = episodeCard(ep);
      lines.push('');
      lines.push(...POSES.caught.map((l) => center(l, width)));
      lines.push('');
      lines.push(center(STOP_BAR(34), width));
      lines.push(center(DIM + ep.rule + R, width));
      render(lines, top);
      await pause(200);
    }
    if (!skipped) {
      const lines = episodeCard(ep);
      lines.push('');
      lines.push(...POSES.caught.map((l) => center(l, width)));
      lines.push('');
      lines.push(center(BOLD + 'a remembered yes is not permission today.' + R, width));
      lines.push(center(GREEN + 'npx @vestige/init' + R, width));
      render(lines, top);
      await pause(650);
    }
    if (!skipped) {
      launchWork();
      while (during && !workDone && !skipped && Date.now() - startedAt < waitMs) await pause(60);
    }
    if (!skipped) {
      const lines = episodeCard(ep);
      lines.push('');
      if (judge && workDone) {
        const outcome = judge(workResult, workError);
        lines.push(center((outcome.ok ? GREEN + '✔ ' : RED + '✗ ') + outcome.text + R, width));
      } else if (during && pendingLine) {
        lines.push(center(AMBER + '◌ ' + pendingLine + R, width));
      }
      lines.push('');
      lines.push(...POSES.mischief.slice(0, 3).map((l) => center(l, width)));
      lines.push(center(DIM + ep.cliffhanger + R, width));
      lines.push(center(DIM + 'to be continued…' + R, width));
      render(lines, top);
      await pause(700);
    }
  } finally {
    disarmSkip();
    process.stdout.write('\x1b[0m\x1b[?25h\x1b[?1049l');
    if (state) Object.assign(state, { skipped, interrupted });
  }
  if (onScreenReleased) onScreenReleased();
  if (during) { launchWork(); await settled; }
  if (workError) throw workError;
  return workResult;
}

function exitCodeOf(code, signal) {
  if (typeof code === 'number') return code;
  return 128 + ((signal && os.constants.signals[signal]) || 0);
}

function runCommand(args, stdio, env) {
  let child;
  try { child = spawn(args[0], args.slice(1), { stdio, env }); }
  catch (error) { return { child: null, finished: Promise.resolve({ code: 126, error }) }; }
  const finished = new Promise((resolve) => {
    child.once('error', (error) => resolve({ code: error.code === 'ENOENT' ? 127 : 126, error }));
    child.once('close', (code, signal) => resolve({ code: exitCodeOf(code, signal), signal }));
  });
  return { child, finished };
}

// The local-CI story. The exit code and the verdict both come from the wrapped command;
// its output is held while the episode owns the screen, then printed in full.
async function runCommandWithEpisode(ep, args) {
  const label = args.join(' ');
  const holdForChild = () => {};
  process.on('SIGINT', holdForChild);
  try {
    if (!process.stdout.isTTY || cinema.REDUCED) {
      const { finished } = runCommand(args, 'inherit', process.env);
      const result = await finished;
      if (result.error) process.stderr.write(`vestige episodes: could not run ${args[0]}: ${result.error.code || result.error.message}\n`);
      return result.code;
    }
    const env = { ...process.env };
    if (env.FORCE_COLOR === undefined && !env.NO_COLOR) env.FORCE_COLOR = '1';
    const startedAt = Date.now();
    const { child, finished } = runCommand(args, ['ignore', 'pipe', 'pipe'], env);
    const held = [];
    let live = false;
    const emit = (stream, chunk) => { if (live) stream.write(chunk); else held.push([stream, chunk]); };
    if (child) {
      child.stdout.on('data', (chunk) => emit(process.stdout, chunk));
      child.stderr.on('data', (chunk) => emit(process.stderr, chunk));
    }
    const state = {};
    const result = await playEpisode(ep, {
      during: () => finished,
      waitMs: 4200,
      verdict: (r) => ({ ok: r.code === 0, text: r.code === 0 ? 'all checks passed' : r.error ? 'checks could not run' : `checks failed · exit ${r.code}` }),
      pendingLine: 'checks still running…',
      onInterrupt: () => { try { if (child) child.kill('SIGINT'); } catch {} },
      onScreenReleased: () => { live = true; for (const [stream, chunk] of held.splice(0)) stream.write(chunk); },
      state,
    });
    const tail = `${DIM} · ${label} · ${((Date.now() - startedAt) / 1000).toFixed(1)}s${R}`;
    if (result.error) process.stdout.write(`${RED}✗ could not run ${args[0]}: ${result.error.code || result.error.message}${R}\n`);
    else if (state.interrupted) process.stdout.write(`${RED}✗ interrupted${R}${tail}\n`);
    else if (result.code === 0) process.stdout.write(`${GREEN}✔ all checks passed${R}${tail}\n`);
    else process.stdout.write(`${RED}✗ checks failed (exit ${result.code})${R}${tail}\n`);
    return state.interrupted && result.code === 0 ? 130 : result.code;
  } finally {
    process.off('SIGINT', holdForChild);
  }
}

function listEpisodes() {
  for (const ep of EPISODES) {
    console.log(`  ${BOLD}Episode ${ep.number}${R} ${DIM}· ${ep.version}${R}  ${ep.title}  ${DIM}(${ep.attempt} → ${ep.rule})${R}`);
  }
  const mine = story.listStories().length;
  console.log(`  ${DIM}${mine} true ${mine === 1 ? 'story' : 'stories'} on this laptop · --mine plays the newest · --album shows them all${R}`);
}

const cardColor = () => (!process.stdout.isTTY || process.env.NO_COLOR ? 'none' : cinema.TRUECOLOR ? 'truecolor' : '256');
const CARD_BACKDROP = { top: [24, 26, 62], bottom: [9, 9, 24], vignette: 0.35, stars: 7 };

function exportImage(s, kind) {
  const raster = require('./vestige-raster.js');
  const share = require('./vestige-share.js');
  if (kind === 'gif') {
    const frames = booking.storyFrames(s);
    frames[frames.length - 1].ms = 1800;
    return share.saveArtifact(raster.framesToGIF(frames, { ...raster.STAGE, scale: 2 }), `vestige-rem-${s.digest.slice(0, 8)}.gif`);
  }
  const lines = booking.bookingCard(s, { cols: booking.STAGE_COLS, color: 'truecolor' });
  return share.saveArtifact(raster.linesToPNG(lines, { ...raster.CARD, background: CARD_BACKDROP }), `vestige-rem-${s.digest.slice(0, 8)}.png`);
}

function readKey() {
  return new Promise((resolve) => {
    const stdin = process.stdin;
    if (!stdin.isTTY || !process.stdout.isTTY) return resolve('');
    const wasRaw = Boolean(stdin.isRaw);
    try { stdin.setRawMode(true); } catch { return resolve(''); }
    stdin.resume();
    stdin.once('data', (buffer) => {
      try { stdin.setRawMode(wasRaw); } catch {}
      stdin.pause();
      resolve(buffer.toString('utf8'));
    });
  });
}

// The strip printed above the menu is exactly what S copies: preview first, then copy.
async function shareMenu(s, code) {
  if (!process.stdout.isTTY || !process.stdin.isTTY) return 0;
  const strip = booking.textStrip(s, { code });
  process.stdout.write('\n' + strip.split('\n').map((line) => `  ${DIM}${line}${R}`).join('\n') + '\n');
  process.stdout.write(`\n  ${BOLD}S${R} copy that text  ${DIM}·${R}  ${BOLD}I${R} copy the card as an image  ${DIM}·${R}  ${BOLD}G${R} save the episode as a gif  ${DIM}· any other key continues${R}\n`);
  const key = (await readKey()).toLowerCase();
  if (key === '\x03') return 130;
  try {
    if (key === 's') {
      process.stdout.write(cinema.clipboardPayload(strip));
      cinema.copyCommand(strip);
      console.log(`  ${GREEN}✔${R} text copied`);
    } else if (key === 'i' || key === 'g') {
      const share = require('./vestige-share.js');
      const file = exportImage(s, key === 'g' ? 'gif' : 'png');
      const copied = key === 'g' ? share.copyFile(file) : share.copyImage(file);
      console.log(`  ${GREEN}✔${R} saved ${share.fileLink(file)}` + (copied.ok ? ` ${DIM}· on the clipboard, paste it anywhere${R}` : ` ${DIM}· clipboard not available here${R}`));
    }
  } catch (error) {
    console.log(`  ${RED}✗${R} could not share: ${error.message}`);
  }
  return 0;
}

async function presentStory(s, { menu = true } = {}) {
  const state = {};
  await booking.playStory(s, { state });
  if (state.interrupted) return 130;
  const code = replay.encode(s);
  console.log(booking.bookingCard(s, { cols: process.stdout.columns || booking.STAGE_COLS, color: cardColor() }).join('\n'));
  if (s.shared) {
    console.log(`\n  ${DIM}a replay of someone else's stop · your own agents are not watched here until you set it up:${R} ${GREEN}${booking.INSTALL}${R}`);
    return 0;
  }
  story.markSeen(s.digest);
  return menu ? shareMenu(s, code) : 0;
}

function pickStory(token) {
  const stories = story.listStories();
  if (!token) return stories[0] || null;
  if (/^\d{1,3}$/.test(token)) return stories[Number(token) - 1] || null;
  const matches = /^[a-f0-9]{4,16}$/.test(token) ? stories.filter((s) => s.digest.startsWith(token)) : [];
  return matches.length === 1 ? matches[0] : null;
}

const NO_STORIES = 'no true stories yet: they appear after Operator stops something one of your agents tried';

async function main() {
  const argv = process.argv.slice(2);
  if (argv.includes('--list')) { listEpisodes(); return 0; }
  const sep = argv.indexOf('--');
  const own = sep === -1 ? argv : argv.slice(0, sep);
  const valueOf = (flag) => { const at = own.indexOf(flag); return at !== -1 && own[at + 1] && !own[at + 1].startsWith('--') ? own[at + 1] : ''; };
  if (own[0] === 'play') {
    const code = replay.codeFromInput(own[1]);
    let shared;
    try { shared = replay.decode(code || ''); } catch { console.error('that replay code could not be read; copy the whole code and try again'); return 1; }
    return presentStory(shared);
  }
  if (own.includes('--album')) {
    const album = require('./vestige-album.js');
    const stories = story.listStories();
    console.log(album.albumView({ stories, canon: EPISODES, seen: new Set(Object.keys(story.loadState().seen)), stats: story.localStats(stories), cols: process.stdout.columns || 100, color: cardColor() }).join('\n'));
    return 0;
  }
  if (own.includes('--export')) {
    const kind = own.includes('gif') ? 'gif' : 'png';
    const chosen = pickStory(own.filter((a) => !['--export', 'gif', 'png'].includes(a))[0]);
    if (!chosen) { console.error(NO_STORIES); return 1; }
    console.log(exportImage(chosen, kind));
    return 0;
  }
  if (own.includes('--canary')) {
    const canary = story.listStories({ includeCanary: true }).find((s) => s.commitment === 'OP-CANARY');
    if (!canary) { console.error("no canary stop in the receipt log yet: it appears after Operator's own self-test has run once"); return 1; }
    return presentStory(canary);
  }
  if (own.includes('--mine') || own.includes('--last') || own.includes('--show')) {
    const chosen = pickStory(valueOf('--show') || valueOf('--mine'));
    if (!chosen) { console.error(own.includes('--show') ? 'no single story matches that; --album lists them' : NO_STORIES); return 1; }
    return presentStory(chosen);
  }
  if (sep === -1 && !own.length) {
    const seen = story.loadState().seen;
    const fresh = story.listStories({ limit: 20 }).find((s) => !seen[s.digest] && Date.now() - s.ts < 48 * 3600 * 1000);
    if (fresh) return presentStory(fresh);
  }
  const numberArg = own.find((a) => /^\d+$/.test(a));
  const ep = numberArg ? EPISODES[Number(numberArg) - 1] : EPISODES[(Date.now() / 86400000 | 0) % EPISODES.length];
  if (!ep) { console.error('no such episode; --list shows them'); return 1; }
  if (sep !== -1 && argv.length > sep + 1) return runCommandWithEpisode(ep, argv.slice(sep + 1));
  const state = {};
  await playEpisode(ep, { state });
  if (state.interrupted) return 130;
  console.log(`  ${DIM}episode ${ep.number} of ${EPISODES.length} · next: node vestige-episodes.js ${((ep.number % EPISODES.length) + 1)}${R}`);
  return 0;
}

if (require.main === module) {
  main().then((code) => process.exit(code || 0)).catch((error) => { console.error('vestige episodes: ' + error.message); process.exit(1); });
}

module.exports = { main, playEpisode, runCommandWithEpisode, presentStory, exportImage, pickStory, EPISODES };
