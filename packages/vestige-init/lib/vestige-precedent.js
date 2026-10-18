#!/usr/bin/env node
// One Operator stop as a case: its record on the hash chain, its memory, every earlier case under the
// same rule, the permit state, and what the agent did next. Everything is read from this machine and
// nothing is written: receipts and permits under $OPERATOR_HOME (or ~/.operator), the gate's own rule
// list, and Vestige memory through the local dashboard API on loopback only.

const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const http = require('http');
const { spawnSync } = require('child_process');
const story = require('./vestige-story.js');

// The same in-band note the precedent MCP tool ships with every permit.
const PERMIT_NOTE = 'history, not permission. Permits are single-use and cannot be reused or issued through this tool.';
const GENESIS = '0'.repeat(64);
const DAY_FILE = /^\d{4}-\d{2}-\d{2}\.jsonl$/;
const DAY_MS = 86400000;

const operatorHome = (env = process.env) => env.OPERATOR_HOME || path.join(os.homedir(), '.operator');

// The gate's digest is sha256 over the record without `digest`, keys sorted, compact separators.
// Most records escape non-ASCII (Python ensure_ascii); records whose line holds raw UTF-8 hash it raw.
function canonical(value, ascii = true) {
  if (value === null || value === undefined) return 'null';
  if (typeof value === 'string') {
    const text = JSON.stringify(value);
    return ascii ? text.replace(/[\u0080-￿]/g, (ch) => '\\u' + ch.charCodeAt(0).toString(16).padStart(4, '0')) : text;
  }
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) return '[' + value.map((v) => canonical(v, ascii)).join(',') + ']';
  return '{' + Object.keys(value).sort().map((key) => canonical(key, ascii) + ':' + canonical(value[key], ascii)).join(',') + '}';
}

const sha256 = (text) => crypto.createHash('sha256').update(text, 'utf8').digest('hex');

function digestOf(record, ascii = true) {
  const body = {};
  for (const key of Object.keys(record)) if (key !== 'digest') body[key] = record[key];
  return sha256(canonical(body, ascii));
}

// 'ascii' or 'utf8' when the record's own bytes give its digest, else null.
function rederive(record) {
  if (!record || typeof record.digest !== 'string') return null;
  if (digestOf(record, true) === record.digest) return 'ascii';
  if (/[^\u0000-\u007f]/.test(JSON.stringify(record)) && digestOf(record, false) === record.digest) return 'utf8';
  return null;
}

function readLog(home = operatorHome(), { maxFiles = 400 } = {}) {
  const dir = path.join(home, 'receipts');
  let files;
  try { files = fs.readdirSync(dir).filter((name) => DAY_FILE.test(name)).sort().slice(-maxFiles); } catch { return { dir, files: [], records: [], unreadable: 0, head: '' }; }
  const records = [];
  let unreadable = 0;
  for (const file of files) {
    let text;
    try { text = fs.readFileSync(path.join(dir, file), 'utf8'); } catch { continue; }
    const lines = text.split('\n');
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      if (!line) continue;
      let record;
      try { record = JSON.parse(line); } catch { unreadable++; continue; }
      if (!record || typeof record !== 'object' || Array.isArray(record)) { unreadable++; continue; }
      Object.defineProperty(record, '_file', { value: file, enumerable: false });
      Object.defineProperty(record, '_line', { value: i + 1, enumerable: false });
      records.push(record);
    }
  }
  let head = '';
  try { head = fs.readFileSync(path.join(dir, 'HEAD'), 'utf8').trim(); } catch {}
  return { dir, files, records, unreadable, head };
}

