#!/usr/bin/env node

const fs = require('fs');
const os = require('os');
const path = require('path');

const DISGUISES = ['variables-expanded', 'heredoc-extracted', 'command-substitution-analyzed', 'pipe-sink-analyzed', 'base64-decoded'];
const DISGUISE_LABELS = {
  'variables-expanded': 'variables',
  'heredoc-extracted': 'heredoc',
  'command-substitution-analyzed': 'substitution',
  'pipe-sink-analyzed': 'pipe',
  'base64-decoded': 'base64',
};
const BASE_SOURCES = ['claude', 'cursor', 'codex', 'gemini', 'opencode', 'zcode', 'windsurf', 'cline', 'kiro', 'zed', 'amp', 'goose', 'openclaw', 'copilot', 'antigravity', 'lmstudio', 'continue', 'aider', 'crush', 'warp'];
const MAX_ACTION = 80;
const MAX_COUNT = 4294967295;

function allowedSources(env = process.env) {
  const extra = String(env.VESTIGE_REM_SOURCES || '').split(',').map((s) => s.trim().toLowerCase()).filter((s) => /^[a-z0-9-]{1,24}$/.test(s));
  return new Set([...BASE_SOURCES, ...extra]);
}

function receiptsDir(env = process.env) {
  return path.join(env.OPERATOR_HOME || path.join(os.homedir(), '.operator'), 'receipts');
}

function readReceipts(dir = receiptsDir(), { maxFiles = 120 } = {}) {
  let files;
  try { files = fs.readdirSync(dir).filter((name) => name.endsWith('.jsonl')).sort().slice(-maxFiles); } catch { return []; }
  const records = [];
  let prevDigest = '';
  for (const file of files) {
    let text;
    try { text = fs.readFileSync(path.join(dir, file), 'utf8'); } catch { continue; }
    for (const line of text.split('\n')) {
      if (!line.trim()) continue;
      let record;
      try { record = JSON.parse(line); } catch { prevDigest = ''; continue; }
      if (!record || typeof record !== 'object' || Array.isArray(record)) { prevDigest = ''; continue; }
      Object.defineProperty(record, '_prevDigest', { value: prevDigest, enumerable: false });
      prevDigest = typeof record.digest === 'string' ? record.digest : '';
      records.push(record);
    }
  }
  return records;
}

// Checked before any rewriting. A match refuses the whole card; the reason names the class only.
const SECRET_SHAPES = [
  ['aws-key', /AKIA[0-9A-Z]{16}/],
  ['github-token', /\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})/],
  ['api-key', /\bsk-[A-Za-z0-9_-]{20,}/],
  ['slack-token', /\bxox[baprs]-[A-Za-z0-9-]{10,}/],
  ['private-key', /-----BEGIN/],
  ['jwt', /\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}/],
  ['bearer', /\bbearer\s+[A-Za-z0-9._~+/=-]{8,}/i],
  ['auth-header', /\bauthorization\s*[:=]/i],
  ['credential-flag', /--(?:password|passwd|secret|token|api-?key)(?:=|\s+)\S+/i],
  ['credential-assignment', /(?:pass(?:word|wd)?|secret|token|api[_-]?key|access[_-]?key)["']?\s*[:=]\s*["']?[^\s"']{3,}/i],
  ['url-credentials', /\b[a-z][a-z0-9+.-]*:\/\/[^\s/@:]+:[^\s/@]+@/i],
];

const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

function collapsePath(lead, rest) {
  const base = rest.slice(rest.lastIndexOf('/') + 1);
  return `${lead}/…/${base}`;
}

