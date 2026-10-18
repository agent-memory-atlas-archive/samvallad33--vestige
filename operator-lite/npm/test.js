#!/usr/bin/env node
// npm test: what the published package carries. Runs the prepack bundler, then checks that the
// bundled gate is the repository's gate byte for byte, that it ships at package.json's version,
// and that every host the one-command install wires has its installer in the bundle.
// Nothing is installed and no hook is run.
"use strict";
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const pkg = __dirname;
const failures = [];
const check = (ok, what) => {
  console.log((ok ? "PASS: " : "FAIL: ") + what);
  if (!ok) failures.push(what);
};

const bundled = spawnSync(process.execPath, [path.join(pkg, "bin", "bundle.js")], { encoding: "utf8" });
check(bundled.status === 0, "prepack bundler exits 0 (" + (bundled.stdout + bundled.stderr).trim() + ")");

const repoGate = fs.readFileSync(path.join(pkg, "..", "operator-gate.py"));
const gatePath = path.join(pkg, "gate", "operator-gate.py");
const gate = fs.existsSync(gatePath) ? fs.readFileSync(gatePath) : Buffer.alloc(0);
check(gate.equals(repoGate), "the bundled gate is operator-lite/operator-gate.py byte for byte");

const version = (gate.toString("utf8").match(/^VERSION = "([^"]+)"/m) || [])[1];
const want = require(path.join(pkg, "package.json")).version;
check(version === want, "the bundled gate is " + version + ", package.json is " + want);

const cli = fs.readFileSync(path.join(pkg, "bin", "operator-lite.js"), "utf8");
const hosts = [...cli.matchAll(/\{ name: "([a-z-]+)", label:/g)].map((m) => m[1]);
check(hosts.length > 0, "the one-command install names " + hosts.length + " hosts");
for (const host of hosts) {
  check(fs.existsSync(path.join(pkg, "gate", "ports", host, "install.sh")), host + ": installer bundled");
}

console.log("\nnpm package: " + (failures.length ? failures.length + " failed" : "all checks pass"));
process.exit(failures.length ? 1 : 0);
