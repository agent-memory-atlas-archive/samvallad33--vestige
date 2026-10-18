const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const p = require('../lib/vestige-precedent.js');

const OUT = path.join(require('os').tmpdir(), 'vestige-init-tests', 'precedent-tests', `run-${Date.now()}`);

// Digests computed by Python's json.dumps(sort_keys=True, separators=(',',':'), ensure_ascii=...), as the gate does.
const A = { v: 1, ts: '2026-10-18T08:17:27Z', decision: 'STOP', source: 'claude', session: 's1', tool: 'Bash', action_preview: 'rm -rf ~/work', commitment: 'OP-001', rule: 'protect-workspaces', transforms: ['variables-expanded'], loss: { files: 3, bytes: 10, unrecoverable: 1, capped: false }, prev: '0'.repeat(64), integrity: 'reference_digest_not_signature' };
const A_DIGEST = '0bdd2d3051071c2a9f44566c452e91f7e4016205bdd27200a8d125e4aab6eb57';
const B = { v: 1, ts: '2026-10-18T08:17:30Z', decision: 'PASS', source: 'claude', session: 's1', tool: 'Bash', action_preview: 'echo café ✓', commitments: [], prev: A_DIGEST };
const B_ASCII = 'cee0069c76bd5146b3dfa9b33c9418f2d58a7b95fae0782668991da44a8001ed';
const B_RAW = '791ceb9c86d0f1f60b8fd972c56cf5fdbc4f78f70f9486e8639562612d6a82d2';

test('digests match the gate byte for byte, escaped and raw UTF-8', () => {
  assert.equal(p.digestOf(A), A_DIGEST);
  assert.equal(p.digestOf(B, true), B_ASCII);
  assert.equal(p.digestOf(B, false), B_RAW);
  assert.equal(p.rederive({ ...A, digest: A_DIGEST }), 'ascii');
  assert.equal(p.rederive({ ...B, digest: B_RAW }), 'utf8');
  assert.equal(p.rederive({ ...B, digest: B_ASCII }), 'ascii');
  assert.equal(p.rederive({ ...A, digest: A_DIGEST, decision: 'PASS' }), null);
});

// A log written the gate's way: each record sealed, each prev naming the record before it.
function sealedLog(n, { forkAt = -1, breakAt = -1 } = {}) {
  const records = [];
  let prev = '0'.repeat(64);
  for (let i = 0; i < n; i++) {
    const r = { v: 1, ts: new Date(Date.UTC(2026, 9, 18, 8, 0, i)).toISOString().replace('.000', ''), decision: i % 5 === 2 ? 'STOP' : 'PASS', source: 'claude', session: i < n / 2 ? 'sA' : 'sB', tool: 'Bash', action_preview: `echo ${i}`, action_digest: i % 5 === 2 ? 'ad-same' : `ad-${i}`, commitment: i % 5 === 2 ? 'OP-011' : undefined, rule: i % 5 === 2 ? 'no-unrestorable-loss' : undefined, prev };
    for (const k of Object.keys(r)) if (r[k] === undefined) delete r[k];
    if (i === forkAt) r.prev = records[i - 2].digest;
    if (i === breakAt) r.prev = 'f'.repeat(64);
    r.digest = p.digestOf(r);
    records.push(r);
    prev = r.digest;
  }
  return { records, head: prev, files: ['2026-10-18.jsonl'] };
}

test('verifyChain counts links, forks and breaks, and checks HEAD', async () => {
  const clean = await p.verifyChain(sealedLog(40));
  assert.equal(clean.total, 40);
  assert.equal(clean.rederived, 40);
  assert.equal(clean.linked, 39);
  assert.equal(clean.forkCount, 0);
  assert.equal(clean.breaks.length, 0);
  assert.equal(clean.genesis, true);
  assert.equal(clean.headMatches, true);
  const forked = await p.verifyChain(sealedLog(40, { forkAt: 10 }));
  assert.equal(forked.forks.length, 1);
  assert.equal(forked.forks[0].index, 10);
  assert.equal(forked.forkCount, 1);
  const broken = await p.verifyChain(sealedLog(40, { breakAt: 20 }));
  assert.equal(broken.breaks.length, 1);
  assert.equal(broken.breaks[0].index, 20);
  const progress = [];
  await p.verifyChain(sealedLog(40), { slice: 7, onProgress: (r) => progress.push(r.checked) });
  assert.deepEqual(progress, [7, 14, 21, 28, 35, 40]);
});

test('forge runs on a copy and shows each step the chain catches', () => {
  const log = sealedLog(12);
  const before = JSON.stringify(log.records);
  const r = p.forge(log, 7);
  assert.equal(r.step1.from, 'STOP');
  assert.equal(r.step1.to, 'PASS');
  assert.equal(r.step1.digestStillMatches, false);
  assert.equal(r.step2.nextIndex, 8);
  assert.equal(r.step2.nextStillLinks, false);
  assert.equal(r.step3.laterRecords, 4);
  assert.equal(JSON.stringify(log.records), before, 'the log itself is untouched');
  assert.equal(p.forge(log, 11).step2.nextIndex, null);
});

