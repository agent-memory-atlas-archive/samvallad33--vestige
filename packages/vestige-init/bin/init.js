#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const os = require('os');
const readline = require('readline');
const { execFileSync, execSync, spawn } = require('child_process');

const PACKAGE_VERSION = process.env.VESTIGE_INIT_VERSION || require('../package.json').version;
const HOME = os.homedir();
const PLATFORM = os.platform();
const RAW_ARGS = process.argv.slice(2);
const FLAGS = new Set(RAW_ARGS);
const JSON_OUTPUT = FLAGS.has('--json');
const DRY_RUN = FLAGS.has('--dry-run');

function valuesFor(flag) {
  const values = [];
  for (let index = 0; index < RAW_ARGS.length; index++) {
    if (RAW_ARGS[index] === flag && RAW_ARGS[index + 1]) values.push(RAW_ARGS[index + 1]);
  }
  return values;
}

function print(...args) {
  if (!JSON_OUTPUT) console.log(...args);
}

// ─── Branding ───────────────────────────────────────────────────────────────

const COLOR = process.stdout.isTTY && !process.env.NO_COLOR && !JSON_OUTPUT && process.env.TERM !== 'dumb';
const ANSI = COLOR;
const tone = {
  cyan: (value) => ANSI ? `\x1b[38;5;51m${value}\x1b[0m` : value,
  violet: (value) => ANSI ? `\x1b[38;5;141m${value}\x1b[0m` : value,
  magenta: (value) => ANSI ? `\x1b[1;38;5;213m${value}\x1b[0m` : value,
  electric: (value) => ANSI ? `\x1b[1;38;5;87m${value}\x1b[0m` : value,
  dim: (value) => ANSI ? `\x1b[2m${value}\x1b[0m` : value,
  green: (value) => ANSI ? `\x1b[38;5;84m${value}\x1b[0m` : value,
  amber: (value) => ANSI ? `\x1b[38;5;222m${value}\x1b[0m` : value,
};

function banner() {
  print('');
  print(tone.cyan('  ╭──────────────────────────────────────────────╮'));
  print(tone.cyan('  │') + `  ${tone.violet('VESTIGE')}  ${tone.dim('// LOCAL CONTROL PLANE')}            ` + tone.cyan('│'));
  print(tone.cyan('  │') + `  ${tone.dim('detect  →  preserve  →  configure')}            ` + tone.cyan('│'));
  print(tone.cyan('  ╰──────────────────────────────────────────────╯'));
  print(tone.dim(`  setup wizard v${PACKAGE_VERSION} · no cloud account · no telemetry`));
  print('');
}

// ─── IDE Definitions ────────────────────────────────────────────────────────

