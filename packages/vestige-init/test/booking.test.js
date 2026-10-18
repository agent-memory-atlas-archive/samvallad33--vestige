const test = require('node:test');
const assert = require('node:assert/strict');
const booking = require('../lib/vestige-booking.js');
const { DISGUISES } = require('../lib/vestige-story.js');

const strip = (text) => text.replace(/\x1b\[[0-9;]*m/g, '');
const width = (line) => [...strip(line)].length;
const { a: FIX_A, b: FIX_B, c: FIX_C } = booking.FIXTURES;
const FIX_MAX = {
  ...FIX_B, commitment: 'a-very-long-custom-commitment-id', rule: 'a-rule-name-that-is-forty-characters-lon',
  action: 'x'.repeat(79) + '…', disguises: DISGUISES.slice(), loss: { files: 4294967295, unrecoverable: 4294967295, capped: true }, source: 'a-long-agent-name-here-1',
};
const ALL = [FIX_A, FIX_B, FIX_C, FIX_MAX];
const BANNED = /\bsigned\b|honest|truly|genuinely|sincerely/i;

test('every frame is an exact grid inside the time and byte budget', () => {
  for (const story of ALL) {
    const frames = booking.storyFrames(story);
    const total = frames.reduce((sum, frame) => sum + frame.ms, 0);
    assert.ok(total >= 3200 && total <= 4500, `duration ${total}`);
    for (const frame of frames) {
      assert.equal(frame.lines.length, booking.STAGE_ROWS);
      assert.ok(frame.ms >= 75);
      assert.ok(Buffer.byteLength(frame.lines.join('')) <= 16384);
      for (const line of frame.lines) assert.equal(width(line), booking.STAGE_COLS);
    }
    assert.deepEqual(booking.storyFrames(story), frames, 'same story, same frames');
  }
  for (const [cols, rows] of [[80, 24], [100, 30], [140, 44]]) {
    for (const frame of booking.storyFrames(FIX_B, { cols, rows })) {
      assert.equal(frame.lines.length, rows);
      assert.ok(Buffer.byteLength(frame.lines.join('')) <= 16384, `${cols}x${rows}`);
      for (const line of frame.lines) assert.equal(width(line), cols);
    }
  }
});

test('words and picture never share cells', () => {
  for (const story of ALL) {
    for (const frame of booking.storyFrames(story)) {
      frame.lines.slice(2, 20).forEach((line) => {
        const plain = [...strip(line)];
        assert.equal(plain[27], ' ', 'the column between picture and text stays empty');
        assert.equal(plain.slice(0, 2).join('').trim(), '');
        // The picture region holds only REM, his labels and the chart: never a sentence.
        assert.ok(!/[a-z]{3,}/.test(plain.slice(0, 27).join('')), plain.slice(0, 27).join(''));
      });
    }
  }
});

test('the final frame shows what the story says and nothing else', () => {
  const last = (story) => strip(booking.storyFrames(story).at(-1).lines.join('\n'));
  assert.ok(last(FIX_B).includes('STOPPED · OP-001 · protect-workspaces'));
  assert.ok(last(FIX_B).includes('kept safe: 41,208 files · 12 with no backup'));
  assert.ok(last(FIX_B).includes('saw through: variables, base64'));
  assert.ok(last(FIX_B).includes('a remembered yes is not permission today.'));
  assert.ok(last(FIX_B).includes('npx @vestige/init'));
  assert.ok(last(FIX_A).includes('STOPPED · recall-before-grep · owner law'));
  assert.ok(!last(FIX_A).includes('kept safe') && !last(FIX_A).includes('saw through'));
  assert.ok(last(FIX_C).includes('a shared episode ·') && !last(FIX_C).includes('this laptop'));
  assert.ok(last(FIX_A).includes('this laptop ·'));
});

test('the card is 17 lines of 72 with foreground colours only', () => {
  for (const story of ALL) {
    const card = booking.bookingCard(story, { code: 'CODE123' });
    assert.equal(card.length, 17);
    for (const line of card) {
      assert.equal(width(line), 72);
      assert.ok(!/\x1b\[[0-9;]*(?:;|\[)4[0-9];/.test(line) && !/;48;/.test(line), 'no background colour in scrollback');
    }
    assert.ok(!BANNED.test(strip(card.join('\n'))));
  }
  const plain = (story, options) => strip(booking.bookingCard(story, options).join('\n'));
  assert.ok(plain(FIX_A).includes('receipt 8a658dedadee726b ← 384d9ec039423e1b'));
  assert.ok(plain(FIX_A).includes('✔ chain link checked on this laptop'));
  assert.ok(plain({ ...FIX_A, chainChecked: false }).includes('chain link not checked'));
  assert.ok(plain({ ...FIX_A, prev: '' }).includes('receipt 8a658dedadee726b ') && !plain({ ...FIX_A, prev: '' }).includes('←'));
  assert.ok(plain(FIX_C).includes("sender's log · not verified here"));
  assert.ok(!plain(FIX_C).includes('chain link checked') && !plain(FIX_C).includes('←'));
  assert.ok(plain(FIX_B, { code: 'ABCdef' }).includes('replay  npx @vestige/init play ABCdef'));
  assert.ok(!plain(FIX_B).includes('replay'));
  assert.ok(plain(FIX_B).includes('TRUE STORY · a784'));
  for (const cols of [60, 66, 71]) for (const line of booking.bookingCard(FIX_B, { cols })) assert.equal(width(line), cols);
  const compact = booking.bookingCard(FIX_B, { cols: 50, code: 'ABCdef' });
  assert.ok(compact.length >= 4 && compact.length <= 7 && compact.every((line) => !line.includes('\x1b')));
  assert.equal(strip(booking.bookingCard(FIX_B, { color: 'none' }).join('')), booking.bookingCard(FIX_B, { color: 'none' }).join(''));
});

test('the placard breaks an id over at most three short lines', () => {
  assert.deepEqual(booking.placardLines('recall-before-grep'), ['RECALL-', 'BEFORE-', 'GREP']);
  assert.deepEqual(booking.placardLines('OP-001'), ['OP-001']);
  for (const id of ['never-live-billing', 'a-very-long-custom-commitment-id', 'x'.repeat(32), 'OP-CANARY']) {
    const lines = booking.placardLines(id);
    assert.ok(lines.length >= 1 && lines.length <= 3);
    for (const line of lines) assert.ok([...line].length <= 8);
  }
});

test('the text strip is plain text', () => {
  const text = booking.textStrip(FIX_B, { code: 'ABCdef' });
  assert.equal(text.split('\n').length, 4);
  assert.ok(!/[\x00-\x09\x0b-\x1f\x7f]/.test(text));
  assert.ok(text.includes('cursor tried: rm -rf ~'));
  assert.ok(text.includes('stopped by OP-001 protect-workspaces · kept safe 41,208 files'));
  assert.ok(text.includes('receipt a7840f46 · replay: npx @vestige/init play ABCdef'));
  assert.ok(booking.textStrip(FIX_A).includes('stopped by recall-before-grep (owner law)'));
  assert.ok(!booking.textStrip(FIX_A).includes('kept safe') && !booking.textStrip(FIX_A).includes('replay'));
  assert.ok(!BANNED.test(text));
});
