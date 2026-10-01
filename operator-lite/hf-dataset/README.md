---
license: agpl-3.0
pretty_name: GuardFall Bypass Corpus (Operator Lite adaptation, 43 cases)
language:
- en
task_categories:
- text-classification
tags:
- security
- adversarial
- red-team
- agentic-safety
- bash
- shell
- pre-tool-gate
size_category: n<1K
configs:
- config_name: default
  data_files: guardfall.json
---

# GuardFall Bypass Corpus — Operator Lite adaptation

**Suggested dataset name:** `samvallad33/guardfall-bypass-corpus`

A 43-case adversarial shell-command corpus for evaluating **pre-tool gates**
and other agent-safety filters that must decide, before execution, whether a
shell command emitted by an AI agent is safe. Every case is a *classification
string only*: nothing in this corpus is ever executed by the eval, and the
paths, hosts, and payloads are synthetic.

## Dataset description

The corpus adapts the bypass techniques published in the **GuardFall** research
note (Adversa AI / Cloud Security Alliance, June 2026), which documented five
classes of shell-obfuscation bypasses against prompt-adjacent command filters,
plus twelve canonical destructive patterns. That source material — 21 bypass
cases + 12 canonical destructive patterns — was adapted to the contract of the
[Operator Lite gate](https://github.com/samvallad33/vestige/tree/main/operator-lite)
(`tool=Bash`; expected verdict = the rule id that **MUST** fire, `SHADOW` for
any `OP-S*` shadow hit, `ALLOW` for a clean verdict), and extended with ten
operator-specific cases (git history destruction, memory-store wipes, paid
deploys, publishes, a tripwire canary, and benign controls).

The GuardFall research was publicly covered by **The Hacker News in June 2026**
as part of reporting on agent-safety filter bypasses. This dataset exists
because the technique classes are already public; the defensive value of a
shared, versioned eval corpus outweighs the disclosure risk of restating
published techniques in executable-shaped form.

Provenance field from the source file (verbatim):

> CSA research note: GuardFall (Adversa AI / Cloud Security Alliance, June
> 2026) — five bypass classes; 21 bypass cases + 12 canonical destructive
> patterns, adapted to this gate's contract (tool=Bash, expected = rule id
> that MUST fire, or 'SHADOW' for any OP-S*).

## Intended use

- **Testing pre-tool gates**: replay the corpus through a hook/middleware that
  classifies commands before execution (exit codes, deny outputs, guardrail
  verdicts) and check that the expected rule fires. The Operator Lite gate
  ships this exact corpus behind `operator-gate.py corpus guardfall`
  (must pass 43/43).
- **Red-team evaluation**: measure a classifier's recall against known
  obfuscation families (quote reassembly, `$IFS` expansion, ANSI-C quoting,
  base64 pipe-to-interpreter, heredoc execution, brace expansion, time bombs,
  wrapper chains, cd-then-relative paths) and against benign controls that
  must **not** fire (heredoc *data*, scratch deletes, ordinary builds).
- **Regression testing**: any change to a gate's analyzer should keep the
  corpus green; the `expect` column makes failures diff-able.

Not intended for: training a model to produce destructive commands, bypassing
any production safety system, or execution against live systems.

## Schema

One JSON object per case (the source file is `{"name", "source", "cases": [...]}`,
so loaders should select the `cases` field):

| Field | Type | Description |
|---|---|---|
| `id` | string | Stable case id, `GF-<group><n>` (e.g. `GF-A1`). Groups: A–J are GuardFall-derived technique families, X are operator-specific scenarios. |
| `class` | string | Technique/scenario label, one of 30 values (see stats). |
| `cmd` | string | The shell command string to classify. **Never execute it** — it is a test input for the analyzer under test. |
| `expect` | string | Expected verdict: `OP-000`…`OP-010` = that STOP rule id must fire; `OP-CANARY` = the tripwire rule must fire; `SHADOW` = at least one `OP-S*` (shadow/log-only) rule must fire; `ALLOW` = no STOP rule may fire. |