const IDE_CONFIGS = {
  'Claude Code': {
    detect: () => {
      try {
        execSync('which claude', { stdio: 'ignore' });
        return true;
      } catch { return false; }
    },
    configPath: () => path.join(HOME, '.claude', 'settings.json'),
    format: 'claude-code',
    inject: (binaryPath) => {
      try {
        const result = execFileSync('claude', ['mcp', 'add', 'vestige', binaryPath, '-s', 'user'], {
          encoding: 'utf8',
          stdio: ['ignore', 'pipe', 'pipe'],
        });
        if (result.includes('already exists')) {
          print('  [skip] Claude Code — already configured');
          return false;
        }
        return true;
      } catch (err) {
        const msg = err.stdout || err.stderr || '';
        if (msg.includes('already exists')) {
          print('  [skip] Claude Code — already configured');
          return false;
        }
        return false;
      }
    },
  },

  'Claude Desktop': {
    detect: () => {
      const paths = {
        darwin: path.join(HOME, 'Library', 'Application Support', 'Claude', 'claude_desktop_config.json'),
        win32: path.join(process.env.APPDATA || '', 'Claude', 'claude_desktop_config.json'),
        linux: path.join(HOME, '.config', 'Claude', 'claude_desktop_config.json'),
      };
      return fs.existsSync(paths[PLATFORM] || '');
    },
    configPath: () => {
      const paths = {
        darwin: path.join(HOME, 'Library', 'Application Support', 'Claude', 'claude_desktop_config.json'),
        win32: path.join(process.env.APPDATA || '', 'Claude', 'claude_desktop_config.json'),
        linux: path.join(HOME, '.config', 'Claude', 'claude_desktop_config.json'),
      };
      return paths[PLATFORM];
    },
    format: 'mcpServers',
    key: 'mcpServers',
  },

  'Cursor': {
    detect: () => {
      const configPath = path.join(HOME, '.cursor', 'mcp.json');
      const appExists = PLATFORM === 'darwin'
        ? fs.existsSync('/Applications/Cursor.app')
        : fs.existsSync(path.join(HOME, '.cursor'));
      return appExists || fs.existsSync(configPath);
    },
    configPath: () => path.join(HOME, '.cursor', 'mcp.json'),
    format: 'mcpServers',
    key: 'mcpServers',
  },

  'VS Code (Copilot)': {
    detect: () => {
      try {
        execSync('which code', { stdio: 'ignore' });
        return true;
      } catch {
        return PLATFORM === 'darwin' && fs.existsSync('/Applications/Visual Studio Code.app');
      }
    },
    configPath: () => {
      // User-level MCP config
      const paths = {
        darwin: path.join(HOME, 'Library', 'Application Support', 'Code', 'User', 'settings.json'),
        win32: path.join(process.env.APPDATA || '', 'Code', 'User', 'settings.json'),
        linux: path.join(HOME, '.config', 'Code', 'User', 'settings.json'),
      };
      return paths[PLATFORM];
    },
    format: 'vscode',
    key: 'servers',
    note: 'Tip: For project-level config, create .vscode/mcp.json with {"servers": {"vestige": ...}}',
  },

  'OpenCode': {
    detect: () => {
      try {
        execSync(PLATFORM === 'win32' ? 'where opencode' : 'which opencode', { stdio: 'ignore' });
        return true;
      } catch {
        return fs.existsSync(path.join(HOME, '.config', 'opencode'));
      }
    },
    configPath: () => path.join(HOME, '.config', 'opencode', 'opencode.json'),
    format: 'opencode',
    key: 'mcp',
    note: 'Tip: For project-level memory, add the same mcp.vestige block to an opencode.json in your repo root.',
  },

  'Xcode 26.3': {
    detect: () => {
      if (PLATFORM !== 'darwin') return false;
      return fs.existsSync('/Applications/Xcode.app') ||
             fs.existsSync(path.join(HOME, 'Library', 'Developer', 'Xcode'));
    },
    configPath: () => path.join(HOME, 'Library', 'Developer', 'Xcode', 'CodingAssistant', 'ClaudeAgentConfig', '.claude'),
    format: 'xcode',
    key: 'projects',
  },

  'JetBrains (Junie)': {
    detect: () => {
      const jetbrainsDir = PLATFORM === 'darwin'
        ? path.join(HOME, 'Library', 'Application Support', 'JetBrains')
        : path.join(HOME, '.config', 'JetBrains');
      return fs.existsSync(jetbrainsDir);
    },
    configPath: () => path.join(HOME, '.junie', 'mcp', 'mcp.json'),
    format: 'mcpServers',
    key: 'mcpServers',
  },

  'Windsurf': {
    detect: () => {
      const configPath = PLATFORM === 'win32'
        ? path.join(HOME, '.codeium', 'windsurf', 'mcp_config.json')
        : path.join(HOME, '.codeium', 'windsurf', 'mcp_config.json');
      return fs.existsSync(configPath) ||
             (PLATFORM === 'darwin' && fs.existsSync('/Applications/Windsurf.app'));
    },
    configPath: () => path.join(HOME, '.codeium', 'windsurf', 'mcp_config.json'),
    format: 'mcpServers',
    key: 'mcpServers',
  },
};

// ─── Helpers ────────────────────────────────────────────────────────────────

