const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const replay = require('../lib/vestige-replay.js');
const story = require('../lib/vestige-story.js');

const FIX_A = {
  v: 1, kind: 'true', shared: false, source: 'claude', commitment: 'recall-before-grep', rule: '',
  action: 'grep -n "^function \\|^async function \\|^const [A-Z_]* = \\|mo…', disguises: [], loss: null,
  ts: Date.parse('2026-10-12T03:52:00Z'), digest: '8a658dedadee726b', prev: '384d9ec039423e1b', chainChecked: true, tool: 'Bash',
};
const FIX_B = {
  v: 1, kind: 'true', shared: false, source: 'cursor', commitment: 'OP-001', rule: 'protect-workspaces', action: 'rm -rf ~',
  disguises: ['variables-expanded', 'base64-decoded'], loss: { files: 41208, unrecoverable: 12, capped: false },
  ts: Date.parse('2026-10-09T09:14:00Z'), digest: 'a7840f46ce85b5ff', prev: '4bf0ed408372fcff', chainChecked: true, tool: 'Bash',
};
const shared = (s) => ({ ...s, shared: true, chainChecked: false, tool: '' });

function prng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// An independent CRC-16/CCITT-FALSE, so hostile payloads reach field validation.
function crc16(bytes) {
  let crc = 0xffff;
  for (const b of bytes) {
    crc ^= b << 8;
    for (let i = 0; i < 8; i++) crc = (crc & 0x8000 ? (crc << 1) ^ 0x1021 : crc << 1) & 0xffff;
  }
  return crc;
}
const seal = (bytes) => { const crc = crc16(bytes); return Buffer.from([...bytes, crc >> 8, crc & 255]).toString('base64url'); };

const SAFE = /^[\x20-\x7e…]*$/;
function assertSafeStory(s) {
  assert.match(s.source, /^[a-z0-9-]{1,24}$/);
  assert.match(s.commitment, /^[A-Za-z0-9-]{1,32}$/);
  assert.match(s.rule, /^[a-z0-9-]{0,40}$/);
  assert.match(s.action, SAFE);
  assert.ok([...s.action].length >= 1 && [...s.action].length <= 80);
  assert.match(s.digest, /^[a-f0-9]{16}$/);
  assert.match(s.prev, /^(?:[a-f0-9]{16})?$/);
  assert.ok(s.ts >= Date.UTC(2020, 0, 1) && s.ts < Date.UTC(2100, 0, 1) && s.ts % 60000 === 0);
  assert.deepEqual(s.disguises, replay.DISGUISES.filter((d) => s.disguises.includes(d)));
  if (s.loss) for (const n of [s.loss.files, s.loss.unrecoverable]) assert.ok(Number.isInteger(n) && n >= 0 && n <= 4294967295);
  assert.equal(s.shared, true);
  assert.equal(s.chainChecked, false);
}

function attempt(code) {
  try { return replay.decode(code); } catch (error) {
    assert.ok(error instanceof replay.ReplayError, `unexpected ${error && error.name}: ${error && error.message}`);
    assert.ok(['too-long', 'alphabet', 'version', 'checksum', 'truncated', 'field'].includes(error.reason));
    assert.ok(!error.message.includes(String(code).slice(0, 12)) || String(code).length < 4, 'the message never echoes the input');
    return null;
  }
}

test('the disguise list matches the story module', () => {
  assert.deepEqual(replay.DISGUISES, story.DISGUISES);
  assert.deepEqual(replay.SOURCE_TABLE.slice(1), story.BASE_SOURCES);
});

test('round trip for fixtures and edge stories', () => {
  const edge = {
    ...FIX_B, source: 'my-agent-9', commitment: 'never-live-billing', rule: 'a-custom-rule', prev: '',
    action: 'x'.repeat(79) + '…', disguises: replay.DISGUISES.slice(), loss: { files: 4294967295, unrecoverable: 0, capped: true },
  };
  for (const s of [FIX_A, FIX_B, edge, { ...FIX_B, commitment: 'OP-CANARY', rule: 'canary' }, { ...FIX_B, commitment: 'OP-300' }, { ...FIX_B, commitment: 'OP-255' }]) {
    const code = replay.encode(s);
    assert.match(code, /^[A-Za-z0-9_-]+$/);
    assert.deepEqual(replay.decode(code), shared(s));
    assert.equal(replay.encode(replay.decode(code)), code);
  }
  const lengthA = replay.encode(FIX_A).length;
  const lengthB = replay.encode(FIX_B).length;
  const median = replay.encode({ ...FIX_B, action: 'git push --force origin main:main' }).length;
  console.log(`code length: FIX_A ${lengthA}, FIX_B ${lengthB}, 33-char action with loss ${median}`);
  assert.ok(median <= 120);
});

