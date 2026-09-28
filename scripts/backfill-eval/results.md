# Backfill top-3 evaluation — 20 closed issues with known fixing commits

Run: 2026-09-28T19:15:58.794760+00:00 · corpus: scripts/backfill-eval/corpus.json · window: 365d · lookback: 365d · hits in top-3: **1/20**

| issue | result | rank | causing commit(s) | detail |
|---|---|---|---|---|
| OpenHands/OpenHands#1187 | miss | — | 1115b60a745e, 342302ceef8e, aed82704a947, b654b00aa1c5 | causes=3 rejected=3 gap=no |
| pallets/click#2952 | miss | — | 361915bbc3cc, 4ece17a107d7, 4fd2fea0db42, 9b24c115e14d, ba770cbc96f5, f14b75063fb3, fd183b2ced1c | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (github.com/pallets/click/issues/2897). A commit or note touc |
| encode/httpx#96 | miss | — | 56a79432065f, bb8697011d1b, c9747aa35731 | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (project/venv/lib/python3.7/site-packages/http3/dispatch/conn |
| encode/httpx#634 | miss | — | 499de51f2b47 | causes=3 rejected=3 gap=no |
| pallets/jinja#515 | miss | — | 4083d96160e1, 46acbf02ed9a | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (github.com/mitsuhiko/jinja2/commit/46acbf02ed9ab58c7a92553c9 |
| pallets/jinja#1168 | miss | — | 2a8515d2e53a, 40e70b820c82, e08dadd22056 | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (github.com/pallets/jinja/blob/master/src/jinja2/loaders.py,  |
| go-chi/chi#238 | miss | — | 02e6bbdfd146, 168b7ac573ce, 20a9568fa693, 34bf22a9fefc, 524a02044614, 7e6b856011d4, 9c239d53d23d, b8567b6442e2 | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities ((the failure names no entities)). A commit or note touching  |
| go-sql-driver/mysql#108 | miss | — | 04653f211726 | causes=3 rejected=3 gap=no |
| go-sql-driver/mysql#257 | hit | 1 | 4834ee6ac209, 6b302cc76eab, 734d65ec97a2, 81d54a2bbf46 | causes=3 rejected=3 gap=no |
| jackc/pgx#2246 | miss | — | 228cfffc20bd | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (retry/fallback, pg_hba.conf, github.com/jackc/pgx/issues/158 |
| jackc/pgx#1481 | ground truth unreachable | — | — | no commit found touching the fix files before the issue |
| golang-jwt/jwt#158 | miss | — | 80625fb51660, a2aa65562708 | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (time.unix, jwt.newnumericdate). A commit or note touching on |
| golang-jwt/jwt#71 | miss | — | 2ebb50f957d6 | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (github.com/dgrijalva/jwt-go/issues/360). A commit or note to |
| jpadilla/pyjwt#709 | miss | — | 98620ab2a396, a988e1a11e5a | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (signing_key.key, stackoverflow.com/questions/50002149/why-p- |
| jpadilla/pyjwt#147 | ground truth unreachable | — | — | no commit found touching the fix files before the issue |
| strawberry-graphql/strawberry#4142 | miss | — | 6cf245cddb1c, 938f940f22f0 | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (strawberry_django.filter_type, strawberry.lazy, errors.straw |
| ron-rs/ron#357 | miss | — | 4070d4d5128f, 412bcf662fda, 5e0f3c5e0073, a8a35008fa9a, d36a191594ac | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities ((the failure names no entities)). A commit or note touching  |
| ron-rs/ron#115 | ground truth unreachable | — | — | no commit found touching the fix files before the issue |
| pallets/werkzeug#682 | miss | — | 135b2ab7f178, 8bc8b1a46622 | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (app.run, werkzeug.run_simple, werkzeug._reloader). A commit  |
| pallets/werkzeug#875 | miss | — | 2b2d921eea7d, b8fd87ed281c, deabf79417fc | causes=0 rejected=3 gap=yes: No record within 365d shares any of the failure's entities (github.com/mitsuhiko/werkzeug/blob/master/werkzeug/formparse |

## Notes
- OpenHands/OpenHands#1187: Substitute for mandated issue #16727, which is still OPEN with an unmerged fix PR (#16806); reported as a deviation, not a silent swap.
- encode/httpx#96: Files use the http3/ layout because the package was renamed http3 -> httpx after these fixes.
- go-sql-driver/mysql#257: Fix landed 3.1 years after the report; ground truth is the verified closing commit.
- jackc/pgx#1481: Body names the version pair v4.17.2 -> v5.2.0: the version-range path is exercised on this entry.
- golang-jwt/jwt#71: Documentation defect: the fix edits doc comments only.
- jpadilla/pyjwt#709: Fix landed 2.9 years after the report; ground truth is the verified closing commit.
- ron-rs/ron#357: Fix landed 1.5 years after the report via PR #451 (internally tagged and untagged enums).
- ron-rs/ron#115: Fix landed 4.9 years after the report (PR #455 adds tests/115_minimal_flattening.rs, named after this issue).
- pallets/werkzeug#875: Fix landed 4.9 years after the report (PR #2017 multipart rewrite).

Ground truth is derived SZZ-style from the verified fixing commit: for each
file the fix touched, the last commit touching that file before the issue was
filed. Every corpus entry runs; results are reported as produced. The mandated
OpenHands issue #16727 is OPEN with an unmerged fix PR, so it cannot anchor a
known-fix row; the nearest-area OpenHands issue with a verified fix (#1187) is
entry #1 and the deviation is disclosed here rather than silently swapped.

Known limitations: issues whose causing commits predate the ingested window,
fixes whose files were renamed, and issues whose bodies name no file at all
are expected misses — the gap report is the designed output for those.