function findBinary() {
  const explicitBinary = process.env.VESTIGE_MCP_PATH;
  if (explicitBinary && fs.existsSync(explicitBinary)) return explicitBinary;

  // Check common locations
  const candidates = [
    path.join(HOME, '.local', 'bin', 'vestige-mcp'),
    path.join('/usr', 'local', 'bin', 'vestige-mcp'),
    path.join(HOME, '.cargo', 'bin', 'vestige-mcp'),
    // npm global install location
    (() => {
      try {
        const npmPrefix = execSync('npm prefix -g', {
          encoding: 'utf8',
          stdio: ['ignore', 'pipe', 'ignore'],
        }).trim();
        return path.join(npmPrefix, 'bin', 'vestige-mcp');
      } catch { return null; }
    })(),
  ].filter(Boolean);

  // Also try which/where
  try {
    const result = execSync(PLATFORM === 'win32' ? 'where vestige-mcp' : 'which vestige-mcp', {
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'ignore'],
    }).trim();
    const firstMatch = result
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)[0];
    if (firstMatch) candidates.unshift(firstMatch);
  } catch {}

  for (const candidate of candidates) {
    if (fs.existsSync(candidate)) return candidate;
  }
  return null;
}

function stripJsonComments(input) {
  let output = '';
  let inString = false;
  let escaped = false;

  for (let i = 0; i < input.length; i++) {
    const current = input[i];
    const next = input[i + 1];

    if (inString) {
      output += current;
      if (escaped) {
        escaped = false;
      } else if (current === '\\') {
        escaped = true;
      } else if (current === '"') {
        inString = false;
      }
      continue;
    }

    if (current === '"') {
      inString = true;
      output += current;
      continue;
    }

    if (current === '/' && next === '/') {
      while (i < input.length && input[i] !== '\n') i++;
      output += '\n';
      continue;
    }

    if (current === '/' && next === '*') {
      i += 2;
      while (i < input.length && !(input[i] === '*' && input[i + 1] === '/')) i++;
      i++;
      continue;
    }

    output += current;
  }

  return output;
}

function removeTrailingCommas(input) {
  return input.replace(/,\s*([}\]])/g, '$1');
}

function readJsonSafe(filePath) {
  try {
    const content = fs.readFileSync(filePath, 'utf8');
    return JSON.parse(removeTrailingCommas(stripJsonComments(content)));
  } catch (err) {
    if (fs.existsSync(filePath)) {
      throw new Error(`Could not parse ${filePath}: ${err.message}`);
    }
    return null;
  }
}

function ensureDir(filePath) {
  const dir = path.dirname(filePath);
  if (!dir || dir === '.') return;
  fs.mkdirSync(dir, { recursive: true });
}

function backupFile(filePath) {
  if (!fs.existsSync(filePath)) return null;
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  const backupPath = `${filePath}.bak.${stamp}`;
  fs.copyFileSync(filePath, backupPath);
  try {
    fs.chmodSync(backupPath, 0o600);
  } catch {}
  return backupPath;
}

function writeJsonAtomic(filePath, value) {
  ensureDir(filePath);
  const backupPath = backupFile(filePath);
  const tempPath = `${filePath}.tmp-${process.pid}-${Date.now()}`;
  fs.writeFileSync(tempPath, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 });
  fs.renameSync(tempPath, filePath);
  try {
    fs.chmodSync(filePath, 0o600);
  } catch {}
  return backupPath;
}

function buildVestigeConfig(binaryPath) {
  return {
    command: binaryPath,
    args: [],
    env: {},
  };
}

function buildOpenCodeConfig(binaryPath) {
  return {
    type: 'local',
    command: [binaryPath],
    enabled: true,
    timeout: 10000,
    environment: {},
  };
}

function buildXcodeConfig(binaryPath) {
  return {
    projects: {
      '*': {
        mcpServers: {
          vestige: {
            type: 'stdio',
            command: binaryPath,
            args: [],
            env: {
              PATH: '/usr/local/bin:/usr/bin:/bin',
            },
          },
        },
      },
    },
  };
}

