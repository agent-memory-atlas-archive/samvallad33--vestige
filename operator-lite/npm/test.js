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

const corpora = fs.readdirSync(path.join(pkg, "..", "corpora")).filter((n) => n.endsWith(".json"));
for (const name of corpora) {
  const a = fs.readFileSync(path.join(pkg, "..", "corpora", name));
  const p = path.join(pkg, "gate", "corpora", name);
  check(fs.existsSync(p) && fs.readFileSync(p).equals(a), "corpus " + name + " bundled byte for byte");
}

// `npx operator-lite test` runs the bundled gate over its bundled corpus, in a throwaway gate home.
const py = ["python3", "python"].find((c) => spawnSync(c, ["-c", "import sys; assert sys.version_info >= (3, 9)"]).status === 0);
if (py) {
  const home = fs.mkdtempSync(path.join(pkg, ".test-home-"));        // beside the package, never a temp folder
  const r = spawnSync(process.execPath, [path.join(pkg, "bin", "operator-lite.js"), "test"],
    { encoding: "utf8", env: { ...process.env, HOME: home, USERPROFILE: home, OPERATOR_HOME: path.join(home, ".operator") } });
  const line = (r.stdout || "").trim().split("\n")[0] || "";
  check(r.status === 0 && /^corpus=guardfall cases=\d+ passed=\d+ failed=0$/.test(line),
    "npx operator-lite test replays the bundled corpus (" + line + ")");
  check(/the gate in this package/.test(r.stderr || ""), "npx operator-lite test runs the packaged gate");
  fs.rmSync(home, { recursive: true, force: true });
} else {
  check(false, "a Python 3.9+ to run the bundled gate");
}

const cli = fs.readFileSync(path.join(pkg, "bin", "operator-lite.js"), "utf8");
const hosts = [...cli.matchAll(/\{ name: "([a-z-]+)", label: "([^"]+)"/g)].map((m) => [m[1], m[2]]);
const expected = [
  ["cursor", "Cursor"],
  ["gemini-cli", "Gemini CLI"],
  ["windsurf", "Windsurf"],
  ["cline", "Cline"],
  ["goose", "Goose"],
  ["opencode", "opencode"],
  ["amazon-q", "Amazon Q"],
  ["roo-code", "Roo Code"],
  ["kilo-code", "Kilo Code"],
  ["continue", "Continue"],
  ["kiro", "Kiro"],
  ["zed", "Zed"],
  ["amp", "Amp"],
];
check(JSON.stringify(hosts) === JSON.stringify(expected),
  "HOSTS is " + hosts.map((h) => h[0] + " (" + h[1] + ")").join(", "));
const contracts = {
  "roo-code": ".roo/hooks.json",
  "kilo-code": ".kilocode/hooks.json",
  "continue": ".continue/hooks.json",
  kiro: ".kiro/settings/hooks.json",
  zed: "zed/hooks.json",
  amp: "amp/hooks.json",
};
for (const [name] of hosts) {
  const bundled = path.join(pkg, "gate", "ports", name, "install.sh");
  check(fs.existsSync(bundled), name + ": installer bundled");
  check(fs.existsSync(path.join(pkg, "gate", "ports", name, "README.md")), name + ": README bundled");
  if (contracts[name]) {
    const text = fs.readFileSync(bundled, "utf8");
    check(text.includes('PORT_NAME="' + name + '"'), name + ": install.sh names itself");
    check(text.includes(contracts[name]), name + ": install.sh targets " + contracts[name]);
    check(text.includes("hook --source ${PORT_NAME}"), name + ": install.sh guards the gate command");
    check(text.includes("[ -f "), name + ": install.sh file-guards a missing gate");
  }
}

console.log("\nnpm package: " + (failures.length ? failures.length + " failed" : "all checks pass"));
process.exit(failures.length ? 1 : 0);
