const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const story = require('../lib/vestige-story.js');

const OUT = path.join(require('os').tmpdir(), 'vestige-init-tests', 'story', `fixtures-${Date.now()}`);
const HOME = '/Users/samplehome';
const USER = 'samplehome';
const red = (text) => story.redactAction(text, { home: HOME, user: USER });
const hex = (seed) => seed.repeat(64).slice(0, 64);

function writeLog(name, records) {
  const dir = path.join(OUT, name, 'receipts');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, '2026-10-11.jsonl'), records.map((r) => (typeof r === 'string' ? r : JSON.stringify(r))).join('\n') + '\n');
  return dir;
}

const base = (over = {}) => ({
  decision: 'STOP', ts: '2026-10-12T03:52:20Z', source: 'claude', session: 's1', tool: 'Bash',
  action_preview: 'rm -rf ~', commitment: 'OP-001', rule: 'protect-workspaces', transforms: [],
  digest: hex('a1'), prev: hex('b2'), ...over,
});

test('only real stops from interactive agents become stories', () => {
  const dir = writeLog('eligibility', [
    base({ digest: hex('01'), prev: hex('00') }),
    base({ decision: 'SHADOW_STOP', digest: hex('02'), prev: hex('01') }),
    base({ decision: 'PASS', digest: hex('03'), prev: hex('02') }),
    base({ decision: 'PERMIT_GRANTED', stop_receipt: hex('01'), digest: hex('04'), prev: hex('03') }),
    base({ source: 'langchain', digest: hex('05'), prev: hex('04') }),
    base({ session: 'vestige-smoke-gemini-canary', digest: hex('06'), prev: hex('05') }),
    base({ commitment: 'OP-CANARY', rule: 'canary', action_preview: 'echo CANARY', digest: hex('07'), prev: hex('06') }),
    base({ digest: 'short', prev: hex('07') }),
    base({ ts: 'not a date', digest: hex('09'), prev: hex('08') }),
  ]);
  const stories = story.listStories({ dir, home: HOME, user: USER });
  assert.equal(stories.length, 1);
  assert.equal(stories[0].digest, hex('01').slice(0, 16));
  assert.equal(story.listStories({ dir, includeCanary: true, home: HOME, user: USER }).length, 2);
  const smoke = writeLog('canary-smoke', [base({ commitment: 'OP-CANARY', rule: 'canary', session: 'vestige-smoke-gemini-canary', action_preview: 'echo CANARY' })]);
  assert.equal(story.listStories({ dir: smoke }).length, 0);
  assert.equal(story.listStories({ dir: smoke, includeCanary: true })[0].commitment, 'OP-CANARY', 'a canary from a smoke session counts when asked for');
  const records = story.readReceipts(dir);
  assert.deepEqual(records.slice(1, 9).map((r) => story.whyNot(r)), ['not-a-stop', 'not-a-stop', 'not-a-stop', 'source', 'smoke-session', 'canary', 'digest', 'timestamp']);
  assert.deepEqual(story.permitsFor(dir), [{ stopDigest16: hex('01').slice(0, 16), ts: Date.parse('2026-10-12T03:52:20Z') }]);
});

