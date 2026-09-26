# WCS-02 checkpoint

- Task: `WCS-02` — isolated small construction.
- Role: narrow test contributor.
- Branch: `gpt/web-chatgpt-wcs02-tests`.
- Exact base SHA: `b7012434ddb9040e42cf7730d7ddea87e5b04ab6`.
- Head: the single package commit containing this checkpoint and the tests. Its exact SHA is returned by GitHub when the commit is created and is reported in the dispatch result; embedding a commit's own SHA inside that same commit would change the SHA.
- Governing test/build note: `docs/governance/gogoke-build-and-release.md` permits Python-script tests under a signed Python runtime and requires native compilation evidence to stay in cloud CI. No native build is added here.

## Scope

Read at the exact base:
- `.codex/skills/web-chatgpt-subagent/scripts/ledger.py`
- `.github/workflows/web-chatgpt-subagent.yml`
- existing `.codex/skills/web-chatgpt-subagent/tests/test_ledger.py`

Written only:
- `.codex/skills/web-chatgpt-subagent/tests/test_ledger_reservations.py`
- `docs/trials/web-chatgpt-subagent/WCS-02-CHECKPOINT.md`

No ledger implementation, workflow, `main`, R2-06a, release, secret, or signing files were changed.

## Added checks

1. With 19 completed `high` events, reserve the twentieth slot, prove a second reservation is blocked, release the still-unsent reservation, then prove capacity is restored by a new successful reservation.
2. With one `high` slot remaining, start reservations from `seat-a` and `seat-b` together. Require exactly one success and one daily-cap failure, then verify the persisted ledger contains exactly one reservation.

Every test uses a fresh temporary `LOCALAPPDATA`; no real account ledger is touched.

## Commands and results

- `python -m py_compile .codex/skills/web-chatgpt-subagent/tests/test_ledger_reservations.py`
  - Result: PASS, exit 0.
- `python -m unittest discover -s .codex/skills/web-chatgpt-subagent/tests -p 'test_*.py' -v`
  - Result in this web-channel construction environment: not executed as evidence because `ledger.py` imports Windows-only `msvcrt`.
  - Canonical execution remains the existing `windows-latest` workflow command above. Codex will inspect the real Actions run, logs, and nonzero test count as required by the task card.

## Defect status

No implementation defect was identified by static inspection while constructing these cases. Runtime behavior is pending the required Windows Actions evidence; if Actions exposes an implementation defect, this checkpoint does not authorize changing the ledger.

## Round-trip note

The task card, ledger implementation, workflow, existing tests, and build/test governance were read before any repository write. The change is one focused test file plus this checkpoint, avoiding an implementation or CI edit and avoiding an extra round trip before the single package commit.
