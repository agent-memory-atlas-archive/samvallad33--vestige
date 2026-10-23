#!/usr/bin/env node
'use strict';
// Host output adapter only. Every policy/permit/receipt decision stays in Operator Lite.
const fs = require('fs');
const { spawnSync } = require('child_process');
const args = process.argv.slice(2); const value = flag => args[args.indexOf(flag) + 1];
const client = value('--client');
function deny() {
  if (client === 'cursor') process.stdout.write('{"permission":"deny","user_message":"Operator stopped this tool call. Inspect its local receipt."}\n');
  else if (client === 'gemini') process.stdout.write('{"decision":"deny","reason":"Operator stopped this tool call. Inspect its local receipt."}\n');
  else process.stderr.write('Operator stopped this tool call. Inspect its local receipt.\n');
  process.exitCode = 2;
}
try {
  const config = JSON.parse(fs.readFileSync(value('--config'), 'utf8'));
  const input = fs.readFileSync(0, 'utf8');
  const result = spawnSync(config.python, ['-I', config.gate, 'hook', '--source', client], { input, encoding: 'utf8', timeout: 10000, maxBuffer: 256 * 1024 });
  // A crashed or missing guard never becomes permission to run.
  if (result.error || result.status !== 0) { if (result.stderr) process.stderr.write(result.stderr); deny(); }
  else if (client === 'cursor') process.stdout.write('{"permission":"allow"}\n');
  else process.stdout.write('{}\n');
} catch { deny(); }