test('story fields follow the contract', () => {
  const dir = writeLog('fields', [
    base({ digest: hex('aa'), prev: hex('99') }),
    base({
      digest: hex('bb'), prev: hex('aa'), source: 'Cursor', ts: '2026-10-09T09:14:59Z',
      transforms: ['base64-decoded', 'unknown', 'variables-expanded'],
      loss: { files: 41208, bytes: 9, unrecoverable: 12, capped: true },
    }),
    base({ digest: hex('cc'), prev: hex('00'), commitment: 'recall-before-grep', rule: undefined, loss: { files: -5, unrecoverable: 1e20 } }),
    '{ broken json',
    base({ digest: hex('dd'), prev: hex('cc') }),
  ]);
  const [last, third, second, first] = story.listStories({ dir, home: HOME, user: USER });
  assert.equal(first.chainChecked, false, 'the first record has nothing before it to check against');
  assert.equal(second.chainChecked, true);
  assert.equal(second.source, 'cursor');
  assert.deepEqual(second.disguises, ['variables-expanded', 'base64-decoded']);
  assert.deepEqual(second.loss, { files: 41208, unrecoverable: 12, capped: true });
  assert.equal(second.ts, Date.parse('2026-10-09T09:14:00Z'));
  assert.equal(second.prev, hex('aa').slice(0, 16));
  assert.equal(third.chainChecked, false, 'prev does not match the record before it');
  assert.equal(third.rule, '');
  assert.deepEqual(third.loss, { files: 0, unrecoverable: 4294967295, capped: false });
  assert.equal(last.chainChecked, false, 'a broken line breaks the link');
  for (const s of [first, second, third, last]) {
    assert.match(s.source, /^[a-z0-9-]{1,24}$/);
    assert.match(s.commitment, /^[A-Za-z0-9-]{1,32}$/);
    assert.match(s.rule, /^[a-z0-9-]{0,40}$/);
    assert.match(s.action, /^[\x20-\x7e…]{1,80}$/);
    assert.match(s.digest, /^[a-f0-9]{16}$/);
  }
  const stats = story.localStats([first, second, third, last]);
  assert.equal(stats.total, 4);
  assert.equal(stats.byCommitment['OP-001'], 3);
  assert.equal(stats.byDisguise['base64-decoded'], 1);
  assert.equal(stats.firstTs['OP-001'], second.ts);
});

test('redaction rewrites paths, hosts and identities', () => {
  assert.equal(red('rm -rf /Users/samplehome').text, 'rm -rf ~');
  assert.equal(red('cat /Users/samplehome/Developer/vestige/src/main.rs').text, 'cat ~/…/main.rs');
  assert.equal(red('cat /home/other/notes.txt').text, 'cat ~/notes.txt');
  assert.equal(red('ls /usr/local/lib/node_modules').text, 'ls /…/node_modules');
  assert.equal(red('cd "/Users/samplehome/Downloads/Vestige Wizard" && grep -n x a.js').text, 'grep -n x a.js');
  assert.equal(red("cd /a && cd 'b c' ; ls").text, 'ls');
  assert.equal(red('cat "/Users/samplehome/My Files/deep/inner/report final.txt"').text, 'cat "~/…/report final.txt"');
  assert.equal(red('curl -s https://example.com/a/b?c=1 -o out').text, 'curl -s https://example.com/… -o out');
  assert.equal(red('git push https://example.com').text, 'git push https://example.com');
  assert.equal(red('mail -s hi person@example.com').text, 'mail -s hi <email>');
  assert.equal(red('ssh 10.0.0.12 uptime').text, 'ssh <ip> uptime');
  assert.equal(red('chown samplehome file').text, 'chown user file');
  assert.equal(red('git checkout 3f2a9c1b7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a').text, 'git checkout <redacted>');
  assert.equal(red('echo café 你好 …').text, 'echo caf? ?? …');
  assert.ok(!red('ls /Users/samplehome/x').text.includes('samplehome'));
});

test('redaction strips control bytes and cuts at 80', () => {
  assert.equal(red('echo a\x1b[31mb\x1b]52;c;QUJD\x07c\x1b]8;;http://x\x1b\\d‮e​f\x00g\r\nh').text, 'echo a b c d e f g h');
  const long = red('echo ' + 'word '.repeat(40));
  assert.equal([...long.text].length, 80);
  assert.ok(long.text.endsWith('…'));
  assert.equal([...red('x'.repeat(80)).text].length, 80);
  assert.ok(!red('x'.repeat(80)).text.endsWith('…'));
  assert.deepEqual(red('cd /somewhere'), { refused: true, reason: 'empty' });
  assert.deepEqual(red('   '), { refused: true, reason: 'empty' });
  assert.deepEqual(red('ls -la'), red('ls -la'));
});