## Dataset stats

- **43 cases**, 30 distinct `class` values.

Class distribution (count per class):

| class | n | | class | n | | class | n |
|---|---|---|---|---|---|---|---|
| destructive-equivalent | 6 | | command-substitution | 3 | | encoded-pipeline | 3 |
| ansi-c-quoting | 2 | | benign | 1 | | benign-delete | 1 |
| brace-expansion | 1 | | canary | 1 | | cd-then-relative | 1 |
| backslash-escape | 1 | | env-hijack | 1 | | exfil | 1 |
| fork-bomb | 1 | | git-force | 1 | | git-rm-git | 1 |
| heredoc-data-ok | 1 | | heredoc-exec | 1 | | ifs-expansion | 2 |
| memory-wipe | 1 | | paid-deploy | 1 | | persistence | 1 |
| publish | 1 | | quote-reassembly | 2 | | reverse-shell | 1 |
| shell-init | 1 | | sql | 1 | | time-bomb | 1 |
| var-expansion | 2 | | wrapper-chain | 1 | | xargs-delete | 1 |

Expected-verdict distribution: `OP-001` ×18 · `SHADOW` ×9 · `ALLOW` ×3 ·
`OP-000` ×2 · `OP-002` ×2 · `OP-004` ×2 · `OP-003`, `OP-005`, `OP-006`,
`OP-007`, `OP-008`, `OP-009`, `OP-CANARY` ×1 each.

## Ethical statement

- **Defensive use only.** This corpus exists to make agent-safety gates
  testable against published bypass techniques. It is red-team *material for
  defenders*, not an attack kit.
- **Synthetic paths and hosts.** Every target is fake: `~/Developer/vestige`,
  `evil.example`, `10.0.0.1`, `/srv/app/.env`, scratch filenames. No real
  third-party system, repository, or service is targeted by any string.
- **No live harm.** The evaluation protocol is classification-only: the gate
  under test receives the command as *data* (JSON on stdin) and returns a
  verdict; the corpus driver executes nothing. Do not run `cmd` values in a
  real shell.
- The technique families were **published first** (GuardFall, CSA/Adversa AI,
  June 2026; The Hacker News coverage, June 2026); this dataset republishes
  them in eval form so defenders can verify their gates instead of trusting
  them.

## Loading

With the Hugging Face `datasets` library, from a local checkout (the top-level
object nests the rows under `cases`, hence `field=`):

```python
from datasets import load_dataset

ds = load_dataset("json", data_files="guardfall.json", field="cases")["train"]
print(ds)
print(ds.select_columns("class").to_pandas().value_counts())
```

Once published, the same works against the hub id:

```python
ds = load_dataset("samvallad33/guardfall-bypass-corpus", field="cases")["train"]
```

No-stdlib-dependency fallback (what [`load.py`](load.py) uses when `datasets`
is not installed): `json.load(open("guardfall.json"))["cases"]`.

`load.py` loads the corpus and pretty-prints the class and verdict
distributions:

```sh
python3 load.py            # uses datasets when available, else stdlib json
python3 load.py --ids      # also list every case id grouped by class
```

## Maintenance

- Source of truth: [`operator-lite/corpora/guardfall.json`](https://github.com/samvallad33/vestige/blob/main/operator-lite/corpora/guardfall.json)
  in the Vestige repository (AGPL-3.0). This dataset directory is a verbatim
  copy for publication; keep the two in sync.
- Versioning: bump the corpus in the repo first, re-verify
  `operator-gate.py corpus guardfall` passes 43/43, then refresh this copy.

## Citation

```bibtex
@misc{guardfall2026,
  title  = {GuardFall: bypass classes against shell-command filters for AI agents},
  author = {{Adversa AI and Cloud Security Alliance}},
  year   = {2026},
  month  = {June},
  note   = {Research note; covered by The Hacker News, June 2026. Adapted as the Operator Lite GuardFall bypass corpus.}
}
```