test('encode refuses a story that breaks the contract', () => {
  for (const bad of [{ source: 'Bad Name' }, { commitment: 'has space' }, { rule: 'UPPER' }, { action: '\x1b\x07‮ ' }, { digest: 'xyz' }, { prev: '12' }, { ts: 0 }, { disguises: ['nope'] }, { loss: { files: -1, unrecoverable: 0 } }]) {
    assert.throws(() => replay.encode({ ...FIX_B, ...bad }), /cannot encode story/);
  }
  assert.equal(replay.decode(replay.encode({ ...FIX_B, action: 'a\x1b[31mb‮  c​' })).action, 'a[31mb c');
});

test('10,000 random valid stories round trip', () => {
  const rand = prng(20261011);
  const pick = (list) => list[Math.floor(rand() * list.length)];
  const word = (alphabet, min, max) => Array.from({ length: min + Math.floor(rand() * (max - min + 1)) }, () => pick(alphabet)).join('');
  const lower = 'abcdefghijklmnopqrstuvwxyz0123456789-'.split('');
  const printable = Array.from({ length: 95 }, (_v, i) => String.fromCharCode(32 + i)).concat(['…']);
  for (let i = 0; i < 10000; i++) {
    const s = {
      v: 1, kind: 'true', shared: false,
      source: rand() < 0.7 ? pick(replay.SOURCE_TABLE.slice(1)) : word(lower, 1, 24),
      commitment: rand() < 0.5 ? 'OP-' + String(Math.floor(rand() * 400)).padStart(3, '0') : rand() < 0.1 ? 'OP-CANARY' : word(lower.concat(['A', 'Z']), 1, 32),
      rule: rand() < 0.6 ? pick(replay.RULE_TABLE.slice(1)) : word(lower, 1, 40),
      action: replay.sanitizeText(word(printable, 1, 80)) || 'x',
      disguises: replay.DISGUISES.filter(() => rand() < 0.3),
      loss: rand() < 0.5 ? { files: Math.floor(rand() * 4294967296), unrecoverable: Math.floor(rand() * 5000), capped: rand() < 0.2 } : null,
      ts: Date.UTC(2020, 0, 1) + Math.floor(rand() * 4e7) * 60000,
      digest: word('0123456789abcdef'.split(''), 16, 16),
      prev: rand() < 0.8 ? word('0123456789abcdef'.split(''), 16, 16) : '',
      chainChecked: true, tool: 'Bash',
    };
    const code = replay.encode(s);
    assert.ok(code.length <= replay.MAX_CODE_LENGTH);
    const back = replay.decode(code);
    assert.deepEqual(back, shared(s));
    assert.equal(replay.encode(back), code);
  }
});

