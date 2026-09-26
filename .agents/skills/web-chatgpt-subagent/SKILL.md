---
name: web-chatgpt-subagent
description: Draft, serial GPT-6 Pro web route for self-contained Astra-class GitHub tasks, operated directly by the Controller.
---

# Web ChatGPT subagent — Controller runbook

PR #48 remains **draft**; this skill does not enable routine routing or amend `docs/model-routing.md`. Owner authorized one GPT-6 Pro Send trial. WCS-22, WCS-23, and WCS-24 stopped before Send, so that authorization remains available. The Controller operates the Codex in-app browser **directly in one foreground tab**; there is no Luna courier or browser-operation script. One web task uses one task ID, one ChatGPT tab, and at most one Send.

## 1. Eligibility and economic gate

Check **every** condition before preparing a card; any failure keeps the task in Codex:

1. The task would otherwise go to an **Astra** subagent, and the Controller expects its Codex cost clearly above the web route's fixed overhead. “Astra task” is necessary, never sufficient.
2. One complete task description plus repository files can finish it. It needs no inherited Controller context, `fork_turns`, or mid-task question.
3. It needs no local signing, Owner Windows 11/Smart App Control test, real local CLI integration, or local chore.
4. It is outside the critical path; waiting roughly half an hour does not block the Controller's next step.
5. GitHub can prove the result: a conclusion file naming an exact reviewed commit or an assigned result branch, plus a **real CI run with more than zero executed tests**.

Typical candidates: an independent audit of a frozen SHA, final fixed-commit review, design review against complete repository documents, or a self-contained construction slice with CI. Keep iterative diagnosis, ordinary PR review suited to Sol, medium/small work, and urgent tasks in Codex.

Observed relative prices were Astra : Sol : Luna ≈ **100 : 20 : 1**. WCS-20's page dispatch cost in this Controller context was about **$0.72/task** (Controller $0.68, Luna $0.04), independent of task size; comparison medians were Astra about $4.1, Sol large about $1.8, Sol medium about $0.8, and Luna about $0.04. Direct Controller dispatch is estimated at **$1.6–1.9/task** and must be measured in a real direct trial; use the estimate only to screen for a clear saving over Astra. Before dispatch, record in the **private ledger reservation** a one-line economic reason and `--original-seat astra`. Before any Send, check the current Controller request's input context is below **272,000 tokens**; if close, compact the thread first rather than paying the higher tier above that size. After a real task, report Controller request counts, input/cached/output tokens **separately for Send and page patrol**, and Send-to-result-commit time.

## 2. Model, accounting, and authority

