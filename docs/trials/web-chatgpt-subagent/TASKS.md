# Web ChatGPT channel trial cards

These cards test the route without touching R2-06a, main, release, signing, or secrets. The web seat may push only its assigned branch and may not merge a PR. A failed connector write is a trial result, not permission to change the boundary.

## WCS-01 — independent document audit

- Role: independent, read-only reviewer of the specified main commit.
- Repository: `taiyun668/gogoke`.
- Base: `main@5ec3d643a6015de34ef582b5d047e7cc7e46da43`.
- Read: `AGENTS.md`, `docs/model-routing.md`, `docs/directions/2026-09-26-web-chatgpt-subagent-channel.md` at the base commit.
- Question: identify concrete conflicts or missing operational boundaries between the direction and existing routing. Do not treat the direction as already adopted.
- Write: only branch `gpt/web-chatgpt-wcs01-audit`, file `docs/trials/web-chatgpt-subagent/WCS-01-RESULT.md`.
- Result: name the exact reviewed SHA, findings with source paths/lines, and unverified points. A GitHub commit is required; chat text alone is not a result.
- Stop: after one result commit, or immediately if branch creation/write permission fails. Do not change any other file.

## WCS-02 — isolated small construction

- Role: narrow test contributor.
- Repository: `taiyun668/gogoke`.
- Base: the web channel skill branch commit named in the dispatch starter; never the R2-06a branch.
- Read: `.codex/skills/web-chatgpt-subagent/scripts/ledger.py` and `.github/workflows/web-chatgpt-subagent.yml` at that base.
- Write: only branch `gpt/web-chatgpt-wcs02-tests`, under `.codex/skills/web-chatgpt-subagent/tests/`, plus `docs/trials/web-chatgpt-subagent/WCS-02-CHECKPOINT.md`.
- Task: add focused Python `unittest` cases that prove releasing an unsent reservation restores capacity and that simultaneous reservations from two seats cannot exceed one remaining slot. Existing tests cover disabled GPT-6 Pro, the 5.6 Sol Pro cap, and a switched reply. Use a temporary `LOCALAPPDATA` so no real account state is touched. Do not change the ledger implementation or workflow.
- Result: commit the tests and checkpoint; include exact base/head SHAs, commands and results. Codex will run and inspect actual GitHub Actions, including logs and nonzero test count.
- Stop: after one committed test package. If an implementation defect appears, report it in the checkpoint; do not expand the write scope.

## WCS-03 — forced fallback

- Owner of the route: Codex Controller.
- Exercise: set a trial deadline already in the past before any web send, then apply the skill's timeout path. The expected result is a local record saying `web_not_sent`, reason `deadline_expired`, and an actual Codex subagent handoff. This is a negative control; do not charge a web message or claim a web trial completed.
- If a real web send later produces a wrong model, record the actual tier, block the requested tier, and return the task to Codex. Do not deliberately consume messages to force a provider quota switch.

## WCS-04 — GitHub Actions connector readability

- Role: read-only GitHub Actions evidence checker.
- Repository: `taiyun668/gogoke`.
- Run: `36208879407`, workflow `web ChatGPT subagent skill`, from PR #48's first CI run.
- Read: run summary and its `ledger` job log via the GitHub connector. Do not infer log access from a green badge.
- Write: only branch `gpt/web-chatgpt-wcs04-actions`, file `docs/trials/web-chatgpt-subagent/WCS-04-RESULT.md`.
- Result: exact run/job IDs, conclusion, number of executed tests, and whether individual log lines were actually accessible. If logs cannot be read, say so without inventing a count. Include the source commit SHA and one result commit.
- Stop: after one result commit, or immediately on connector permission failure. Do not change any other file.

## WCS-05 — fresh skill review on GPT-5.6 Sol Pro

- Role: independent, read-only reviewer of the draft skill PR. This is trial evidence, not acceptance.
- Repository: `taiyun668/gogoke`; PR #48.
- Review commit: `29f7f434c7d24893edc25861f3c8de7b5f4d7193`.
- Read: the PR diff and the requirements at `docs/directions/2026-09-26-web-chatgpt-subagent-channel.md`, root `AGENTS.md`, and existing `docs/model-routing.md`.
- Question: find actionable defects in dispatch safety, model verification, GitHub-only handoff, private ledger semantics, and CI coverage. Distinguish implementation failures from unverified trial questions. Do not treat the web seat as a high-risk acceptance auditor.
- Write: only branch `gpt/web-chatgpt-wcs05-skill-audit`, file `docs/trials/web-chatgpt-subagent/WCS-05-RESULT.md`.
- Result: exact reviewed commit, file/line findings, and unverified points. One result commit required.
- Stop: after one result commit, or immediately on connector permission failure. No code change, merge, publish, secret, or signing action.
