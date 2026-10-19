const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const kernel = require('../lib/vestige-kernel.js');

const POSIX = process.platform !== 'win32';

function scratch(name) {
  return fs.mkdtempSync(path.join(os.tmpdir(), `vestige-kernel-${name}-`));
}

// A stand-in binary that answers --version the way vestige-mcp does.
function fakeBinary(dir, name, version) {
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, name);
  fs.writeFileSync(file, `#!/bin/sh\necho "${name} ${version}"\n`, { mode: 0o755 });
  return file;
}

function bundle(version) {
  const dir = scratch('bundle');
  for (const name of kernel.BINARIES) fakeBinary(dir, name, version);
  return dir;
}

function withEnv(vars, fn) {
  const saved = {};
  for (const [key, value] of Object.entries(vars)) {
    saved[key] = process.env[key];
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
  try {
    return fn();
  } finally {
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const backup = fs.existsSync(file) ? `${file}.bak.test` : null;
  if (backup) fs.copyFileSync(file, backup);
  fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
  return backup;
}

test('each supported platform maps to the package that carries its binaries', () => {
  assert.equal(kernel.platformPackage('darwin', 'arm64'), '@vestige/mcp-darwin-arm64');
  assert.equal(kernel.platformPackage('darwin', 'x64'), '@vestige/mcp-darwin-x64');
  assert.equal(kernel.platformPackage('linux', 'arm64'), '@vestige/mcp-linux-arm64');
  assert.equal(kernel.platformPackage('linux', 'x64'), '@vestige/mcp-linux-x64');
  assert.equal(kernel.platformPackage('win32', 'x64'), '@vestige/mcp-win32-x64');
  assert.equal(kernel.platformPackage('win32', 'arm64'), null);
  assert.equal(kernel.platformPackage('freebsd', 'x64'), null);
});

test('versions are read from --version output and compared numerically', () => {
  assert.deepEqual(kernel.parseVersion('vestige-mcp 5.0.0\n'), [5, 0, 0]);
  assert.equal(kernel.parseVersion('no version here'), null);
  assert.ok(kernel.compareVersions([5, 0, 10], [5, 0, 9]) > 0);
  assert.ok(kernel.compareVersions([4, 9, 9], [5, 0, 0]) < 0);
  assert.equal(kernel.compareVersions([5, 0, 0], [5, 0, 0]), 0);
});

test('placing a binary over a symlink replaces the link and leaves its target alone', { skip: !POSIX }, () => {
  const dir = scratch('symlink');
  const pinned = fakeBinary(path.join(dir, 'pin'), 'vestige-mcp', '4.0.0');
  const link = path.join(dir, 'bin', 'vestige-mcp');
  fs.mkdirSync(path.dirname(link));
  fs.symlinkSync(pinned, link);
  const fresh = fakeBinary(path.join(dir, 'new'), 'vestige-mcp', '5.0.0');

  kernel.placeFile(fresh, link);

  assert.equal(fs.lstatSync(link).isSymbolicLink(), false);
  assert.match(fs.readFileSync(link, 'utf8'), /5\.0\.0/);
  assert.match(fs.readFileSync(pinned, 'utf8'), /4\.0\.0/, 'the pinned binary must not be overwritten');
});

test('a machine with no vestige gets all four binaries from the bundle', { skip: !POSIX }, () => {
  const target = scratch('install');
  const result = withEnv({ VESTIGE_BUNDLED_BIN_DIR: bundle('5.0.0'), VESTIGE_INSTALL_DIR: target },
    () => kernel.ensureBinaries({ found: null }));
  assert.equal(result.status, 'installed');
  assert.equal(result.version, '5.0.0');
  assert.equal(result.binaryPath, path.join(target, 'vestige-mcp'));
  assert.deepEqual(result.copied.sort(), [...kernel.BINARIES].sort());
  for (const name of kernel.BINARIES) assert.ok(fs.existsSync(path.join(target, name)), name);
});

test('an installed vestige-mcp as new as the bundle is kept, and nothing is copied', { skip: !POSIX }, () => {
  const target = scratch('keep');
  const found = fakeBinary(scratch('found'), 'vestige-mcp', '5.0.0');
  const result = withEnv({ VESTIGE_BUNDLED_BIN_DIR: bundle('5.0.0'), VESTIGE_INSTALL_DIR: target },
    () => kernel.ensureBinaries({ found }));
  assert.equal(result.status, 'kept');
  assert.equal(result.binaryPath, found);
  assert.deepEqual(fs.readdirSync(target), []);
});

test('an older vestige-mcp is replaced by the bundled one, and a dry run changes nothing', { skip: !POSIX }, () => {
  const target = scratch('upgrade');
  const found = fakeBinary(target, 'vestige-mcp', '4.2.0');
  const env = { VESTIGE_BUNDLED_BIN_DIR: bundle('5.0.0'), VESTIGE_INSTALL_DIR: target };

  const planned = withEnv(env, () => kernel.ensureBinaries({ found, dryRun: true }));
  assert.equal(planned.status, 'planned');
  assert.match(fs.readFileSync(found, 'utf8'), /4\.2\.0/);

  const result = withEnv(env, () => kernel.ensureBinaries({ found }));
  assert.equal(result.status, 'installed');
  assert.equal(result.replaced, '4.2.0');
  assert.match(fs.readFileSync(found, 'utf8'), /5\.0\.0/);
});

test('no bundle and no installed binary reports missing instead of guessing', () => {
  const result = withEnv({ VESTIGE_BUNDLED_BIN_DIR: path.join(scratch('empty'), 'nothing'), VESTIGE_INSTALL_DIR: scratch('none') },
    () => kernel.ensureBinaries({ found: null }));
  assert.equal(result.status, 'missing');
});

test('the rules hook is merged into Claude Code settings once, beside what is already there', () => {
  const home = scratch('home');
  const settingsPath = path.join(home, '.claude', 'settings.json');
  const vestige = fakeBinary(path.join(home, 'bin'), 'vestige', '5.0.0');
  const gate = { matcher: '*', hooks: [{ type: 'command', command: 'python3 ~/.operator/gate/operator-gate.py hook --source claude' }] };
  const other = { matcher: 'startup', hooks: [{ type: 'command', command: 'echo hello' }] };
  writeJson(settingsPath, { model: 'opus', hooks: { PreToolUse: [gate], SessionStart: [other] } });

  const first = kernel.wireRulesHook(vestige, { home, writeJson });
  assert.equal(first.status, 'written');
  const settings = JSON.parse(fs.readFileSync(settingsPath, 'utf8'));
  assert.equal(settings.model, 'opus');
  assert.deepEqual(settings.hooks.PreToolUse, [gate]);
  assert.deepEqual(settings.hooks.SessionStart[0], other);
  const added = settings.hooks.SessionStart[1];
  assert.equal(added.matcher, kernel.RULES_MATCHER);
  assert.equal(added.hooks[0].command, `${vestige} hook session-start`);
  assert.equal(added.hooks[0].timeout, 10);
  assert.ok(first.backupPath && fs.existsSync(first.backupPath));

  const second = kernel.wireRulesHook(vestige, { home, writeJson });
  assert.equal(second.status, 'present');
  assert.equal(JSON.parse(fs.readFileSync(settingsPath, 'utf8')).hooks.SessionStart.length, 2);
});

test('a settings file that does not parse is left exactly as it is', () => {
  const home = scratch('broken');
  const settingsPath = path.join(home, '.claude', 'settings.json');
  fs.mkdirSync(path.dirname(settingsPath), { recursive: true });
  const raw = '{ "hooks": { // mid-edit\n';
  fs.writeFileSync(settingsPath, raw);
  const vestige = fakeBinary(path.join(home, 'bin'), 'vestige', '5.0.0');

  const result = kernel.wireRulesHook(vestige, { home, writeJson });
  assert.equal(result.status, 'review');
  assert.equal(fs.readFileSync(settingsPath, 'utf8'), raw);
});

test('a dry run plans the rules hook and writes nothing; a path with spaces is quoted', () => {
  const home = scratch('dry');
  fs.mkdirSync(path.join(home, '.claude'), { recursive: true });
  const vestige = fakeBinary(path.join(home, 'Application Support', 'bin'), 'vestige', '5.0.0');

  const result = kernel.wireRulesHook(vestige, { home, dryRun: true, writeJson });
  assert.equal(result.status, 'planned');
  assert.equal(result.command, `${JSON.stringify(vestige)} hook session-start`);
  assert.equal(fs.existsSync(path.join(home, '.claude', 'settings.json')), false);
});

test('the gate installer exit codes map to what the summary tells the user', () => {
  assert.equal(kernel.GATE_EXIT[0], 'installed');
  assert.equal(kernel.GATE_EXIT[3], 'agent-shell');
  assert.equal(kernel.GATE_EXIT[4], 'kept-existing');
  assert.equal(kernel.GATE_EXIT[5], 'installed-unwired');
});

test('the data directory follows VESTIGE_DATA_DIR, then the platform default', () => {
  const home = '/home/someone';
  assert.equal(withEnv({ VESTIGE_DATA_DIR: '/srv/vestige' }, () => kernel.dataDir(home, 'linux')), '/srv/vestige');
  assert.equal(withEnv({ VESTIGE_DATA_DIR: undefined }, () => kernel.dataDir(home, 'darwin')),
    path.join(home, 'Library', 'Application Support', 'com.vestige.core'));
  assert.equal(withEnv({ VESTIGE_DATA_DIR: undefined, XDG_DATA_HOME: undefined }, () => kernel.dataDir(home, 'linux')),
    path.join(home, '.local', 'share', 'vestige'));
});