Only **GPT-6 Pro** is a dispatch target. Move `思考强度` to the far right and require the **resolved composer label `6 Pro` immediately before Send**. A menu option, old conversation title, or `Latest` is not enough. The pre-Send label cannot prove which model served the reply. The page may automatically fall back to GPT-5.6 Thinking Medium after Pro exhaustion, as described in [OpenAI's Help Center](https://help.openai.com/en/articles/20001354-gpt-56-and-gpt-6-pro-in-chatgpt). Inspect any per-reply model field read-only if it exists; a “Retry with…” option describes a future choice, not the served model. Record `unverified` when no served-model field exists. On visible limit or downgrade, block 6 Pro in the ledger until an observed reset; do not accept a downgraded reply as Pro work.

The private ledger counts **only sends made through this skill from its reconciled start** and caps them at **25 per local day**. The account's reported 6 Pro **200/week** allowance is shared with Owner manual use and other web construction seats; their use is unknown to this ledger. A new/lost `state.json` is closed to dispatch until the Controller runs `reconcile --source NOTE`, recording “baseline begins now; earlier usage unknown.” No historical usage request to Owner is needed. `task-ids.jsonl` retires IDs across state loss. `enable-6-pro` records the Owner's trial decision locally; it does not enable project routing. Do not commit account usage numbers, reset details, conversation links, or the private ledger to this public repository.

## 3. One message, no follow-up

Send **one complete message** per task. After clicking Send once, never click Send again for that ID, including after a tool error, a stalled reply, or an uncertain click. A new user turn **and** a new assistant turn in the saved conversation are Send evidence; a cleared composer or Stop button alone is insufficient. Mark an uncertain click `mark-uncertain` and investigate read-only. No web follow-up and no new ID to resend the same work after it reaches GPT.

If GPT lacks information or meets a real persistent permission/platform blocker, it writes the blocker in the assigned result file, commits if possible, and ends instead of asking in chat. Ordinary tool, CI, or remote-write failures **inside the same task** require continued diagnosis and repair within the assigned branch. A blocked, incomplete, or unacceptable result goes to a Codex subagent **with any web result file as input**, without a web follow-up. The ledger records one-shot **task completion** separately from `served_model_status`; a task result with unknown served model never proves Pro generated it. A low one-shot success rate narrows eligibility.

Pre-Send mistakes, including a wrong card path or a control not yet loaded, are **not** follow-ups to GPT: correct and recheck before clicking Send. A released reservation retires its ID, so the next authorized trial uses a new ID. On deadline, `finish-task --timed-out --fallback-agent NAME --reason REASON` voids the ID immediately. A result committed afterward is recorded only with `late-result`; never adopt, merge, or count it as success.

## 4. Controller's browser procedure

1. Prepare the [task card template](#7-task-description-template), record current remote `main` SHA, and verify the card's exact remote commit. **Before committing each card**, run `python tools/check-public-source.py --file PATH_TO_CARD --self-test --quiet` and require `leaks=0`, then run `python tools/check-public-source.py --quiet` for the already committed tree. The default scanner reads HEAD and misses an uncommitted card. Keep user-profile paths out of the public card. Read hidden ledger `status`: no cooldown, no active task, reconciled baseline, Pro enabled, local cap available. Count IAB ChatGPT tabs: **zero before opening**; if one exists, identify and finish/clean it rather than opening another; if more than one, stop and clean completed tabs. Reserve exactly once with hidden `reserve --tier gpt-6-pro --task ID --seat web-channel --original-seat astra --economics "ONE-LINE REASON"`.
2. Open **one foreground** Codex IAB tab to a fresh non-Project ChatGPT conversation. A zero-tab IAB is a normal start; the *possibility* of login is not a reason to stop before opening. Confirm the new-chat URL and zero message turns; do **not** require a legacy Chat radio control. If actual login, verification, suspicious activity, or an unrelated draft appears, follow §5. Let the page settle and reread it if the tier pill is missing from the first accessibility state; it can appear later. If the pill initially says `中`, open it, inspect `思考强度` and nested `选择模型`, set the far-right Pro strength, and verify the final composer label says **`6 Pro`**. Do not infer the model family from `中` alone. Check limit/downgrade and “已使用记忆” indicators. Paste the [single starter template](#6-web-gpt-starter-template), verify task ID, immutable card SHA, branch, text, final `6 Pro`, and exactly one ChatGPT tab; click **Send once**.
3. Confirm new user and assistant turns, then hidden `mark-sent ID --url URL`. If the click outcome is uncertain, hidden `mark-uncertain ID --reason CODE` and do not retry. If abandoning **before** Send after entering text, clear the composer in that same tab and read it back empty **before** `release ID` and closing it. If cleanup cannot be verified, retain the reservation and report the blocker; do not silently free the slot. A previously existing unrelated draft is preserved, not cleared. After a confirmed Send, **keep the tab open** and mark it for handoff if the browser tool requires it.
4. Start the GitHub watcher using §5's hidden wrapper. While GPT is running, return to the **same page about every five minutes** and read only its state; do not click page controls. The watcher checks the assigned GitHub branch about every 90 seconds, then every five minutes after 20 minutes, and should notify the Controller when the result appears. GPT retrying a GitHub write while it is still generating is normal; a temporarily unchanged answer is not a reason to resend. A page error, stopped generation with no further actions, expired connector authorization, limit, or downgrade is an interruption. **Do not click Retry, Continue generating, or any other button**; record failure, keep evidence, and hand the work to Codex. Report login loss explicitly. A connector write-confirmation prompt is handled under §5, not clicked. If the watcher fails to notify by deadline, inspect its receipt once for candidate, notification error, or deadline expiry; avoid model-driven GitHub polling. Update Owner only at confirmed Send, verified GitHub result, or abnormal stop, not for unchanged patrols.
5. Verify the real ledger and GitHub branch, exact commit/parent, allowed diff, result content, reviewed SHA, and **real Actions logs showing >0 executed tests**. Compare remote `main` to its pre-dispatch SHA; an unexpected commit stops this route and is reported to Owner. Read any reply-served-model location without clicking retry. Only **after** GitHub result confirmation, close that task's ChatGPT tab and verify the IAB inventory. Then hidden `approve-result --task ID --commit SHA` and `finish-task --task ID --result-commit SHA`. A GPT claim or watcher notification alone is not acceptance. A tab that disappears before inspection leaves model identity and explicit post-result close unverified.

Page lessons came from WCS-01–WCS-21: visible IAB tabs failed in **subagent** threads (WCS-08), but this route now uses Controller foreground; blank-chat pages may omit a Chat radio (WCS-09); tier controls may load after the initial state (WCS-10); the task card lives at its own immutable card-branch SHA, not the older review SHA or unwritten result branch (WCS-13); `中` may hide nested model controls (WCS-16); unsent drafts can persist (WCS-17/21); and marking the GitHub-card tab does not preserve the ChatGPT conversation (WCS-11/12/14). These are checks on the actual page, not a fixed browser script.

## 5. Safety, hidden processes, and stop conditions

One active web task and one ChatGPT tab maximum. Never create a second tab as a recovery shortcut. On “可疑活动” or security verification, immediately stop all web dispatch, hidden `cooldown --reason suspicious_activity` or `security_verification`, notify Owner, and route queued work to Codex. The state persists across restarts for at least **24 hours** with no retry or automatic recovery. After 24 hours only the Controller may verify a safe page and explicitly clear it. Demonstrations use temporary `LOCALAPPDATA`, never the real ledger.

For login, authentication, MFA, QR/verification codes, passwords, permissions, or user-interaction steps, the only authoritative page is the Owner's currently focused Codex IAB URL in the latest Owner ambient state. An automation tab is not proof Owner sees it. Never claim an input page is visible or ready without a later Owner turn reporting that exact focused URL. Stop at an unproven sensitive-page boundary; do not automate it or ask Owner to use a background tab. If a GitHub connector **write confirmation** (“是否允许” or similar) appears, **do not click it**. Preserve the exact text or screenshot, stop, and ask Owner to decide future handling. A visible limit/downgrade stops Pro until observed reset. A real security warning takes priority over ordinary failure handling.

The web GPT may push **only** to the branch assigned in its card; no push or merge to `main`, release, publication, tag, secrets, or signing. Branch instructions alone are not a connector permission sandbox: verify changed paths and `main` after every result. Do not change frozen project gates or treat web self-report as acceptance. Result CI must have **actual executed tests >0**; skip/0-test/green badge alone does not pass.

No separate OS window may appear on Owner's screen, including a brief console. Browser work stays inside the Codex foreground IAB tab. Invoke `ledger.py` only through `.agents/skills/web-chatgpt-subagent/scripts/Invoke-LedgerHidden.ps1`, resolved locally from repository root. It starts `pythonw.exe` with `CreateNoWindow=true` and `WindowStyle=Hidden`; require `exit_code=0` and `console_window_present=false`. Start `.agents/skills/web-chatgpt-subagent/scripts/Start-WatcherHidden.ps1` the same way; it launches `gh` and `codex queue` with `CREATE_NO_WINDOW`. Do not run `python.exe ledger.py` or a visible `Start-Process`. Store all private receipts under `%LOCALAPPDATA%\gogoke\web-chatgpt-subagent`; no user-profile absolute path belongs in the public task card.

## 6. Web GPT starter template

Use one message by filling **all** fields below from the committed task card; verify the card at `{CARD_SHA}`. The starter carries the common construction discipline even when GPT has not yet opened the card:

```text
任务 {TASK_ID}；角色 {ROLE}；仓库 {REPOSITORY}；固定基准提交 {BASE_SHA}。
任务说明：{CARD_BRANCH}@{CARD_SHA} 的 {CARD_PATH}。
只向 {ASSIGNED_BRANCH} 推送；只写 {ALLOWED_WRITE_PATHS}；结果文件 {RESULT_PATH}；到 {STOP_CONDITION} 即结束。不得推送/合并 main，不得发布、打标签、接触密钥或签名。

开工前先读 docs/governance/gpt-construction-window-rules.md 的第 2、3、4、5、5a、6、8 节。第 8a、8b、9 节只适用于长期施工窗口，本单次子任务不读、不执行。
一次完成，不在本对话追问。缺信息或确认持续权限/平台阻断时，把阻塞点写进结果文件并提交后结束；普通实现、工具、CI 或远端写入失败要在当前任务内继续定位和推进。按第 2 节尽早把可恢复进度推到指定 GitHub 分支，使回合截断后 Codex 能接手。

远端写入失败不等于未写入，也不等于没权限：先重读远端 branch、commit、文件及实际状态，确认是否已落地；已落地就继续，不重复制造对象。未落地才按递增间隔重试，并区分 ChatGPT 动作权限、GitHub App 安装范围、所选仓库、仓库权限、ref 接口和分支状态。一次 403 不足以断定无写权限；422 “already exists” 可能表示上次已成功。远端状态不明时，不盲目重复有副作用的写入。

按任务卡完成工作，并在 {RESULT_PATH} 写明审查的精确提交、证据、测试/CI 结果与未验证点。只提交指定分支和路径，完成后结束。
```

## 7. Task-description template

Commit a self-contained JSON or Markdown card with the following fields and instructions. A task-specific card may add details but may not omit these. Keep the **Controller's page procedure out of the GPT task**; this template describes only what GPT receives.

```text
task_id: {TASK_ID}
role: {ROLE}
repository: {REPOSITORY}
card_branch: {CARD_BRANCH}
card_path: {CARD_PATH}
card_sha: {CARD_SHA, supplied in starter after commit}
base_sha: {BASE_SHA}
review_sha: {EXACT_REVIEW_SHA, if audit}
assigned_branch: {ASSIGNED_BRANCH}
allowed_read_paths: {READ_PATHS}
allowed_write_paths: {WRITE_PATHS}
result_path: {RESULT_PATH}
success_criteria: {EXACT_GITHUB_ARTIFACT_AND_REAL_CI_WITH_TESTS_GT_ZERO}
deadline: {DEADLINE}
stop_condition: {ONE_RESULT_COMMIT_OR_PERSISTENT_BLOCKER}
original_codex_seat: astra
economic_reason: {ONE_LINE_COST_JUDGMENT}

Before work, read docs/governance/gpt-construction-window-rules.md §§2,3,4,5,5a,6,8; §§8a,8b,9 are long-window-only and do not apply. Use small reads, early durable progress, ordinary-failure continuation, and real executed-test evidence. One message and no chat questions. If information is missing or a permission/platform block is confirmed persistent, write the blocker in result_path, commit if possible, and end. If even result_path cannot be written, stop with exact remote state; the Controller's page patrol/deadline handles the missing artifact.
For a GitHub write error, first reread remote branch/commit/file. If already landed, continue without duplicate writes. If not, retry with increasing intervals and identify the failing layer: ChatGPT action permission, GitHub App installation scope, selected repository, repository permission, ref API, or branch state. A lone 403 does not prove no permission; 422 already-exists may prove a prior success; never repeat a side-effecting write while remote state is unknown.
Push recoverable progress early to assigned_branch, only within allowed_write_paths. Never push/merge main, publish, release, tag, access secrets, or sign. No self-acceptance; Controller checks exact SHA, diff, CI logs, and main.
```

The pre-Send model gate and stop-on-visible-downgrade guidance adapt Ivan Kwiatkowski's [MIT-licensed consultation skill](references/ivkiwi-MIT.txt), whose attribution and license are retained. Send-once, turn confirmation, stall, and low-frequency observation were informed by [ask-web-ai](https://github.com/pgw10086/ask-web-ai); no unlicensed code was copied.
