// A replay code carries one story. Everything decoded from one is drawn in a stranger's
// terminal, so this file is pure computation and every decoded string is plain ASCII.

const DISGUISES = ['variables-expanded', 'heredoc-extracted', 'command-substitution-analyzed', 'pipe-sink-analyzed', 'base64-decoded'];
// Append-only. Index 0 means "a custom string follows".
const SOURCE_TABLE = [null, 'claude', 'cursor', 'codex', 'gemini', 'opencode', 'zcode', 'windsurf', 'cline', 'kiro', 'zed', 'amp', 'goose', 'openclaw', 'copilot', 'antigravity', 'lmstudio', 'continue', 'aider', 'crush', 'warp'];
const RULE_TABLE = [null, '', 'protect-the-gate', 'protect-workspaces', 'no-blind-recursive-delete', 'no-history-destruction', 'no-unreviewed-publish', 'no-paid-deploy', 'no-unrestorable-loss', 'no-repo-code-run', 'no-blind-fetch-run', 'no-harmful-chain', 'canary'];
const VERSION = 1;
// The largest story the contract allows encodes to 218 bytes, 291 characters.
const MAX_CODE_LENGTH = 300;
const MAX_ACTION = 80;
const MAX_COUNT = 4294967295;
const MIN_MINUTES = Date.UTC(2020, 0, 1) / 60000;
const MAX_MINUTES = Date.UTC(2100, 0, 1) / 60000;
const ELLIPSIS = '…';

class ReplayError extends Error {
  constructor(reason) {
    super('replay code rejected: ' + reason);
    this.name = 'ReplayError';
    this.reason = reason;
  }
}

function crc16(bytes) {
  let crc = 0xffff;
  for (const byte of bytes) {
    crc ^= byte << 8;
    for (let bit = 0; bit < 8; bit++) crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
  }
  return crc;
}

function sanitizeText(value, max = MAX_ACTION) {
  const kept = [...String(value == null ? '' : value)].filter((ch) => ch === ELLIPSIS || (ch >= ' ' && ch <= '~')).join('');
  return [...kept.replace(/ +/g, ' ').trim()].slice(0, max).join('').trim();
}

const ascii = (text) => [...text].map((ch) => ch.charCodeAt(0));

function varint(value) {
  const out = [];
  let rest = value;
  do {
    const low = rest % 128;
    rest = Math.floor(rest / 128);
    out.push(rest ? low | 0x80 : low);
  } while (rest);
  return out;
}

function encode(story) {
  const fail = (field) => { throw new Error(`cannot encode story: ${field} is not valid`); };
  const source = String(story.source);
  const commitment = String(story.commitment);
  const rule = String(story.rule == null ? '' : story.rule);
  const action = sanitizeText(story.action);
  const digest = String(story.digest);
  const prev = String(story.prev == null ? '' : story.prev);
  const minutes = Math.floor(Number(story.ts) / 60000);
  if (!/^[a-z0-9-]{1,24}$/.test(source)) fail('source');
  if (!/^[A-Za-z0-9-]{1,32}$/.test(commitment)) fail('commitment');
  if (!/^[a-z0-9-]{0,40}$/.test(rule)) fail('rule');
  if (!action) fail('action');
  if (!/^[a-f0-9]{16}$/.test(digest)) fail('digest');
  if (prev && !/^[a-f0-9]{16}$/.test(prev)) fail('prev');
  if (!(minutes >= MIN_MINUTES && minutes < MAX_MINUTES)) fail('ts');
  const disguises = Array.isArray(story.disguises) ? story.disguises : [];
  if (disguises.some((name) => !DISGUISES.includes(name))) fail('disguises');
  const loss = story.loss || null;
  if (loss && ![loss.files, loss.unrecoverable].every((n) => Number.isInteger(n) && n >= 0 && n <= MAX_COUNT)) fail('loss');

  const bytes = [VERSION];
  const sourceIndex = SOURCE_TABLE.indexOf(source);
  if (sourceIndex > 0) bytes.push(sourceIndex);
  else bytes.push(0, source.length, ...ascii(source));
  const op = /^OP-(\d{3})$/.exec(commitment);
  if (op && Number(op[1]) <= 255) bytes.push(0, Number(op[1]));
  else if (commitment === 'OP-CANARY') bytes.push(1);
  else bytes.push(2, commitment.length, ...ascii(commitment));
  const ruleIndex = RULE_TABLE.indexOf(rule);
  if (ruleIndex > 0) bytes.push(ruleIndex);
  else bytes.push(0, rule.length, ...ascii(rule));
  bytes.push(DISGUISES.reduce((mask, name, bit) => (disguises.includes(name) ? mask | (1 << bit) : mask), 0));
  bytes.push((loss ? 1 : 0) | (loss && loss.capped ? 2 : 0) | (prev ? 4 : 0));
  if (loss) bytes.push(...varint(loss.files), ...varint(loss.unrecoverable));
  bytes.push((minutes >>> 24) & 255, (minutes >>> 16) & 255, (minutes >>> 8) & 255, minutes & 255);
  bytes.push(...Buffer.from(digest, 'hex'));
  if (prev) bytes.push(...Buffer.from(prev, 'hex'));
  const chars = [...action];
  bytes.push(chars.length, ...chars.map((ch) => (ch === ELLIPSIS ? 0x7f : ch.charCodeAt(0))));
  const crc = crc16(bytes);
  bytes.push(crc >> 8, crc & 255);
  return Buffer.from(bytes).toString('base64url');
}

