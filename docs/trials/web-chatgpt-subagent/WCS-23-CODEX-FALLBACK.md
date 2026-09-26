# WCS-23 Codex fallback (no web Send)

- Fixed review commit: `b76d5386de2a2b54830063dda5b90169d52a5c6c`.
- Web status: pre-Send stop on unsupported visible IAB tab creation in Luna; real reservation released, task ID retired, no ChatGPT tab or message. This is a Codex Astra subagent review, **not** a GPT-6 Pro or GitHub connector result.
- Scope: fixed-commit `AGENTS.md`, `docs/model-routing.md`, `docs/directions/2026-09-26-web-chatgpt-subagent-channel.md`, and `.agents/skills/web-chatgpt-subagent/SKILL.md`. Read-only static review; no runtime/CI/provider claim.

## Findings

1. **Controller browser authority conflicted.** At fixed-SHA `SKILL.md:16`, Controller was said never to operate the page; lines 24 and 37 assigned it safe-page verification and reply inspection/tab closure. The updated skill scopes the prohibition to dispatch and assigns result-time reply inspection/closure to the same Luna; Controller retains the explicitly required safe-page observation after a real cooldown.
2. **Foreground rule source was absent for no-context Luna.** At fixed-SHA `SKILL.md:20`, the skill referred Luna to a foreground invariant “in AGENTS.md,” but the fixed repository `AGENTS.md` contains no such rule. The updated skill includes the full sensitive-page authority and stop boundary directly in its handoff text.

No finding was made that the draft route had been silently enabled: fixed-SHA skill retained Owner adoption authority, and `docs/model-routing.md` had no web route. The skill also identified that the later one-shot 6 Pro decision superseded historical 5.6 trial instructions without rewriting those records. This review did not verify a live browser, GitHub connector, Actions run, or final project acceptance.