// Checks every record against its own bytes and every link against the record before it.
// Runs in slices so a caller can show the real count while it climbs.
async function verifyChain(log, { slice = 1500, onProgress } = {}) {
  const started = Date.now();
  const { records } = log;
  const seen = new Map();
  const result = { total: records.length, rederived: 0, notRederived: [], linked: 0, forks: [], breaks: [], genesis: false, headMatches: false, ms: 0 };
  for (let start = 0; start < records.length; start += slice) {
    const end = Math.min(records.length, start + slice);
    for (let i = start; i < end; i++) {
      const record = records[i];
      if (rederive(record)) result.rederived++; else if (result.notRederived.length < 50) result.notRederived.push(i);
      if (i === 0) result.genesis = record.prev === GENESIS;
      else if (record.prev === records[i - 1].digest) result.linked++;
      else if (seen.has(record.prev)) { if (result.forks.length < 50) result.forks.push({ index: i, parent: seen.get(record.prev), ts: record.ts }); }
      else if (result.breaks.length < 50) result.breaks.push({ index: i, ts: record.ts });
      if (typeof record.digest === 'string') seen.set(record.digest, i);
    }
    result.checked = end;
    result.ms = Date.now() - started;
    if (onProgress) onProgress({ ...result });
    if (end < records.length) await new Promise((resolve) => setImmediate(resolve));
  }
  const last = records[records.length - 1];
  result.headMatches = Boolean(last && log.head && log.head === last.digest);
  result.forkCount = records.length - 1 - result.linked - result.breaks.length;
  result.ms = Date.now() - started;
  return result;
}

// Tries a forgery on in-memory copies only and reports exactly where the chain gives it away.
function forge(log, index, { field = 'decision', value = 'PASS' } = {}) {
  const records = log.records;
  const original = records[index];
  if (!original) return null;
  const edited = { ...original, [field]: value };
  const step1 = { at: index, field, from: original[field], to: value, digestStillMatches: digestOf(edited, true) === original.digest || digestOf(edited, false) === original.digest };
  const resealed = digestOf(edited, true);
  const next = records[index + 1];
  const step2 = { newDigest: resealed, nextIndex: next ? index + 1 : null, nextStillLinks: next ? next.prev === resealed : null };
  const after = records.length - 1 - index;
  const step3 = { laterRecords: after, headWouldChange: Boolean(log.head) };
  return { step1, step2, step3 };
}

function ruleCatalog(home = operatorHome(), { python = process.env.OPERATOR_PYTHON || 'python3', timeoutMs = 5000 } = {}) {
  const catalog = new Map();
  const gate = path.join(home, 'gate', 'operator-gate.py');
  if (fs.existsSync(gate)) {
    const run = spawnSync(python, [gate, 'status', '--rules'], { encoding: 'utf8', timeout: timeoutMs, env: { ...process.env, OPERATOR_HOME: home } });
    for (const line of String(run.stdout || '').split('\n')) {
      const match = line.match(/^\s+(OP-[A-Z0-9]+)\s+(STOP|SHADOW)\s+(\S+)\s+(.+)$/);
      if (match) catalog.set(match[1], { id: match[1], mode: match[2], name: match[3], text: match[4].trim() });
    }
  }
  try {
    const laws = JSON.parse(fs.readFileSync(path.join(home, 'commitments.json'), 'utf8')).commitments || [];
    for (const law of laws) {
      if (law && typeof law.commitment_id === 'string' && law.state !== 'retired') {
        catalog.set(law.commitment_id, { id: law.commitment_id, mode: 'LAW', name: law.commitment_id, text: String(law.directive || '').trim() });
      }
    }
  } catch {}
  return catalog;
}

// A time in seconds between 2001 and 2100; anything else in a permit file is not a time.
const plausible = (v) => typeof v === 'number' && Number.isFinite(v) && v > 1e9 && v < 4.1e9;

// Every permit file in name order, as the precedent tool loads them.
function loadPermits(home = operatorHome()) {
  const dir = path.join(home, 'permits');
  let names;
  try { names = fs.readdirSync(dir).sort(); } catch { return []; }
  const permits = [];
  for (const name of names) {
    const match = name.match(/^(.+?)\.json(\.used|\.expired)?$/);
    if (!match) continue;
    let body;
    try { body = JSON.parse(fs.readFileSync(path.join(dir, name), 'utf8')); } catch { continue; }
    if (!body || typeof body !== 'object') continue;
    permits.push({
      action_digest: typeof body.action_digest === 'string' ? body.action_digest : match[1],
      status: match[2] === '.used' ? 'used' : match[2] === '.expired' ? 'expired' : 'active',
      granted: plausible(body.granted) ? body.granted : null,
      expires: plausible(body.expires) ? body.expires : null,
      single_use: body.single_use === true,
      by: typeof body.by === 'string' && /^[\w.@-]{1,24}$/.test(body.by) ? body.by : null,
      stop_receipt: typeof body.stop_receipt === 'string' ? body.stop_receipt : null,
      note: PERMIT_NOTE,
    });
  }
  return permits;
}