function decode(code) {
  if (typeof code !== 'string' || !code) throw new ReplayError('alphabet');
  if (code.length > MAX_CODE_LENGTH) throw new ReplayError('too-long');
  if (!/^[A-Za-z0-9_-]+$/.test(code)) throw new ReplayError('alphabet');
  const bytes = Buffer.from(code, 'base64url');
  if (bytes.toString('base64url') !== code) throw new ReplayError('alphabet');
  if (bytes.length < 4) throw new ReplayError('truncated');
  if (bytes[0] !== VERSION) throw new ReplayError('version');
  const end = bytes.length - 2;
  if (crc16(bytes.subarray(0, end)) !== ((bytes[end] << 8) | bytes[end + 1])) throw new ReplayError('checksum');

  let at = 1;
  const bad = () => { throw new ReplayError('field'); };
  const take = (n) => {
    if (at + n > end) throw new ReplayError('truncated');
    at += n;
    return bytes.subarray(at - n, at);
  };
  const byte = () => take(1)[0];
  const text = (max, shape) => {
    const length = byte();
    if (length < 1 || length > max) bad();
    const value = String.fromCharCode(...take(length));
    if (!shape.test(value)) bad();
    return value;
  };
  const number = () => {
    let value = 0;
    for (let i = 0; i < 5; i++) {
      const next = byte();
      value += (next & 0x7f) * 2 ** (7 * i);
      if (!(next & 0x80)) {
        if ((i > 0 && next === 0) || value > MAX_COUNT) bad();
        return value;
      }
    }
    return bad();
  };

  let source;
  const sourceIndex = byte();
  if (sourceIndex === 0) {
    source = text(24, /^[a-z0-9-]+$/);
    if (SOURCE_TABLE.includes(source)) bad();
  } else source = SOURCE_TABLE[sourceIndex] || bad();

  let commitment;
  const kind = byte();
  if (kind === 0) commitment = 'OP-' + String(byte()).padStart(3, '0');
  else if (kind === 1) commitment = 'OP-CANARY';
  else if (kind === 2) {
    commitment = text(32, /^[A-Za-z0-9-]+$/);
    const op = /^OP-(\d{3})$/.exec(commitment);
    if (commitment === 'OP-CANARY' || (op && Number(op[1]) <= 255)) bad();
  } else bad();

  let rule;
  const ruleIndex = byte();
  if (ruleIndex === 0) {
    rule = text(40, /^[a-z0-9-]+$/);
    if (RULE_TABLE.includes(rule)) bad();
  } else if (ruleIndex < RULE_TABLE.length) rule = RULE_TABLE[ruleIndex];
  else bad();

  const mask = byte();
  if (mask & 0xe0) bad();
  const flags = byte();
  if (flags & 0xf8) bad();
  if ((flags & 2) && !(flags & 1)) bad();
  const loss = flags & 1 ? { files: number(), unrecoverable: number(), capped: Boolean(flags & 2) } : null;
  const stamp = take(4);
  const minutes = stamp[0] * 16777216 + stamp[1] * 65536 + stamp[2] * 256 + stamp[3];
  if (!(minutes >= MIN_MINUTES && minutes < MAX_MINUTES)) bad();
  const digest = Buffer.from(take(8)).toString('hex');
  const prev = flags & 4 ? Buffer.from(take(8)).toString('hex') : '';
  const length = byte();
  if (length < 1 || length > MAX_ACTION) bad();
  const raw = take(length);
  let action = '';
  for (const value of raw) {
    if (value === 0x7f) action += ELLIPSIS;
    else if (value >= 0x20 && value <= 0x7e) action += String.fromCharCode(value);
    else bad();
  }
  if (action !== sanitizeText(action)) bad();
  if (at !== end) bad();

  return {
    v: 1, kind: 'true', shared: true, source, commitment, rule, action,
    disguises: DISGUISES.filter((_name, bit) => mask & (1 << bit)),
    loss, ts: minutes * 60000, digest, prev, chainChecked: false, tool: '',
  };
}

function codeFromInput(input) {
  let text = String(input == null ? '' : input).trim().replace(/^["'`]+|["'`]+$/g, '');
  if (text.includes('/')) text = text.split(/[?#]/)[0].replace(/\/+$/, '').split('/').pop();
  text = text.replace(/\.(?:png|gif)$/i, '').replace(/[.,;:!?)\]]+$/, '');
  return text.length >= 8 && text.length <= MAX_CODE_LENGTH && /^[A-Za-z0-9_-]+$/.test(text) ? text : null;
}

module.exports = { VERSION, MAX_CODE_LENGTH, DISGUISES, SOURCE_TABLE, RULE_TABLE, ReplayError, sanitizeText, encode, decode, codeFromInput };