function injectConfig(ide, ideName, binaryPath) {
  const configPath = ide.configPath();
  if (!configPath) return false;

  // Claude Code uses its own CLI
  if (ide.format === 'claude-code') {
    return { status: ide.inject(binaryPath) ? 'written' : 'skipped', backupPath: null };
  }

  ensureDir(configPath);
  let config = readJsonSafe(configPath) || {};

  if (ide.format === 'xcode') {
    // Xcode has a different structure
    const xcodeConfig = buildXcodeConfig(binaryPath);
    if (config.projects && config.projects['*'] && config.projects['*'].mcpServers && config.projects['*'].mcpServers.vestige) {
      print(`  [skip] ${ideName} — already configured`);
      return { status: 'skipped', backupPath: null };
    }
    // Merge with existing config
    if (!config.projects) config.projects = {};
    if (!config.projects['*']) config.projects['*'] = {};
    if (!config.projects['*'].mcpServers) config.projects['*'].mcpServers = {};
    config.projects['*'].mcpServers.vestige = xcodeConfig.projects['*'].mcpServers.vestige;
  } else if (ide.format === 'vscode') {
    // VS Code uses "mcp" key in settings.json with "servers" subkey
    if (!config.mcp) config.mcp = {};
    if (!config.mcp.servers) config.mcp.servers = {};
    if (config.mcp.servers.vestige) {
      print(`  [skip] ${ideName} — already configured`);
      return { status: 'skipped', backupPath: null };
    }
    config.mcp.servers.vestige = buildVestigeConfig(binaryPath);
  } else if (ide.format === 'opencode') {
    // OpenCode uses top-level "mcp" entries with command arrays.
    if (!config.$schema) config.$schema = 'https://opencode.ai/config.json';
    if (!config.mcp) config.mcp = {};
    let migratedOpenCodeConfig = false;
    if (config.mcpServers && config.mcpServers.vestige) {
      delete config.mcpServers.vestige;
      migratedOpenCodeConfig = true;
      if (Object.keys(config.mcpServers).length === 0) {
        delete config.mcpServers;
      }
      print(`  [migrate] ${ideName} — moved vestige from mcpServers to mcp`);
    }
    if (config.mcp.vestige) {
      if (!migratedOpenCodeConfig) {
        print(`  [skip] ${ideName} — already configured`);
        return { status: 'skipped', backupPath: null };
      }
      // Preserve the valid OpenCode entry while still writing the stale-key cleanup.
    } else {
      config.mcp.vestige = buildOpenCodeConfig(binaryPath);
    }
  } else {
    // Standard mcpServers format (Cursor, Claude Desktop, JetBrains, Windsurf)
    const key = ide.key || 'mcpServers';
    if (!config[key]) config[key] = {};
    if (config[key].vestige) {
      print(`  [skip] ${ideName} — already configured`);
      return { status: 'skipped', backupPath: null };
    }
    config[key].vestige = buildVestigeConfig(binaryPath);
  }

  const backupPath = writeJsonAtomic(configPath, config);
  if (backupPath) {
    print(`  [backup] ${path.basename(backupPath)}`);
  }
  return { status: 'written', backupPath };
}

// ─── Main ───────────────────────────────────────────────────────────────────

function ask(question) {
  const terminal = readline.createInterface({ input: process.stdin, output: process.stdout });
  return new Promise((resolve) => terminal.question(question, (answer) => {
    terminal.close();
    resolve(answer.trim());
  }));
}

const CLIENT_ALIASES = {
  claude: 'Claude Code',
  'claude-code': 'Claude Code',
  'claude-desktop': 'Claude Desktop',
  cursor: 'Cursor',
  vscode: 'VS Code (Copilot)',
  'vs-code': 'VS Code (Copilot)',
  opencode: 'OpenCode',
  xcode: 'Xcode 26.3',
  jetbrains: 'JetBrains (Junie)',
  junie: 'JetBrains (Junie)',
  windsurf: 'Windsurf',
};

function requestedClientNames() {
  return valuesFor('--client')
    .flatMap((value) => value.split(','))
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean)
    .map((value) => ({ value, name: CLIENT_ALIASES[value] }));
}