const unix = (ts) => { const t = Date.parse(ts); return Number.isNaN(t) ? null : Math.floor(t / 1000); };

// The permit that answers this record, by the precedent tool's rule: one naming this receipt wins;
// otherwise grant time pairs them, ALLOW/PERMIT_GRANTED the latest at or before, others the earliest
// at or after that names no other stop. Ties go to the first file by name.
function findPermit(permits, record) {
  const ad = record && record.action_digest;
  if (typeof ad !== 'string') return null;
  const same = permits.filter((p) => p.action_digest === ad);
  const named = same.find((p) => p.stop_receipt && p.stop_receipt === record.digest);
  if (named) return named;
  const ts = unix(record.ts);
  if (ts === null) return null;
  let best = null;
  let bestAt = null;
  const answering = record.decision === 'ALLOW' || record.decision === 'PERMIT_GRANTED';
  for (const p of same) {
    if (p.granted === null) continue;
    const g = Math.floor(p.granted);
    if (answering) { if (g <= ts && (bestAt === null || g > bestAt)) { best = p; bestAt = g; } }
    else if (g >= ts && !p.stop_receipt && (bestAt === null || g < bestAt)) { best = p; bestAt = g; }
  }
  return best;
}

// The precedent tool's rule filter: a case-insensitive part of rule, commitment or any commitments entry.
function matchesRule(record, needle) {
  const low = String(needle).toLowerCase();
  const hit = (value) => typeof value === 'string' && value.toLowerCase().includes(low);
  return hit(record.rule) || hit(record.commitment) || (Array.isArray(record.commitments) && record.commitments.some(hit));
}

// What precedent(rule=…, since=…) answers, in its own words, computed here from the same files.
function precedentSummary(records, { rule, sinceDays, now = Date.now() }) {
  const cutoff = Math.floor(now / 1000) - sinceDays * 86400;
  let stops = 0;
  let permitted = 0;
  for (const record of records) {
    const ts = unix(record.ts);
    if (ts === null || ts < cutoff) continue;
    if (rule && !matchesRule(record, rule)) continue;
    if (record.decision === 'STOP') stops++;
    else if (record.decision === 'ALLOW' && record.via === 'permit') permitted++;
  }
  return `${stops} stop${stops === 1 ? '' : 's'} in the last ${sinceDays}d` + (permitted ? `, ${permitted} permit${permitted === 1 ? '' : 's'} used` : '');
}

const ruleKeyOf = (record) => String(record.commitment || record.rule || '');
const dayKey = (ts) => new Date(ts).toISOString().slice(0, 10);

function caseLaw(records, index, catalog) {
  const record = records[index];
  const key = ruleKeyOf(record);
  const cases = [];
  for (let i = 0; i < records.length; i++) if (records[i].decision === 'STOP' && ruleKeyOf(records[i]) === key) cases.push(i);
  const position = cases.indexOf(index) + 1;
  const firstIndex = cases.find((i) => !Number.isNaN(Date.parse(records[i].ts))) ?? cases[0];
  const first = records[firstIndex];
  const sources = {};
  const days = new Map();
  let sameAction = 0;
  for (const i of cases) {
    const r = records[i];
    const source = String(r.source || 'unknown').toLowerCase();
    sources[source] = (sources[source] || 0) + 1;
    const t = Date.parse(r.ts);
    if (!Number.isNaN(t)) days.set(dayKey(t), (days.get(dayKey(t)) || 0) + 1);
    if (record.action_digest && r.action_digest === record.action_digest) sameAction++;
  }
  const rule = catalog.get(key) || (record.rule ? { id: key, mode: 'STOP', name: record.rule, text: '' } : { id: key, mode: 'LAW', name: key, text: '' });
  const firstTs = Date.parse(first.ts);
  return {
    key, rule, count: cases.length, position, sameAction,
    first: { ts: Number.isNaN(firstTs) ? null : firstTs, source: String(first.source || '').toLowerCase(), digest16: String(first.digest || '').slice(0, 16), isThis: firstIndex === index },
    sources: Object.entries(sources).sort((a, b) => b[1] - a[1]),
    days: [...days.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1)),
    caseIndexes: cases,
  };
}

