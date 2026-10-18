const test = require('node:test');
const assert = require('node:assert/strict');
const stage = require('../lib/vestige-stage.js');

const tty = (depth = 8) => ({ isTTY: true, getColorDepth: () => depth });

test('colour ladder: NO_COLOR and dumb terminals win, then COLORTERM, then known terminals', () => {
  assert.equal(stage.colorMode({ NO_COLOR: '1', COLORTERM: 'truecolor' }, tty(24)), 'none');
  assert.equal(stage.colorMode({ TERM: 'dumb' }, tty(24)), 'none');
  assert.equal(stage.colorMode({}, { isTTY: false }), 'none');
  assert.equal(stage.colorMode({ VESTIGE_COLOR: '16', COLORTERM: 'truecolor' }, tty(24)), '16');
  // macOS 26 Terminal sets COLORTERM=truecolor while Node's own check says 256 (mem-00000000000a361b)
  assert.equal(stage.colorMode({ TERM_PROGRAM: 'Apple_Terminal', COLORTERM: 'truecolor', TERM: 'xterm-256color' }, tty(8)), 'truecolor');
  assert.equal(stage.colorMode({ TERM_PROGRAM: 'Apple_Terminal', TERM: 'xterm-256color' }, tty(8)), '256');
  assert.equal(stage.colorMode({ WT_SESSION: 'x' }, tty(4)), 'truecolor');
  assert.equal(stage.colorMode({ TERM: 'xterm-kitty' }, tty(4)), 'truecolor');
  assert.equal(stage.colorMode({ TERM: 'xterm' }, tty(4)), '16');
  assert.equal(stage.colorMode({ NO_COLOR: '' , TERM: 'xterm-256color' }, tty(8)), '256', 'an empty NO_COLOR is not set');
});

test('ascii drawing only for an explicit non-UTF-8 locale', () => {
  assert.equal(stage.asciiOnly({}), false);
  assert.equal(stage.asciiOnly({ LANG: 'en_US.UTF-8' }), false);
  assert.equal(stage.asciiOnly({ LC_ALL: 'C' }), true);
  assert.equal(stage.asciiOnly({ LANG: 'POSIX' }), true);
  assert.equal(stage.asciiOnly({ LANG: 'de_DE.ISO8859-1' }), true);
  assert.equal(stage.asciiOnly({ LC_ALL: 'C.UTF-8' }), false);
  assert.equal(stage.asciiOnly({ LANG: 'C', VESTIGE_ASCII: '0' }), false);
});

test('the canvas never holds a wide character, and drops emoji selectors', () => {
  const cv = new stage.Canvas(10, 1);
  cv.put(0, 0, 'a⚡b✔️c漢d');
  assert.equal(cv.text()[0], 'a?b✔c?d   ');
  assert.equal([...cv.text()[0]].length, 10);
});

