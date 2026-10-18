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