const TOOL_LABEL = (tool) => String(tool || '?').replace(/^mcp__([^_]+(?:_[^_]+)*)__/, 'mcp:').replace(/[^\w.:-]/g, '').slice(0, 18) || '?';

function aftermath(records, index, { limit = 40 } = {}) {
  const record = records[index];
  const session = record.session;
  const next = [];
  let calls = 0;
  let retried = 0;
  let retriedStopped = 0;
  if (session) {
    for (let i = index + 1; i < records.length; i++) {
      const r = records[i];
      if (r.session !== session) continue;
      if (!['PASS', 'STOP', 'SHADOW_STOP', 'ALLOW', 'SHADOW_TASTE'].includes(r.decision)) continue;
      calls++;
      const same = Boolean(record.action_digest) && r.action_digest === record.action_digest;
      if (same) { retried++; if (r.decision === 'STOP') retriedStopped++; }
      if (next.length < limit) next.push({ tool: TOOL_LABEL(r.tool), decision: r.decision, same, dt: Math.max(0, Math.round((Date.parse(r.ts) - Date.parse(record.ts)) / 1000)) });
    }
  }
  const later = [];
  if (record.action_digest) {
    for (let i = index + 1; i < records.length; i++) {
      const r = records[i];
      if (r.session === session || r.action_digest !== record.action_digest) continue;
      if (!['STOP', 'ALLOW', 'PASS', 'SHADOW_STOP'].includes(r.decision)) continue;
      later.push({ ts: Date.parse(r.ts), source: String(r.source || '').toLowerCase(), decision: r.decision, session8: String(r.session || '').slice(0, 8) });
    }
  }
  const sessions = new Set(later.map((l) => l.session8));
  return { session8: String(session || '').slice(0, 8), calls, next, retried, retriedStopped, later: later.slice(-12), laterCount: later.length, laterSessions: sessions.size };
}

// Every STOP the journey can tell, newest first, each with its index in the log.
function eligibleCases(log, { includeCanary = true, home, user } = {}) {
  const out = [];
  for (let i = log.records.length - 1; i >= 0; i--) {
    const record = log.records[i];
    if (record.decision !== 'STOP') continue;
    const s = story.storyFromReceipt(record, { includeCanary, home, user });
    if (s) out.push({ index: i, story: s });
  }
  return out;
}

function caseNumber(log, index) {
  let n = 0;
  for (let i = 0; i <= index; i++) if (log.records[i].decision === 'STOP') n++;
  return n;
}

function buildCase(log, index, { catalog = new Map(), permits = [], home, user, now = Date.now() } = {}) {
  const record = log.records[index];
  const s = story.storyFromReceipt(record, { includeCanary: true, home, user });
  if (!s) return null;
  // The chain link is checked from the log itself, not from file order alone.
  s.chainChecked = index > 0 && record.prev === log.records[index - 1].digest && Boolean(rederive(record));
  const law = caseLaw(log.records, index, catalog);
  const permit = findPermit(permits, record);
  const sinceDays = law.first.ts ? Math.min(365, Math.max(1, Math.ceil((now - law.first.ts) / DAY_MS) + 1)) : 30;
  return {
    index,
    story: s,
    number: caseNumber(log, index),
    totalStops: log.records.reduce((n, r) => n + (r.decision === 'STOP' ? 1 : 0), 0),
    position: index + 1,
    total: log.records.length,
    digest: record.digest,
    prev: record.prev,
    actionDigest: typeof record.action_digest === 'string' ? record.action_digest : '',
    session8: String(record.session || '').slice(0, 8),
    file: record._file,
    line: record._line,
    neighbours: [-3, -2, -1, 0, 1, 2].map((d) => log.records[index + d]).filter(Boolean).map((r) => ({ position: log.records.indexOf(r) + 1, digest16: String(r.digest || '').slice(0, 16), prev16: String(r.prev || '').slice(0, 16), decision: r.decision, ok: Boolean(rederive(r)) })),
    law,
    permit: permit ? { status: permit.status, granted: permit.granted, by: permit.by, singleUse: permit.single_use, named: permit.stop_receipt === record.digest } : null,
    permitsUsedForRule: log.records.filter((r) => r.decision === 'ALLOW' && r.via === 'permit' && ruleKeyOf(r) === law.key).length,
    aftermath: aftermath(log.records, index),
    precedent: { call: `precedent(rule="${law.key}", since="${sinceDays}d")`, answer: precedentSummary(log.records, { rule: law.key, sinceDays, now }) },
    recall: `recall(handle: "op-receipt-${String(record.digest).slice(0, 16)}")`,
  };
}