function usage() {
  return [
    'Usage: vestige-init [--interactive] [--client <name>] [--all --yes] [--dry-run] [--json]',
    '',
    '  --interactive       choose detected clients in this terminal',
    '  --client <name>     configure one explicit client (claude-code, cursor, vscode, opencode, xcode, jetbrains, windsurf)',
    '  --all --yes         configure every detected client without a prompt',
    '  --dry-run           inspect proposed changes without writing',
    '  --json              print a machine-readable result',
    '  --line-menu         use the numbered text selector instead of keyboard control',
    '  --no-animate        keep the keyboard selector still',
  ].join('\n');
}

function selectedByName(detected, requests) {
  const unknown = requests.filter(({ name }) => !name);
  if (unknown.length > 0) {
    throw new Error(`unknown client: ${unknown.map(({ value }) => value).join(', ')}`);
  }
  const wanted = new Set(requests.map(({ name }) => name));
  const selected = detected.filter(({ name }) => wanted.has(name));
  if (selected.length !== wanted.size) {
    const unavailable = [...wanted].filter((name) => !selected.some((client) => client.name === name));
    throw new Error(`requested client not detected: ${unavailable.join(', ')}`);
  }
  return selected;
}

function supportsSignalField() {
  return !JSON_OUTPUT &&
    !process.env.CI &&
    !FLAGS.has('--line-menu') &&
    process.env.TERM !== 'dumb' &&
    process.stdin.isTTY &&
    process.stdout.isTTY &&
    typeof process.stdin.setRawMode === 'function';
}

function signalGlyph(phase, selected) {
  if (!selected) return tone.dim('○');
  return [tone.cyan('◉'), tone.electric('✦'), tone.magenta('◈'), tone.electric('✧')][phase % 4];
}

function signalFieldLines(detected, cursor, selected, phase, notice = '') {
  const aura = ['·  ·  ◌  ·  ·', '·  ◌  ✦  ◌  ·', '◌  ✦  ◉  ✦  ◌', '·  ✦  ◌  ✦  ·'][phase % 4];
  const width = Math.min(process.stdout.columns || 74, 80);
  const compact = width < 72;
  const selectionCount = selected.size;
  const lines = [];

  if (!compact) {
    const rule = '═'.repeat(width - 4);
    lines.push(`  ${tone.cyan('╭' + rule + '╮')}`);
    lines.push(`  ${tone.cyan('│')}  ${tone.magenta('VESTIGE')} ${tone.dim('// LOCAL CONTROL PLANE')} ${tone.electric(aura)} ${' '.repeat(Math.max(0, width - 49))}${tone.cyan('│')}`);
    lines.push(`  ${tone.cyan('╰' + rule + '╯')}`);
  } else {
    lines.push(`  ${tone.magenta('VESTIGE')} ${tone.dim('// LOCAL CONTROL PLANE')} ${tone.electric(aura)}`);
  }

  lines.push('');
  lines.push(`  ${tone.violet('01')}  ${tone.electric('CHOOSE YOUR LOCAL CLIENTS')}  ${tone.dim(`${selectionCount} selected`)}`);
  lines.push(`      ${tone.magenta('◉')} ${tone.dim('STRATA')}  ${tone.dim('local evidence source')}`);
  detected.forEach(({ name }, index) => {
    const active = index === cursor;
    const checked = selected.has(index);
    const pointer = active ? tone.electric('›') : ' ';
    const mark = signalGlyph(phase + index, checked);
    const label = active ? tone.electric(name.padEnd(24)) : name.padEnd(24);
    const state = checked ? tone.green('ARMED') : tone.dim('idle');
    lines.push(`      ${pointer} ${mark}  ${label} ${state}`);
  });
  lines.push('');
  lines.push(`  ${tone.electric('↑↓ / j k')} move   ${tone.electric('space')} toggle   ${tone.electric('A')} all   ${tone.electric('N')} none`);
  lines.push(`  ${tone.electric('enter')} review changes   ${tone.electric('esc / Q')} leave unchanged`);
  lines.push(tone.dim('  Selection stays local until you approve the review screen.'));
  lines.push(notice ? tone.amber(`  ${notice}`) : tone.dim('  Every selected configuration is backed up before Vestige is added.'));
  return lines;
}