function redactAction(input, { home = os.homedir(), user = os.userInfo().username } = {}) {
  let text = String(input == null ? '' : input);
  text = text.replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?/g, ' ').replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, ' ');
  text = text.replace(/[\x00-\x1f\x7f-\x9f​-‏‪-‮⁠-⁩﻿]/g, ' ').replace(/\s+/g, ' ').trim();
  for (;;) {
    const next = text.replace(/^cd\s+(?:"[^"]*"|'[^']*'|\S+)\s*(?:&&|;|$)\s*/, '');
    if (next === text) break;
    text = next;
  }
  for (const [reason, shape] of SECRET_SHAPES) if (shape.test(text)) return { refused: true, reason };
  text = text.replace(/\b([a-z][a-z0-9+.-]*:\/\/)([^\s/"'?#]+)([^\s"']*)/gi, (_m, scheme, host, tail) => (tail ? `${scheme}${host}/…` : scheme + host));
  text = text.replace(/[\w.+-]+@[\w-]+(?:\.[\w-]+)+/g, '<email>');
  text = text.replace(/\b(?:\d{1,3}\.){3}\d{1,3}\b/g, '<ip>');
  if (home && home.length > 1) text = text.split(home).join('~');
  text = text.replace(/\/(?:Users|home)\/[^/\s"']+/g, '~');
  if (user && user.length > 2) text = text.replace(new RegExp(`(^|[^A-Za-z0-9_])${escapeRegExp(user)}(?![A-Za-z0-9_])`, 'g'), '$1user');
  text = text.replace(/(["'])((?:~|\.{1,2})?)((?:\/[^"'/]+){3,})\1/g, (_m, quote, lead, rest) => quote + collapsePath(lead, rest) + quote);
  text = text.replace(/(^|[\s=:(,])((?:~|\.{1,2}|[\w.-]+)?)((?:\/[^\s"'/:;|&<>()]+){3,})/g, (_m, pre, lead, rest) => pre + collapsePath(lead, rest));
  text = text.replace(/[A-Za-z0-9+/_=-]{24,}/g, (run) => (/[A-Za-z]/.test(run) && /\d/.test(run) ? '<redacted>' : run));
  text = [...text].map((ch) => (ch === '…' || (ch >= ' ' && ch <= '~') ? ch : '?')).join('').replace(/\s+/g, ' ').trim();
  if (!text) return { refused: true, reason: 'empty' };
  const chars = [...text];
  if (chars.length > MAX_ACTION) text = chars.slice(0, MAX_ACTION - 1).join('').trimEnd() + '…';
  return { refused: false, text };
}

const count = (value) => Math.max(0, Math.min(MAX_COUNT, Math.floor(Number(value)) || 0));

function evaluate(record, options = {}) {
  if (!record || record.decision !== 'STOP') return { reason: 'not-a-stop' };
  const source = String(record.source || '').toLowerCase();
  if (!(options.sources || allowedSources()).has(source)) return { reason: 'source' };
  const commitment = String(record.commitment || '');
  // The canary is the gate's own harmless self-test, so its smoke-test sessions count when it is asked for.
  if (commitment === 'OP-CANARY' && !options.includeCanary) return { reason: 'canary' };
  if (commitment !== 'OP-CANARY' && /^vestige-smoke-/.test(String(record.session || ''))) return { reason: 'smoke-session' };
  if (!/^[A-Za-z0-9-]{1,32}$/.test(commitment)) return { reason: 'commitment' };
  const digest = String(record.digest || '');
  if (!/^[a-f0-9]{64}$/.test(digest)) return { reason: 'digest' };
  const ts = Date.parse(record.ts);
  if (Number.isNaN(ts)) return { reason: 'timestamp' };
  const redacted = redactAction(record.action_preview || record.normalized || '', options);
  if (redacted.refused) return { reason: 'redaction:' + redacted.reason };
  const prev = /^[a-f0-9]{64}$/.test(String(record.prev || '')) ? record.prev : '';
  const rule = String(record.rule || '');
  const transforms = Array.isArray(record.transforms) ? record.transforms : [];
  const loss = record.loss && typeof record.loss === 'object'
    ? { files: count(record.loss.files), unrecoverable: count(record.loss.unrecoverable), capped: record.loss.capped === true }
    : null;
  return {
    story: {
      v: 1,
      kind: 'true',
      shared: false,
      source,
      commitment,
      rule: /^[a-z0-9-]{0,40}$/.test(rule) ? rule : '',
      action: redacted.text,
      disguises: DISGUISES.filter((name) => transforms.includes(name)),
      loss,
      ts: Math.floor(ts / 60000) * 60000,
      digest: digest.slice(0, 16),
      prev: prev.slice(0, 16),
      chainChecked: Boolean(prev) && prev === record._prevDigest,
      tool: String(record.tool || '').replace(/[^\w.-]/g, '').slice(0, 24),
    },
  };
}

const storyFromReceipt = (record, options) => evaluate(record, options).story || null;
const whyNot = (record, options) => evaluate(record, options).reason || '';

function listStories({ dir, sources, includeCanary, limit = Infinity, home, user } = {}) {
  const stories = [];
  const records = readReceipts(dir);
  for (let i = records.length - 1; i >= 0 && stories.length < limit; i--) {
    const story = storyFromReceipt(records[i], { sources, includeCanary, home, user });
    if (story) stories.push(story);
  }
  return stories;
}

const latestStory = (options = {}) => listStories({ ...options, limit: 1 })[0] || null;

function localStats(stories) {
  const stats = { total: stories.length, byCommitment: {}, bySource: {}, byDisguise: {}, firstTs: {} };
  for (const story of stories) {
    stats.byCommitment[story.commitment] = (stats.byCommitment[story.commitment] || 0) + 1;
    stats.bySource[story.source] = (stats.bySource[story.source] || 0) + 1;
    for (const name of story.disguises) stats.byDisguise[name] = (stats.byDisguise[name] || 0) + 1;
    if (!(story.commitment in stats.firstTs) || story.ts < stats.firstTs[story.commitment]) stats.firstTs[story.commitment] = story.ts;
  }
  return stats;
}

function permitsFor(dir) {
  return readReceipts(dir)
    .filter((record) => record.decision === 'PERMIT_GRANTED' && /^[a-f0-9]{16,64}$/.test(String(record.stop_receipt || '')))
    .map((record) => ({ stopDigest16: record.stop_receipt.slice(0, 16), ts: Date.parse(record.ts) || 0 }));
}

function statePath(env = process.env) {
  return path.join(env.VESTIGE_REM_HOME || path.join(os.homedir(), '.vestige', 'rem'), 'state.json');
}

function loadState(env = process.env) {
  try {
    const state = JSON.parse(fs.readFileSync(statePath(env), 'utf8'));
    if (state && typeof state.seen === 'object' && state.seen && !Array.isArray(state.seen)) return { v: 1, seen: state.seen };
  } catch {}
  return { v: 1, seen: {} };
}

const isSeen = (digest16, env = process.env) => Object.prototype.hasOwnProperty.call(loadState(env).seen, digest16);

function markSeen(digest16, env = process.env) {
  if (!/^[a-f0-9]{16}$/.test(String(digest16))) return false;
  const file = statePath(env);
  const state = loadState(env);
  if (state.seen[digest16]) return true;
  state.seen[digest16] = Date.now();
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const draft = `${file}.${process.pid}.draft`;
    fs.writeFileSync(draft, JSON.stringify(state));
    fs.renameSync(draft, file);
    return true;
  } catch { return false; }
}

function printStats(dir = receiptsDir()) {
  const records = readReceipts(dir);
  const reasons = {};
  const stories = [];
  for (const record of records) {
    if (record.decision !== 'STOP') continue;
    const result = evaluate(record);
    if (result.story) stories.push(result.story);
    else reasons[result.reason] = (reasons[result.reason] || 0) + 1;
  }
  const stats = localStats(stories);
  console.log(`records read        ${records.length}`);
  console.log(`STOP records        ${records.filter((r) => r.decision === 'STOP').length}`);
  console.log(`eligible stories    ${stories.length}`);
  console.log(`chain link checked  ${stories.filter((s) => s.chainChecked).length}`);
  console.log(`left out, by reason ${JSON.stringify(reasons)}`);
  console.log(`by commitment       ${JSON.stringify(stats.byCommitment)}`);
  console.log(`by source           ${JSON.stringify(stats.bySource)}`);
  console.log(`by disguise         ${JSON.stringify(stats.byDisguise)}`);
}

if (require.main === module) {
  if (process.argv.includes('--stats')) printStats();
  else console.log('usage: node vestige-story.js --stats   (counts only; command text is never printed)');
}

module.exports = {
  DISGUISES, DISGUISE_LABELS, BASE_SOURCES, MAX_ACTION, allowedSources, receiptsDir, readReceipts, redactAction,
  storyFromReceipt, whyNot, listStories, latestStory, localStats, permitsFor, statePath, loadState, isSeen, markSeen,
};