// Vestige memory, read through the local dashboard API. Loopback only: nothing leaves this machine.
function apiBase(env = process.env) {
  const raw = env.VESTIGE_API || 'http://127.0.0.1:3927';
  try {
    const url = new URL(raw);
    if (url.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]', '::1'].includes(url.hostname)) return null;
    return url;
  } catch { return null; }
}

// One deadline for the whole request: a server that never answers, or drips, gets null in time.
function getJSON(base, pathname, timeoutMs) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (value) => { if (!settled) { settled = true; clearTimeout(timer); resolve(value); } };
    const req = http.get({ hostname: base.hostname.replace(/^\[|\]$/g, ''), port: base.port || 80, path: pathname, timeout: timeoutMs, headers: { accept: 'application/json' } }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => { body += chunk; if (body.length > 2_000_000) { req.destroy(); done(null); } });
      res.on('end', () => { if (res.statusCode !== 200) return done(null); try { done(JSON.parse(body)); } catch { done(null); } });
      res.on('error', () => done(null));
    });
    const timer = setTimeout(() => { req.destroy(); done(null); }, timeoutMs);
    req.on('timeout', () => { req.destroy(); done(null); });
    req.on('error', () => done(null));
    req.on('close', () => done(null));
  });
}

// Only what reads as a memory gets through: an id of the right shape, strengths between 0 and 1.
const unit = (v) => { const n = Number(v); return Number.isFinite(n) && n >= 0 && n <= 1 ? n : null; };
const memoryView = (m) => (m && typeof m === 'object' && /^(mem-[0-9a-f]{16}|[0-9a-f-]{36})$/.test(String(m.id || ''))
  ? { id: String(m.id), type: String(m.nodeType || '').replace(/[^\w-]/g, '').slice(0, 16), retention: unit(m.retentionStrength), retrieval: unit(m.retrievalStrength), storage: unit(m.storageStrength), reviews: Number(m.reviewCount) || 0, createdAt: Date.parse(m.createdAt) || null, digestAgrees: null, content: String(m.content || '') }
  : null);

async function memoryOf(theCase, { env = process.env, timeoutMs = 1500 } = {}) {
  const base = apiBase(env);
  if (!base) return { reachable: false, reason: 'not-loopback' };
  const d16 = String(theCase.digest).slice(0, 16);
  const tag = (t) => `/api/memories?tag=${encodeURIComponent(t)}&limit=1`;
  const [health, stop, rule, under] = await Promise.all([
    getJSON(base, '/api/health', timeoutMs),
    getJSON(base, tag(`op-receipt-${d16}`), timeoutMs),
    getJSON(base, tag(`op-rule-${theCase.law.key}`), timeoutMs),
    getJSON(base, tag(theCase.law.key), timeoutMs),
  ]);
  if (!health) return { reachable: false, reason: 'no-answer', where: `${base.hostname}:${base.port || 80}` };
  const stopMemory = memoryView(stop && Array.isArray(stop.memories) ? stop.memories[0] : null);
  // A second, independent record: the memory quotes the receipt digest it was written from.
  if (stopMemory) stopMemory.digestAgrees = stopMemory.content.includes(d16);
  if (stopMemory) delete stopMemory.content;
  const ruleMemory = memoryView(rule && Array.isArray(rule.memories) ? rule.memories[0] : null);
  if (ruleMemory) delete ruleMemory.content;
  return {
    reachable: true,
    where: `${base.hostname}:${base.port || 80}`,
    total: Number.isFinite(Number(health.totalMemories)) ? Number(health.totalMemories) : null,
    version: String(health.version || '').replace(/[^\w.-]/g, '').slice(0, 16),
    stop: stopMemory,
    rule: ruleMemory,
    rememberedUnderRule: under && Number.isFinite(Number(under.total)) && Number(under.total) >= 0 ? Number(under.total) : null,
  };
}