function paintTransient(lines, state) {
  if (state.lineCount > 0) process.stdout.write(`\x1b[${state.lineCount}A`);
  const lineCount = Math.max(lines.length, state.lineCount);
  for (let index = 0; index < lineCount; index++) {
    process.stdout.write(`\r\x1b[2K${lines[index] || ''}\n`);
  }
  state.lineCount = lines.length;
}

function reviewSelectedClients(detected, selected) {
  return new Promise((resolve) => {
    const names = detected.filter((_, index) => selected.has(index)).map(({ name }) => name);
    print('');
    print(tone.cyan('  02  REVIEW LOCAL CHANGES'));
    names.forEach((name) => print(`      ${tone.magenta('◉')}  ${name}`));
    print('');
    print(tone.dim('  Vestige will add an MCP server entry for these clients.'));
    print(tone.dim('  Existing file-based configuration is backed up before modification.'));
    print('');
    print(`  ${tone.electric('enter')} write local configuration   ${tone.electric('esc')} go back   ${tone.electric('Q')} leave unchanged`);

    const wasRaw = Boolean(process.stdin.isRaw);
    const cleanup = () => {
      process.stdin.off('keypress', onKey);
      process.stdin.setRawMode(wasRaw);
      process.stdin.pause();
    };
    const onKey = (_value, key = {}) => {
      if (key.name === 'return' || key.name === 'enter') {
        cleanup();
        print('');
        resolve('confirm');
      } else if (key.name === 'escape') {
        cleanup();
        print('');
        resolve('back');
      } else if (key.name === 'q' || (key.ctrl && key.name === 'c')) {
        cleanup();
        if (key.ctrl) process.exitCode = 130;
        print('');
        resolve('cancel');
      }
    };
    readline.emitKeypressEvents(process.stdin);
    process.stdin.setRawMode(true);
    process.stdin.resume();
    process.stdin.on('keypress', onKey);
  });
}

async function selectClientsWithSignalField(detected) {
  const state = { lineCount: 0 };
  let cursor = 0;
  let phase = 0;
  let notice = '';
  const selected = new Set(detected.map((_, index) => index));
  const reducedMotion = FLAGS.has('--no-animate') || process.env.VESTIGE_REDUCE_MOTION === '1';
  const redraw = () => paintTransient(signalFieldLines(detected, cursor, selected, phase, notice), state);

  const choose = await new Promise((resolve) => {
    const wasRaw = Boolean(process.stdin.isRaw);
    let timer = null;
    const cleanup = () => {
      if (timer) clearInterval(timer);
      process.stdin.off('keypress', onKey);
      if (process.stdout.off) process.stdout.off('resize', redraw);
      process.stdin.setRawMode(wasRaw);
      process.stdin.pause();
      process.stdout.write('\x1b[?25h');
    };
    const done = (result) => {
      cleanup();
      process.stdout.write('\n');
      resolve(result);
    };
    const onKey = (_value, key = {}) => {
      notice = '';
      if (key.ctrl && key.name === 'c') {
        process.exitCode = 130;
        return done('cancel');
      }
      if (key.name === 'escape' || key.name === 'q') return done('cancel');
      if (key.name === 'return' || key.name === 'enter') {
        if (selected.size === 0) {
          notice = 'No clients selected — Space toggles, A selects all.';
          return redraw();
        }
        return done('review');
      }
      if (key.name === 'up' || key.name === 'k') cursor = (cursor + detected.length - 1) % detected.length;
      else if (key.name === 'down' || key.name === 'j') cursor = (cursor + 1) % detected.length;
      else if (key.name === 'space') {
        if (selected.has(cursor)) selected.delete(cursor);
        else selected.add(cursor);
      } else if (key.name === 'a') {
        detected.forEach((_, index) => selected.add(index));
      } else if (key.name === 'n') {
        selected.clear();
      }
      if (selected.size === 0) notice = 'No clients selected — Space toggles, A selects all.';
      redraw();
    };
    readline.emitKeypressEvents(process.stdin);
    process.stdout.write('\x1b[?25l');
    process.stdin.setRawMode(true);
    process.stdin.resume();
    process.stdin.on('keypress', onKey);
    if (process.stdout.on) process.stdout.on('resize', redraw);
    if (!reducedMotion) timer = setInterval(() => {
      phase = (phase + 1) % 4;
      redraw();
    }, 320);
    redraw();
  });

  if (choose === 'cancel') return [];
  if (selected.size === 0) return selectClientsWithSignalField(detected);
  const review = await reviewSelectedClients(detected, selected);
  if (review === 'confirm') return detected.filter((_, index) => selected.has(index));
  if (review === 'cancel') return [];
  return selectClientsWithSignalField(detected);
}