test('a secret shape refuses the card and the reason never repeats it', () => {
  const cases = {
    'aws-key': ['aws configure set aws_access_key_id AKIAIOSFODNN7EXAMPLE', 'echo AKIAABCDEFGHIJKLMNOP'],
    'github-token': ['git clone https://ghp_abcdefghijklmnopqrstuvwxyz0123456789@github.com/a/b', 'echo github_pat_11ABCDEFG0abcdefghijkl_mnop'],
    'api-key': ['export OPENAI=sk-proj-abcdefghijklmnopqrstuvwx', 'echo sk-ant-api03-abcdefghijklmnopqrstu'],
    'slack-token': ['curl -d xoxb-1234567890-abcdefghij', 'echo xoxp-0987654321-zyxwvutsrq'],
    'private-key': ['echo "-----BEGIN RSA PRIVATE KEY-----"', 'cat <<EOF -----BEGIN OPENSSH PRIVATE KEY-----'],
    jwt: ['curl -H x eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.sig', 'echo eyJ0eXAiOiJKV1Qi.eyJpc3MiOiJhYmMi.x'],
    bearer: ['curl -H "X: Bearer abcdef123456"', 'http GET x "bearer zzzzzzzz99"'],
    'auth-header': ['curl -H "Authorization: Basic abc"', 'curl -H authorization=xyz'],
    'credential-assignment': ['mysql -u root password=hunter22', 'export API_KEY="abc123"', 'curl -d "secret: topsecret"', 'x TOKEN=abcd'],
    'credential-flag': ['deploy --password hunter22', 'tool --api-key=abc123'],
    'url-credentials': ['git push https://name:pw123@example.com/r.git', 'psql postgres://u:p@db/x'],
  };
  for (const [reason, inputs] of Object.entries(cases)) {
    for (const input of inputs) {
      const out = red(input);
      assert.equal(out.refused, true, input);
      assert.equal(out.reason, reason, input);
      assert.equal(out.text, undefined);
    }
  }
  assert.equal(red('llm --max_tokens=100 run').refused, false);
  assert.equal(red('npm run tokenize').refused, false);
  const dir = writeLog('secret', [base({ action_preview: 'echo ghp_abcdefghijklmnopqrstuvwxyz0123456789' })]);
  assert.equal(story.listStories({ dir }).length, 0);
  assert.equal(story.whyNot(story.readReceipts(dir)[0]), 'redaction:github-token');
});

test('seen state survives a round trip and a corrupt file', () => {
  const env = { VESTIGE_REM_HOME: path.join(OUT, 'state') };
  assert.equal(story.isSeen('0123456789abcdef', env), false);
  assert.equal(story.markSeen('0123456789abcdef', env), true);
  assert.equal(story.isSeen('0123456789abcdef', env), true);
  assert.equal(story.markSeen('not-a-digest', env), false);
  fs.writeFileSync(story.statePath(env), '{ not json');
  assert.deepEqual(story.loadState(env), { v: 1, seen: {} });
  assert.equal(story.markSeen('fedcba9876543210', env), true);
  assert.deepEqual(Object.keys(story.loadState(env).seen), ['fedcba9876543210']);
  assert.ok(story.statePath(env).startsWith(OUT), 'the test never writes to the real state folder');
});

test('extra sources come from the environment', () => {
  const sources = story.allowedSources({ VESTIGE_REM_SOURCES: 'myagent, Bad Name!,other-1' });
  assert.ok(sources.has('myagent') && sources.has('other-1') && sources.has('claude'));
  assert.ok(!sources.has('bad name!'));
});

test('real log: every story is safe to show (counts only)', { skip: !fs.existsSync(story.receiptsDir()) }, () => {
  const stories = story.listStories();
  const home = os.homedir();
  const user = os.userInfo().username;
  let leaks = 0;
  for (const s of stories) {
    assert.match(s.action, /^[\x20-\x7e…]{1,80}$/);
    if (s.action.includes(home) || new RegExp(`(^|[^A-Za-z0-9_])${user}(?![A-Za-z0-9_])`).test(s.action)) leaks++;
  }
  assert.equal(leaks, 0);
  console.log(`real log: ${stories.length} stories checked, 0 contain the home path or user name`);
});
