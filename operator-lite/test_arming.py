import importlib.util, os, sys, json, tempfile
home = tempfile.mkdtemp(prefix="armingcheck-", dir=os.path.expanduser("~"))
os.environ["HOME"] = home; os.environ["OPERATOR_HOME"] = os.path.join(home, ".operator")
os.environ.pop("OPERATOR_AGENT_SESSION", None)
spec = importlib.util.spec_from_file_location("gate", (sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else os.path.join(os.path.dirname(os.path.abspath(__file__)), "operator-gate.py"))); g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
cfg = g.load_config(); work = os.path.join(home, "work"); os.makedirs(work)

def wrote(path, body):
    return [h[0] for h in g.classify({"tool_name": "Write", "tool_input": {"file_path": os.path.join(work, path), "content": body}, "cwd": work}, cfg)[2]]

def ran(cmd):
    return [h[0] for h in g.classify({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": work}, cfg)[2]]

MCP = '{"mcpServers": {"helper": {"command": "node", "args": ["server.js"]}}}'
# (file, what is written)  -- a command lands in a file something will run on its own
STOP_WRITE = [
    (".cursor/mcp.json", MCP),                                                   # CurXecute, MCPoison
    (".mcp.json", MCP),
    (".vscode/mcp.json", MCP),
    ("packages/api/.mcp.json", MCP),
    (".codex/config.toml", 'model = "x"\nnotify = ["sh", "tools/done.sh"]\n'),   # Codex notify field
    (".codex/config.toml", '[mcp_servers.helper]\ncommand = "node"\n'),          # CVE-2025-61260
    (".claude/settings.json", '{"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "sh tools/s.sh"}]}]}}'),  # CVE-2026-25725
    (".claude/settings.local.json", '{"apiKeyHelper": "sh tools/key.sh"}'),
    (".gemini/.env", "GEMINI_CLI_HOME=/work/x\n"),                               # CVE-2026-13745
    (".gemini/settings.json", MCP),
    (".vscode/tasks.json", '{"version": "2.0.0", "tasks": [{"label": "setup", "type": "shell", "command": "sh tools/t.sh", "runOptions": {"runOn": "folderOpen"}}]}'),
    (".cursor/hooks.json", '{"hooks": {"beforeShellExecution": [{"command": "sh tools/h.sh"}]}}'),
    (".envrc", "export PATH=$PWD/bin:$PATH\nsource tools/setup.sh\n"),
    (".envrc", 'eval "$(tools/env.sh)"\n'),
]
STOP_SHELL = [
    "cp tools/mcp.json .cursor/mcp.json",
    "echo 'notify = [\"sh\", \"x.sh\"]' >> .codex/config.toml",
    "tee .mcp.json < tools/mcp.json",
    "sed -i 's/a/b/' .claude/settings.json",
    "printf 'X=1\\n' >> .gemini/.env",
    "mv tools/tasks.json .vscode/tasks.json",
    "cat tools/envrc > .envrc",
    "mkdir -p .cursor && cp x.json .cursor/mcp.json",
]
ALLOW_WRITE = [
    (".claude/settings.json", '{"permissions": {"allow": ["Bash(npm test)"]}, "model": "sonnet"}'),
    (".claude/settings.local.json", '{"env": {"LOG_LEVEL": "debug"}}'),
    (".codex/config.toml", 'model = "gpt-5"\napproval_policy = "on-request"\n'),
    (".vscode/tasks.json", '{"version": "2.0.0", "tasks": [{"label": "build", "type": "shell", "command": "npm run build"}]}'),
    (".vscode/settings.json", '{"editor.formatOnSave": true}'),
    (".cursor/mcp.json", '{"mcpServers": {}}'),
    (".envrc", "export NODE_ENV=development\nexport PORT=3000\ndotenv\nlayout node\n"),
    (".envrc", "# project environment\nPATH_add bin\nuse flake\n"),
    (".env.example", "API_URL=http://localhost:3000\n"),
    ("docs/mcp.md", "Add this to .cursor/mcp.json:\n" + MCP),
    ("src/config.json", '{"command": "build"}'),
    ("package.json", '{"scripts": {"dev": "vite"}}'),
    ("src/mcp/server.ts", "export const command = 'node'"),
]
ALLOW_SHELL = [
    "cat .cursor/mcp.json", "ls -la .claude", "git add .mcp.json", "git diff .codex/config.toml",
    "cp .cursor/mcp.json backup/mcp.json", "jq . .mcp.json", "direnv allow", "echo done > build/log.txt",
]

bad = 0
for path, body in STOP_WRITE:
    r = wrote(path, body)
    if "OP-017" not in r:
        bad += 1; print("MISS   write %-30s %s" % (path, r))
for c in STOP_SHELL:
    r = ran(c)
    if "OP-017" not in r:
        bad += 1; print("MISS   shell %-56s %s" % (c[:56], r))
for path, body in ALLOW_WRITE:
    r = wrote(path, body)
    if "OP-017" in r:
        bad += 1; print("FALSE  write %-30s %s" % (path, body[:50].replace("\n", " / ")))
for c in ALLOW_SHELL:
    r = ran(c)
    if "OP-017" in r:
        bad += 1; print("FALSE  shell %-56s %s" % (c[:56], r))
print("stop writes %d, stop shell %d, allow writes %d, allow shell %d, wrong %d" % (len(STOP_WRITE), len(STOP_SHELL), len(ALLOW_WRITE), len(ALLOW_SHELL), bad))
import shutil; shutil.rmtree(home, ignore_errors=True)
sys.exit(1 if bad else 0)