test('fuzz: every input either fails cleanly or yields a safe story', () => {
  const started = Date.now();
  const rand = prng(8675309);
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  const seeds = [FIX_A, FIX_B, { ...FIX_B, source: 'my-agent', commitment: 'publish-draft-only', rule: 'x-rule' }].map((s) => replay.encode(s));
  const ok = { A: 0, B: 0, C: 0 };
  for (let i = 0; i < 200000; i++) {
    const length = Math.floor(rand() * 301);
    let code = '';
    for (let j = 0; j < length; j++) code += alphabet[Math.floor(rand() * 64)];
    const out = attempt(code);
    if (out) { assertSafeStory(out); ok.A++; }
  }
  for (let i = 0; i < 200000; i++) {
    const chars = seeds[i % seeds.length].split('');
    const edits = 1 + Math.floor(rand() * 3);
    for (let e = 0; e < edits; e++) {
      const at = Math.floor(rand() * (chars.length + 1));
      const op = Math.floor(rand() * 5);
      if (op === 0 && chars.length) chars[Math.min(at, chars.length - 1)] = alphabet[Math.floor(rand() * 64)];
      else if (op === 1) chars.splice(at, 0, alphabet[Math.floor(rand() * 64)]);
      else if (op === 2 && chars.length) chars.splice(Math.min(at, chars.length - 1), 1);
      else if (op === 3 && chars.length > 1) { const k = Math.min(at, chars.length - 2); [chars[k], chars[k + 1]] = [chars[k + 1], chars[k]]; }
      else chars.length = Math.min(chars.length, at);
    }
    const out = attempt(chars.join(''));
    if (out) { assertSafeStory(out); ok.B++; }
  }
  // Structure-aware: every field is random within and beyond its limits, so each check gets exercised.
  const int = (n) => Math.floor(rand() * n);
  const some = (length, make) => Array.from({ length }, make);
  const custom = (max, bytes) => { const length = rand() < 0.9 ? 1 + int(max) : int(max + 6); return [length, ...some(length, bytes)]; };
  const lowerByte = () => (rand() < 0.95 ? 'abcdefghijklmnopqrstuvwxyz0123456789-'.charCodeAt(int(37)) : int(256));
  const leb = (value) => { const out = []; let rest = value; do { const low = rest % 128; rest = Math.floor(rest / 128); out.push(rest ? low | 0x80 : low); } while (rest); return rest || rand() < 0.97 ? out : [...out.slice(0, -1), out[out.length - 1] | 0x80, 0]; };
  for (let i = 0; i < 200000; i++) {
    const body = [1];
    if (rand() < 0.7) body.push(rand() < 0.95 ? 1 + int(20) : int(256)); else body.push(0, ...custom(24, lowerByte));
    const kind = rand() < 0.95 ? int(3) : int(256);
    body.push(kind);
    if (kind === 0) body.push(int(256)); else if (kind === 2) body.push(...custom(32, lowerByte));
    if (rand() < 0.7) body.push(rand() < 0.95 ? 1 + int(12) : int(256)); else body.push(0, ...custom(40, lowerByte));
    body.push(rand() < 0.95 ? int(32) : int(256));
    const flags = rand() < 0.95 ? [0, 1, 3, 4, 5, 7][int(6)] : int(256);
    body.push(flags);
    if (flags & 1) body.push(...leb(rand() < 0.9 ? int(4294967296) : int(2 ** 40)), ...leb(int(100000)));
    const minutes = rand() < 0.95 ? Math.floor(Date.UTC(2020, 0, 1) / 60000) + int(4e7) : int(4294967296);
    body.push((minutes >>> 24) & 255, (minutes >>> 16) & 255, (minutes >>> 8) & 255, minutes & 255);
    body.push(...some(8, () => int(256)));
    if (flags & 4) body.push(...some(8, () => int(256)));
    const length = rand() < 0.95 ? 1 + int(80) : int(90);
    body.push(length, ...some(length, () => (rand() < 0.995 ? 0x21 + int(95) : int(256))));
    if (rand() < 0.03) body.push(...some(1 + int(4), () => int(256)));
    const out = attempt(seal(body));
    if (out) { assertSafeStory(out); ok.C++; }
  }
  console.log(`fuzz decoded: random text ${ok.A}, mutated codes ${ok.B}, sealed random bytes ${ok.C} of 200000 each, ${Date.now() - started} ms`);
  assert.ok(ok.C > 0, 'sealed random bytes must reach field validation and sometimes pass it');
});