test('rows render at exact width in every mode, ascii included', () => {
  const cv = new stage.Canvas(12, 2);
  cv.fill(0, 0, 11, 1, [10, 10, 30]);
  cv.put(0, 0, '╭─●○…→✔✗█▀▄', { fg: [255, 100, 90], bold: true });
  for (const mode of ['truecolor', '256', '16', 'none']) {
    const line = stage.renderRow(cv, 0, mode).replace(/\x1b\[[0-9;]*m/g, '');
    assert.equal([...line].length, 12, mode);
  }
  // an ellipsis becomes '.', never '~' (which reads like a home path); half blocks keep their weight as '#'
  assert.equal(stage.renderRow(cv, 0, 'none', true).replace(/\x1b\[[0-9;]*m/g, ''), '+-o..>vx### ');
  assert.doesNotMatch(stage.renderRow(cv, 0, 'none'), /38;|48;/, 'no colour codes in none');
  assert.doesNotMatch(stage.renderRow(cv, 0, '16'), /38;2|48;2|38;5|48;5/, 'only 16-colour codes');
  assert.doesNotMatch(stage.renderRow(cv, 0, '256'), /38;2|48;2/, 'no truecolor in 256');
});

test('keys: arrows, enter, escape, ctrl keys and pasted runs', () => {
  assert.deepEqual(stage.parseKeys('\x1b[A\x1b[B\x1b[C\x1b[D'), ['up', 'down', 'right', 'left']);
  assert.deepEqual(stage.parseKeys('\x1bOC\x1b[Z'), ['right', 'shift-tab']);
  assert.deepEqual(stage.parseKeys('\r\n \t\x7f'), ['enter', 'enter', 'space', 'tab', 'backspace']);
  assert.deepEqual(stage.parseKeys('\x1b'), ['esc']);
  assert.deepEqual(stage.parseKeys('\x03\x04'), ['ctrl-c', 'ctrl-d']);
  assert.deepEqual(stage.parseKeys('Qm6'), ['q', 'm', '6']);
  assert.deepEqual(stage.parseKeys('\x1b[3~'), ['delete']);
});

test('reserved cells keep decoration off words', () => {
  const cv = new stage.Canvas(10, 3);
  cv.reserve(2, 1, 5, 1);
  assert.equal(cv.free(1, 1), true);
  assert.equal(cv.free(2, 1), false);
  assert.equal(cv.free(5, 1), false);
  assert.equal(cv.free(6, 1), true);
  assert.equal(cv.free(-1, 0), false);
});

// Security review 2026-10-18, finding 2: C1 controls, DEL, bidi and format characters never reach the terminal.
test('the canvas turns C1, DEL, bidi and format characters into ?', () => {
  const cv = new stage.Canvas(16, 1);
  cv.put(0, 0, 'a\u009b2jb\u009dc\u007fd‮e​f⁦');
  const row = stage.renderRow(cv, 0, 'truecolor');
  assert.ok(!/[\u0080-\u009f\u007f‪-‮​-‏⁦-⁩]/.test(row), JSON.stringify(row));
  assert.equal(cv.text()[0].trimEnd(), 'a?2jb?c?d?e?f?');
  // ordinary drawing characters are untouched
  const ok = new stage.Canvas(8, 1);
  ok.put(0, 0, '╭─●…✔✗█');
  assert.equal(ok.text()[0].trimEnd(), '╭─●…✔✗█');
});

test('every way out of the screen also turns off reverse video and mouse modes', () => {
  const writes = [];
  const out = { isTTY: true, columns: 80, rows: 24, write: (s) => { writes.push(s); return true; }, on() {}, off() {}, getColorDepth: () => 24 };
  const input = { isTTY: false, isRaw: false, resume() {}, pause() {}, on() {}, off() {}, setRawMode() {} };
  const term = new stage.Term({ out, input, env: { COLORTERM: 'truecolor' } });
  term.open();
  term.restore();
  const last = writes[writes.length - 1];
  for (const mode of ['?5l', '?1000l', '?1002l', '?1003l', '?1006l', '?9l', '?25h', '?1049l']) assert.ok(last.includes(`\x1b[${mode}`), mode);
  assert.ok(require('../lib/vestige-intro.js').SCREEN_OFF.includes('\x1b[?5l'), 'the intro resets its flash on every exit');
});

// 2026-10-18: npx run with no shell above it (tmux new-window 'npx ...', exec npx, an IDE task) froze for good on
// ctrl-z, because the npm path sent SIGSTOP to a group nothing could ever resume.
test('ctrl-z stops the job only when a job-control shell above can resume it', () => {
  const me = process.pid, up = process.ppid;
  const table = (rows) => rows.map((r) => r.join(' ')).join('\n');
  // node -> npm (same group) -> interactive shell (own group, same terminal)
  assert.equal(stage.jobShellAbove(table([[me, up, 500, 'ttys003'], [up, 400, 500, 'ttys003'], [400, 300, 400, 'ttys003']])), true);
  // node -> npm as the session leader of a tmux pane -> the tmux server, which has no terminal
  assert.equal(stage.jobShellAbove(table([[me, up, 500, 'ttys003'], [up, 77, 500, 'ttys003'], [77, 1, 77, '??']])), false);
  // a parent group on another terminal is not a shell that can bring this job back
  assert.equal(stage.jobShellAbove(table([[me, up, 500, 'pts/4'], [up, 9, 600, 'pts/1']])), false);
  // no terminal at all, and a table that does not list this process
  assert.equal(stage.jobShellAbove(table([[me, up, 500, '?']])), false);
  assert.equal(stage.jobShellAbove(table([[1, 0, 1, '??']])), null);

  const sent = [];
  const kill = process.kill;
  process.kill = (pid, sig) => { sent.push([pid, sig]); return true; };
  let resumed = 0;
  try {
    stage.stopJob(() => { resumed++; }, () => false);
  } finally { process.kill = kill; }
  assert.deepEqual(sent, [], 'no stop signal when nobody could send SIGCONT');
  return new Promise((done) => setTimeout(() => { assert.equal(resumed, 1, 'the screen is taken back'); done(); }, 20));
});