async function selectClients(detected) {
  const requests = requestedClientNames();
  const interactive = FLAGS.has('--interactive') || (process.stdin.isTTY && !JSON_OUTPUT && !FLAGS.has('--all') && requests.length === 0 && !DRY_RUN);

  if (requests.length > 0) return selectedByName(detected, requests);
  if (DRY_RUN) return detected;
  if (FLAGS.has('--all')) {
    if (!FLAGS.has('--yes')) throw new Error('--all changes every detected client; add --yes to confirm');
    return detected;
  }
  if (!interactive) {
    throw new Error('non-interactive mode makes no changes by default; use --dry-run, --client <name>, or --all --yes');
  }

  if (supportsSignalField()) return selectClientsWithSignalField(detected);

  print(tone.cyan('  01  DETECTED CLIENTS'));
  detected.forEach(({ name }, index) => {
    print(`      ${tone.violet(String(index + 1).padStart(2, '0'))}  ${name}`);
  });
  print(`      ${tone.violet('A ')}  Configure every detected client`);
  print(`      ${tone.violet('Q ')}  Exit without changing anything`);
  print('');
  print(tone.dim('  Each selected configuration is backed up before Vestige is added.'));

  const answer = (await ask(tone.cyan('  Choose clients [A]: '))).toLowerCase();
  if (!answer || answer === 'a' || answer === 'all') return detected;
  if (answer === 'q' || answer === 'quit') return [];

  const selections = new Set(answer.split(/[\s,]+/).map(Number).filter(Number.isInteger));
  const selected = detected.filter((_, index) => selections.has(index + 1));
  if (selected.length === 0) {
    print(tone.amber('  No valid client selection. Nothing changed.'));
  }
  return selected;
}

function renderProofPulse({ detected, configured, skipped, backups }) {
  print(tone.cyan('  03  PROOF PULSE'));
  print(`      ${tone.green('●')}  DETECTED     ${detected} local client${detected === 1 ? '' : 's'}`);
  print(`      ${tone.green('●')}  PRESERVED    ${backups} configuration backup${backups === 1 ? '' : 's'} created`);
  print(`      ${tone.green('●')}  CONFIGURED   ${configured} Vestige MCP connection${configured === 1 ? '' : 's'} written${skipped ? ` · ${skipped} already present` : ''}`);
  print(`      ${tone.amber('○')}  VERIFY NEXT  restart the selected client and confirm its MCP tools`);
  print('');
  print(tone.violet('      CONFIGURED ≠ VERIFIED'));
  print(tone.dim('      The local configuration is written. A live client response is the next proof.'));
}

function dashboardBinary(binaryPath) {
  const executable = PLATFORM === 'win32' ? 'vestige.exe' : 'vestige';
  const sibling = path.join(path.dirname(binaryPath), executable);
  return fs.existsSync(sibling) ? sibling : null;
}

function openDashboard(binaryPath) {
  const binary = dashboardBinary(binaryPath);
  if (!binary) return { started: false, reason: 'the local vestige dashboard binary was not found beside vestige-mcp' };
  try {
    const child = spawn(binary, ['dashboard'], { detached: true, stdio: 'ignore' });
    child.unref();
    return { started: true };
  } catch (error) {
    return { started: false, reason: error.message };
  }
}

async function offerObservatory(binaryPath) {
  if (JSON_OUTPUT || FLAGS.has('--no-dashboard')) return { offered: false, started: false };
  if (FLAGS.has('--open-dashboard')) return openDashboard(binaryPath);
  if (!process.stdin.isTTY) return { offered: false, started: false };

  print('');
  print(tone.cyan('  04  OPTIONAL LOCAL OBSERVATORY'));
  print(tone.dim('      Open the local dashboard to inspect real records as your agent creates them.'));
  const answer = (await ask(tone.cyan('  Open it now? [y/N]: '))).toLowerCase();
  if (answer !== 'y' && answer !== 'yes') return { offered: true, started: false };
  return { offered: true, ...openDashboard(binaryPath) };
}