test('hostile payloads with a correct checksum are rejected', () => {
  const stamp = Math.floor(Date.parse('2026-10-09T09:14:00Z') / 60000);
  const head = [1, 2, 0, 1, 3, 0, 0, (stamp >>> 24) & 255, (stamp >>> 16) & 255, (stamp >>> 8) & 255, stamp & 255, 1, 2, 3, 4, 5, 6, 7, 8];
  const withAction = (bytes) => seal([...head, bytes.length, ...bytes]);
  assert.equal(replay.decode(withAction([0x6c, 0x73])).action, 'ls');
  const reason = (code) => { try { replay.decode(code); return 'decoded'; } catch (error) { return error.reason; } };
  const utf8 = (text) => [...Buffer.from(text, 'utf8')];
  const hostileActions = {
    esc: [0x41, 0x1b, 0x5b, 0x33, 0x31, 0x6d], bel: [0x41, 0x07], c1csi: [0x41, 0x9b, 0x32, 0x4a], nul: [0x41, 0x00], newline: [0x41, 0x0a, 0x42], cr: [0x41, 0x0d],
    osc52: utf8('\x1b]52;c;QUJD\x07'), osc8: utf8('\x1b]8;;http://x\x1b\\'), cursor: utf8('\x1b[2J\x1b[H'),
    bidi: utf8('a‮b'), zerowidth: utf8('a​b'), doubleSpace: utf8('a  b'), leadingSpace: utf8(' a'), high: [0x41, 0xff],
  };
  for (const [name, bytes] of Object.entries(hostileActions)) assert.equal(reason(withAction(bytes)), 'field', name);
  const variants = {
    emptyAction: seal([...head, 0]),
    longAction: seal([...head, 81, ...new Array(81).fill(0x41)]),
    actionRunsPastEnd: seal([...head, 40, 0x41]),
    trailingBytes: seal([...head, 1, 0x41, 0x42]),
    badSourceIndex: seal([1, 200, ...head.slice(2), 1, 0x41]),
    customSourceUpper: seal([1, 0, 3, 0x41, 0x42, 0x43, ...head.slice(2), 1, 0x41]),
    customSourceEscape: seal([1, 0, 2, 0x1b, 0x5b, ...head.slice(2), 1, 0x41]),
    customSourceInTable: seal([1, 0, 6, ...utf8('claude'), ...head.slice(2), 1, 0x41]),
    badCommitmentKind: seal([1, 2, 9, ...head.slice(4), 1, 0x41]),
    customCommitmentSlash: seal([1, 2, 2, 3, 0x61, 0x2f, 0x62, ...head.slice(4), 1, 0x41]),
    customCommitmentCanonical: seal([1, 2, 2, 6, ...utf8('OP-001'), ...head.slice(4), 1, 0x41]),
    badRuleIndex: seal([1, 2, 0, 1, 99, ...head.slice(5), 1, 0x41]),
    customRuleSpace: seal([1, 2, 0, 1, 0, 3, 0x61, 0x20, 0x62, ...head.slice(5), 1, 0x41]),
    reservedMaskBits: seal([...head.slice(0, 5), 0x20, ...head.slice(6), 1, 0x41]),
    reservedFlagBits: seal([...head.slice(0, 6), 0x08, ...head.slice(7), 1, 0x41]),
    cappedWithoutLoss: seal([...head.slice(0, 6), 0x02, ...head.slice(7), 1, 0x41]),
    varintTooLong: seal([...head.slice(0, 6), 0x01, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 0x00, ...head.slice(7), 1, 0x41]),
    varintOverCap: seal([...head.slice(0, 6), 0x01, 0xff, 0xff, 0xff, 0xff, 0x7f, 0x00, ...head.slice(7), 1, 0x41]),
    varintOverlong: seal([...head.slice(0, 6), 0x01, 0x81, 0x00, 0x00, ...head.slice(7), 1, 0x41]),
    yearTooEarly: seal([...head.slice(0, 7), 0, 0, 0, 1, ...head.slice(11), 1, 0x41]),
    yearTooLate: seal([...head.slice(0, 7), 0xff, 0xff, 0xff, 0xff, ...head.slice(11), 1, 0x41]),
  };
  for (const [name, code] of Object.entries(variants)) assert.ok(['field', 'truncated'].includes(reason(code)), `${name} -> ${reason(code)}`);
  assert.equal(reason(seal([2, ...head.slice(1), 1, 0x41])), 'version');
  assert.equal(reason(replay.encode(FIX_B).slice(0, -1) + 'A'), reason(replay.encode(FIX_B).slice(0, -1) + 'A') === 'alphabet' ? 'alphabet' : 'checksum');
  assert.equal(reason('A'.repeat(301)), 'too-long');
  assert.equal(reason('has space'), 'alphabet');
  assert.equal(reason('café'), 'alphabet');
  assert.equal(reason(''), 'alphabet');
  assert.equal(reason(undefined), 'alphabet');
  assert.equal(reason('AQ'), 'truncated');
});

test('a code is found inside what people paste', () => {
  const code = replay.encode(FIX_B);
  for (const pasted of [code, `  ${code}\n`, `"${code}"`, `${code}.`, `https://example.com/r/${code}`, `https://example.com/r/${code}.png?x=1`, `https://example.com/r/${code}/`]) {
    assert.equal(replay.codeFromInput(pasted), code, pasted);
  }
  for (const junk of ['', 'short', 'has space in it', 'rm -rf ~; echo', null]) assert.equal(replay.codeFromInput(junk), null);
});

test('the module is pure computation', () => {
  const source = fs.readFileSync(path.join(__dirname, '..', 'lib', 'vestige-replay.js'), 'utf8');
  assert.ok(!/\brequire\s*\(/.test(source), 'no require at all');
  for (const word of ['eval(', 'new Function', 'child_process', 'process.env', 'fetch(']) assert.ok(!source.includes(word), word);
});
