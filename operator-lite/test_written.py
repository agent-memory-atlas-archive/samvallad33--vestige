import importlib.util, os, sys, json, tempfile
home = tempfile.mkdtemp(prefix="writtencheck-", dir=os.path.expanduser("~"))
os.environ["HOME"] = home; os.environ["OPERATOR_HOME"] = os.path.join(home, ".operator")
os.environ.pop("OPERATOR_AGENT_SESSION", None)
spec = importlib.util.spec_from_file_location("gate", (sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else os.path.join(os.path.dirname(os.path.abspath(__file__)), "operator-gate.py"))); g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
cfg = g.load_config(); work = os.path.join(home, "work"); os.makedirs(work)

def rules(path, body, tool="Write"):
    ti = {"file_path": os.path.join(work, path)}
    if tool == "Write":
        ti["content"] = body
    elif tool == "Edit":
        ti["old_string"] = "x"; ti["new_string"] = body
    else:
        ti["edits"] = [{"old_string": "x", "new_string": body}]
    return [h[0] for h in g.classify({"tool_name": tool, "tool_input": ti, "cwd": work}, cfg)[2]]

# (file the agent is writing, what it writes)  -- each is one of the eight ported detectors
STOP = [
    ("src/lib/supabase.ts", "export const admin = createClient(url, process.env.SUPABASE_SERVICE_ROLE_KEY)"),
    ("src/components/Admin.tsx", "const key = import.meta.env.VITE_SERVICE_ROLE"),
    ("src/firebase.js", "import admin from 'firebase-admin'"),
    ("src/app.jsx", "const admin = require('firebase-admin')"),
    ("config/gcp-key.json", '{\n  "type": "service_account",\n  "project_id": "prod"\n}'),
    ("app/keys.py", 'KEY = {"type": "service_account", "project_id": "prod"}'),
    ("server/auth.js", "const data = jwt.verify(token, secret, { algorithms: ['none'] })"),
    ("api/auth.py", 'payload = jwt.decode(token, options={"verify_signature": False})'),
    ("api/token.py", 'jwt.decode(token, key, algorithm="none")'),
    ("server/fetch.js", "const agent = new https.Agent({ rejectUnauthorized: false })"),
    ("scripts/sync.py", "r = requests.get(url, verify=False)"),
    ("client/http.go", "tr := &http.Transport{TLSClientConfig: &tls.Config{InsecureSkipVerify: true}}"),
    ("server/index.js", "process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'"),
    ("lib/http.php", "curl_setopt($ch, CURLOPT_SSL_VERIFYPEER, false);"),
    ("docker-compose.yml", "    volumes:\n      - /var/run/docker.sock:/var/run/docker.sock"),
    ("deploy/run.sh", "docker run -v /var/run/docker.sock:/var/run/docker.sock builder"),
    ("k8s/agent.yaml", "        hostPath:\n          path: /var/run/docker.sock"),
    ("app/config.py", "cfg = yaml.load(open('config.yml'))"),
    ("lib/settings.rb", "conf = YAML.load(File.read('settings.yml'))"),
    ("Dockerfile", "FROM node:22\nCOPY .env /app/.env\nCMD [\"node\", \"index.js\"]"),
    ("Dockerfile.prod", "ADD deploy/id_rsa /root/.ssh/id_rsa"),
    ("backend/Dockerfile", "COPY secrets/credentials.json /app/"),
    ("Dockerfile", "COPY .env.production /app/.env"),
]
EDITS = [("server/fetch.js", "  rejectUnauthorized: false,", "Edit"), ("scripts/sync.py", "requests.post(u, json=d, verify=False)", "MultiEdit")]
ALLOW = [
    ("server/admin.ts", "export const admin = createClient(url, process.env.SUPABASE_SERVICE_ROLE_KEY)"),
    ("supabase/functions/sync/index.ts", "const key = Deno.env.get('SUPABASE_SERVICE_ROLE_KEY')"),
    ("api/admin.js", "const admin = require('firebase-admin')"),
    ("src/firebase.js", "import { initializeApp } from 'firebase/app'"),
    ("src/Docs.tsx", "<code>service_role</code> keys must stay on the server"),
    ("src/lib/supabase.ts", "// never put the service_role key here\nexport const sb = createClient(url, anon)"),
    ("config/app.json", '{\n  "type": "module",\n  "name": "app"\n}'),
    ("server/auth.js", "const data = jwt.verify(token, secret, { algorithms: ['RS256'] })"),
    ("api/auth.py", 'payload = jwt.decode(token, key, algorithms=["HS256"])'),
    ("server/fetch.js", "const agent = new https.Agent({ rejectUnauthorized: true })"),
    ("scripts/sync.py", "r = requests.get(url, verify=True, timeout=10)"),
    ("scripts/sync.py", "r = requests.get(url, verify='/etc/ssl/corp-ca.pem')"),
    ("app/models.py", "user = User.objects.create(email=email, verify=False)"),
    ("app/forms.py", "form = SignupForm(data, verify=False)"),
    ("client/http.go", "tr := &http.Transport{TLSClientConfig: &tls.Config{MinVersion: tls.VersionTLS13}}"),
    ("docker-compose.yml", "    volumes:\n      - ./data:/var/lib/postgresql/data"),
    ("app/config.py", "cfg = yaml.safe_load(open('config.yml'))"),
    ("app/config.py", "cfg = yaml.load(f, Loader=yaml.SafeLoader)"),
    ("app/loader.js", "const doc = yaml.load(text)"),
    ("Dockerfile", "FROM node:22\nCOPY package.json package-lock.json ./\nRUN npm ci\nCOPY src ./src"),
    ("Dockerfile", "COPY .env.example /app/.env.example"),
    ("README.md", "Never set `rejectUnauthorized: false` or `verify=False` in production."),
    ("docs/security.md", "algorithms: ['none'] accepts any token"),
    ("tests/test_tls.py", "r = requests.get(url, verify=False)"),
    ("src/__tests__/auth.test.ts", "jwt.verify(token, secret, { algorithms: ['none'] })"),
    ("fixtures/bad.yaml", "path: /var/run/docker.sock"),
    ("examples/insecure.py", "cfg = yaml.load(open('x'))"),
    ("src/index.ts", "export function add(a: number, b: number) { return a + b }"),
    ("main.py", "print('hello')"),
]

bad = 0
for path, body in STOP:
    r = rules(path, body)
    if "OP-016" not in r:
        bad += 1; print("MISS   %-34s %-60s %s" % (path, body[:60].replace("\n", " / "), r))
for path, body, tool in EDITS:
    r = rules(path, body, tool)
    if "OP-016" not in r:
        bad += 1; print("MISS (%s) %-30s %s" % (tool, path, r))
for path, body in ALLOW:
    r = rules(path, body)
    if "OP-016" in r:
        bad += 1; print("FALSE  %-34s %-60s %s" % (path, body[:60].replace("\n", " / "), r))
print("stop cases %d, edit cases %d, allow cases %d, wrong %d" % (len(STOP), len(EDITS), len(ALLOW), bad))
import shutil; shutil.rmtree(home, ignore_errors=True)
sys.exit(1 if bad else 0)