async function main() {
  if (FLAGS.has('--help') || FLAGS.has('-h')) {
    console.log(usage());
    return;
  }
  banner();

  // Step 1: Find the binary
  print(tone.cyan('  00  LOCATE VESTIGE'));
  const binaryPath = findBinary();

  if (!binaryPath) {
    throw new Error('vestige-mcp was not found; install it first from https://github.com/samvallad33/vestige#install');
  }

  print(`      ${tone.green('FOUND')}  ${binaryPath}`);
  print('');

  // Step 2: Detect IDEs
  const detected = [];

  for (const [name, ide] of Object.entries(IDE_CONFIGS)) {
    if (ide.detect()) {
      detected.push({ name, ide });
    }
  }

  if (detected.length === 0) {
    throw new Error('no supported MCP clients found (Claude Code, Claude Desktop, Cursor, VS Code, OpenCode, Xcode, JetBrains, Windsurf)');
  }

  const selected = await selectClients(detected);
  if (selected.length === 0) {
    print('');
    print(tone.dim('  Vestige left your machine unchanged.'));
    return;
  }

  if (DRY_RUN) {
    const plan = { version: PACKAGE_VERSION, mode: 'dry-run', binaryPath, detected: detected.map(({ name }) => name), selected: selected.map(({ name, ide }) => ({ name, configPath: ide.configPath() })) };
    if (JSON_OUTPUT) console.log(JSON.stringify(plan, null, 2));
    else {
      print(tone.cyan('  02  PROPOSED LOCAL CONNECTIONS'));
      selected.forEach(({ name, ide }) => print(`      ${tone.violet('PLAN')}  ${name} → ${ide.configPath()}`));
      print('');
      print(tone.dim('  Dry run only. No configuration changed.'));
    }
    return;
  }

  // Step 3: Inject configs
  print('');
  print(tone.cyan('  02  WRITE LOCAL CONNECTIONS'));
  let configured = 0;
  let skipped = 0;
  const backups = [];
  const reviews = [];

  for (const { name, ide } of selected) {
    try {
      const injected = injectConfig(ide, name, binaryPath);
      if (injected.status === 'written') {
        print(`      ${tone.green('WRITTEN')}  ${name}`);
        configured++;
        if (injected.backupPath) backups.push(injected.backupPath);
        if (ide.note) {
          print(`         ${ide.note}`);
        }
      } else {
        skipped++;
      }
    } catch (err) {
      print(`      ${tone.amber('NEEDS REVIEW')}  ${name} — ${err.message}`);
      reviews.push({ name, reason: err.message });
    }
  }

  print('');

  // Step 4: Summary
  const result = {
    version: PACKAGE_VERSION,
    result: 'configuration-written',
    binaryPath,
    selected: selected.map(({ name }) => name),
    configured,
    skipped,
    backups,
    reviews,
    verification: 'pending: restart the selected client and confirm its MCP tools',
  };
  if (JSON_OUTPUT) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }

  renderProofPulse({ detected: selected.length, configured, skipped, backups: backups.length });
  print('');
  print('  1. Restart the selected client.');
  print('  2. Ask: "What MCP tools do you have access to?"');
  print('  3. Save a record with an exact tag, then recall that same tag in a new session.');
  print('');
  print(tone.violet('  Intelligence can run anywhere. Your records stay here.'));

  const dashboard = await offerObservatory(binaryPath);
  if (dashboard.started) print(tone.green('      LOCAL OBSERVATORY STARTED  Your browser will open on this machine.'));
  if (dashboard.reason) print(tone.amber(`      OBSERVATORY NOT STARTED  ${dashboard.reason}`));

  print('');
  print('Docs: https://github.com/samvallad33/vestige');
}

main().catch((error) => {
  console.error(`vestige init: ${error.message}`);
  process.exitCode = 1;
});