// Plain text, for --plain, non-terminals, and the case file left in scrollback.
const num2 = (v) => (Number.isFinite(v) ? v.toFixed(2) : 'unknown');

function caseFile(c, chain, memory, code = '') {
  const s = c.story;
  const when = new Date(s.ts);
  const clock = `${String(when.getHours()).padStart(2, '0')}:${String(when.getMinutes()).padStart(2, '0')}`;
  const mem = !memory ? 'not checked' : !memory.reachable ? 'Vestige memory did not answer on this machine' : memory.stop
    ? `${memory.stop.id} (retention ${num2(memory.stop.retention)})` + (memory.rule ? ` · derived from rule memory ${memory.rule.id} (${num2(memory.rule.retention)})` : '')
    : 'not in memory: the stop reflex records Claude Code and ZCode stops';
  const after = c.aftermath;
  const afterLine = !after.session8 ? 'no session id on this receipt'
    : after.retried ? `${after.calls} more calls in that session · it tried the same action ${after.retried} more time${after.retried === 1 ? '' : 's'}${after.retriedStopped === after.retried ? ', stopped every time' : ''}`
      : `${after.calls} more call${after.calls === 1 ? '' : 's'} in that session · it never tried that action again`;
  const permit = c.permit ? `${c.permit.status} permit (owner said yes once)` : 'none';
  return [
    `VESTIGE v5 · PRECEDENT · case ${c.number.toLocaleString('en-US')} on this machine`,
    '',
    `1 THE STOP        ${s.source} tried: ${s.action}`,
    `                  ${clock} · stopped by ${s.commitment}${s.rule ? ' ' + s.rule : ''} · nothing was run`,
    `2 THE RECORD      ${chain ? `${chain.rederived.toLocaleString('en-US')} of ${chain.total.toLocaleString('en-US')} receipts re-derived · ${chain.breaks.length} broken · ${chain.forkCount} fork${chain.forkCount === 1 ? '' : 's'} · ${(chain.ms / 1000).toFixed(2)} s` : 'not checked'}`,
    `                  receipt ${s.digest} ← ${s.prev || 'start'}`,
    `3 THE MEMORY      ${mem}`,
    `4 THE PRECEDENT   case ${c.law.position} of ${c.law.count} under ${c.law.key} · first ${c.law.first.ts ? new Date(c.law.first.ts).toDateString().slice(4) : 'unknown'} · permit: ${permit}`,
    `                  ${c.precedent.call} → "${c.precedent.answer}"`,
    `5 THE AFTERMATH   ${afterLine}`,
    `6 THE VERDICT     a remembered yes is not permission today.`,
    ...(code ? [`                  replay: npx @vestige/init play ${code}`] : []),
  ].join('\n');
}

module.exports = {
  PERMIT_NOTE, GENESIS, operatorHome, canonical, digestOf, rederive, readLog, verifyChain, forge, ruleCatalog,
  loadPermits, findPermit, matchesRule, precedentSummary, caseLaw, aftermath, eligibleCases, caseNumber, buildCase,
  apiBase, memoryOf, caseFile,
};

if (require.main === module) {
  (async () => {
    const home = operatorHome();
    const log = readLog(home);
    const chain = await verifyChain(log);
    const cases = eligibleCases(log);
    if (!cases.length) { console.log('no cases yet: they appear after Operator stops something one of your agents tried'); return; }
    const pick = process.argv[2] ? cases.find((c) => c.story.digest.startsWith(process.argv[2])) || cases[Number(process.argv[2]) - 1] : cases[0];
    const c = buildCase(log, pick.index, { catalog: ruleCatalog(home), permits: loadPermits(home) });
    const memory = await memoryOf(c);
    console.log(caseFile(c, chain, memory));
    if (process.argv.includes('--json')) console.log(JSON.stringify({ case: c, chain, memory }, null, 1));
  })();
}