test('permits pair the way the precedent tool pairs them', () => {
  const permits = [
    { action_digest: 'x', status: 'used', granted: 100.4, stop_receipt: null },
    { action_digest: 'x', status: 'used', granted: 200, stop_receipt: 'd-named' },
    { action_digest: 'x', status: 'active', granted: 300.9, stop_receipt: null },
  ];
  const at = (s) => new Date(s * 1000).toISOString();
  assert.equal(p.findPermit(permits, { action_digest: 'x', digest: 'd-named', decision: 'STOP', ts: at(50) }).granted, 200, 'a permit naming the stop wins');
  assert.equal(p.findPermit(permits, { action_digest: 'x', digest: 'd1', decision: 'STOP', ts: at(90) }).granted, 100.4, 'a stop takes the earliest later permit naming no other stop');
  assert.equal(p.findPermit(permits, { action_digest: 'x', digest: 'd2', decision: 'STOP', ts: at(150) }).granted, 300.9, 'skips the one naming another stop');
  assert.equal(p.findPermit(permits, { action_digest: 'x', digest: 'd3', decision: 'ALLOW', ts: at(250) }).granted, 200, 'an ALLOW takes the latest permit at or before it');
  assert.equal(p.findPermit(permits, { action_digest: 'y', digest: 'd4', decision: 'STOP', ts: at(90) }), null);
  assert.equal(p.findPermit(permits, { digest: 'd5', decision: 'STOP', ts: at(90) }), null);
});

test('precedentSummary speaks the tool\'s words', () => {
  const now = Date.parse('2026-10-18T09:00:00Z');
  const records = [
    { decision: 'STOP', commitment: 'OP-011', ts: '2026-10-18T08:00:00Z' },
    { decision: 'STOP', rule: 'no-unrestorable-loss', commitment: 'OP-011', ts: '2026-10-17T08:00:00Z' },
    { decision: 'ALLOW', via: 'permit', commitments: ['OP-011'], ts: '2026-10-18T08:30:00Z' },
    { decision: 'STOP', commitment: 'OP-001', ts: '2026-10-18T08:00:00Z' },
    { decision: 'STOP', commitment: 'OP-011', ts: '2026-09-01T08:00:00Z' },
  ];
  assert.equal(p.precedentSummary(records, { rule: 'OP-011', sinceDays: 3, now }), '2 stops in the last 3d, 1 permit used');
  assert.equal(p.precedentSummary(records, { rule: 'op-001', sinceDays: 1, now }), '1 stop in the last 1d');
});

test('a case: case law, aftermath and the calls an agent can make', () => {
  const log = sealedLog(30);
  const index = 17; // a STOP in session sB
  const c = p.buildCase(log, index, { home: '/Users/nobody', user: 'nobody', now: Date.parse('2026-10-18T09:00:00Z') });
  assert.equal(c.story.decision, undefined);
  assert.equal(c.law.key, 'OP-011');
  assert.equal(c.law.count, 6);
  assert.equal(c.law.position, 4);
  assert.equal(c.law.sameAction, 6);
  assert.equal(c.story.chainChecked, true);
  assert.match(c.precedent.call, /^precedent\(rule="OP-011", since="\d+d"\)$/);
  assert.equal(c.recall, `recall(handle: "op-receipt-${log.records[index].digest.slice(0, 16)}")`);
  assert.equal(c.aftermath.session8, 'sB');
  assert.equal(c.aftermath.calls, 12);
  assert.equal(c.aftermath.retried, 2);
  assert.equal(c.aftermath.retriedStopped, 2);
  assert.equal(c.aftermath.laterCount, 0, 'later sessions only count other sessions');
  const text = p.caseFile(c, { total: 30, rederived: 30, breaks: [], forkCount: 0, ms: 5 }, { reachable: false });
  assert.match(text, /case 4 of 6 under OP-011/);
  assert.match(text, /did not answer/);
  assert.doesNotMatch(text, /undefined|NaN/);
});

test('readLog reads only day files, in order, and the rule catalog reads owner laws', () => {
  const home = path.join(OUT, 'home');
  fs.mkdirSync(path.join(home, 'receipts'), { recursive: true });
  const log = sealedLog(6);
  fs.writeFileSync(path.join(home, 'receipts', '2026-10-18.jsonl'), log.records.slice(3).map((r) => JSON.stringify(r)).join('\n') + '\n');
  fs.writeFileSync(path.join(home, 'receipts', '2026-10-17.jsonl'), log.records.slice(0, 3).map((r) => JSON.stringify(r)).join('\n') + '\nnot json\n');
  fs.writeFileSync(path.join(home, 'receipts', 'notes.jsonl'), '{"decision":"STOP"}\n');
  fs.writeFileSync(path.join(home, 'receipts', 'HEAD'), log.head + '\n');
  fs.writeFileSync(path.join(home, 'commitments.json'), JSON.stringify({ commitments: [{ commitment_id: 'no-prod-deploys', state: 'active', directive: 'Production deploys are owner-only.' }] }));
  const read = p.readLog(home);
  assert.equal(read.records.length, 6);
  assert.equal(read.unreadable, 1);
  assert.equal(read.head, log.head);
  assert.equal(read.records[0]._file, '2026-10-17.jsonl');
  const catalog = p.ruleCatalog(home);
  assert.equal(catalog.get('no-prod-deploys').text, 'Production deploys are owner-only.');
  assert.equal(catalog.get('no-prod-deploys').mode, 'LAW');
});

test('memory is only ever asked of loopback', () => {
  assert.equal(p.apiBase({}).hostname, '127.0.0.1');
  assert.equal(p.apiBase({ VESTIGE_API: 'http://localhost:4000' }).port, '4000');
  assert.equal(p.apiBase({ VESTIGE_API: 'https://example.com' }), null);
  assert.equal(p.apiBase({ VESTIGE_API: 'http://10.0.0.5:3927' }), null);
  assert.equal(p.apiBase({ VESTIGE_API: 'not a url' }), null);
});

test('an unreachable memory answers quickly and plainly', async () => {
  const started = Date.now();
  const m = await p.memoryOf({ digest: 'a'.repeat(64), law: { key: 'OP-001' } }, { env: { VESTIGE_API: 'http://127.0.0.1:9' }, timeoutMs: 400 });
  assert.equal(m.reachable, false);
  assert.ok(Date.now() - started < 2000);
});
