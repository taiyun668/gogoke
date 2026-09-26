---
name: web-chatgpt-subagent
description: Route a large, independent gogoke task to ChatGPT Chat through the Codex in-app browser, with GitHub-only handoff and local tier accounting. Use only after Owner enables the route; keep quick, local, or urgent work in Codex.
---

# Web ChatGPT subagent

This route is **disabled for routine work until Owner explicitly enables it**. Owner-authorized trials may use GPT-5.6 tiers only. GPT-6 Pro is separately disabled until Owner explicitly enables that tier. The requirements are [the direction document](../../../docs/directions/2026-09-26-web-chatgpt-subagent-channel.md); this skill does not amend project routing or acceptance rules.

## Decide and prepare

Use this route only when all five conditions in direction §3 hold. Record a short estimate of Controller, Codex subagent, and web costs and why this route wins. Do not route local work, urgent repairs, or work that needs inherited context. Set a task-specific follow-up limit and a deadline before dispatch.

Commit the self-contained task card to GitHub **before opening a new Chat conversation**. Include task ID, role, goal, repository, assigned branch, base SHA, exact read/write paths, success criteria, result path and the `docs/model-routing.md` result fields, applicable CI and task-class gates, return/stop conditions, and no main merge/publish/secret/signing authority. Use a fresh conversation outside Projects. The starter message contains only those routing fields; the task card is authoritative. Never send a prompt that relies on local files or private context. A web audit's write to its result file is only a reporting exception; it may not mutate the reviewed candidate.

Give every dispatch a distinct task ID. The ID identifies one logical prompt and one send attempt; never reuse it to send the same prompt again. Before staging, inspect the saved conversation for an existing turn with that ID. A lost tab or a journal error is a recovery problem, not a reason to resend.

Branch and path restrictions are instructions, not a proven connector permission sandbox. Verify the resulting GitHub commit's parent, branch, and changed paths. If a connector write returns an ambiguous transient error, re-read the assigned GitHub ref before considering a retry; honor any stronger task-specific stop rule. Existing high-risk audit slots, formal gates, and external final review remain in force. A matching model family does not prove equivalent role instructions, reasoning effort, tools, or audit acceptance.

## Browser and account boundary

Use only the Codex in-app browser. No tunnel, external/system browser, desktop Computer Use, private ChatGPT endpoint, cookie extraction, or `codex-with-chatgpt`. Observe the page directly. If sign-in, verification, permission, or account identity is in question, obey the foreground-browser invariant in the task's AGENTS.md. Stop at that boundary and report only the Owner login action when login is lost. Do not infer foreground visibility from a controlled tab.

Immediately before each send, inspect the final composer on the same page: saved, non-Project conversation; intended task ID and GitHub routing fields; selected model family **and** thinking tier. Close menus and verify the resolved composer label, not a model merely listed in a menu or an ambiguous `Latest`. If any field differs, do not send. Never select GPT-6 Pro unless Owner has enabled it and the local ledger says it is enabled. If the page shows a limit or downgrade before sending, stop. After each send, observe the actual reply model. If it switched or a limit appears, mark the requested tier exhausted and record the page's displayed reset time if available; stop that tier. Do not silently accept a fallback model as the requested work product.

After clicking Send once, **never click Send again for that task ID**, even if the button, tab, or tool reports an error. A new user turn followed by a new assistant turn in the saved conversation is the confirmation that the prompt was submitted. A cleared composer, Stop button, click return, or URL alone is insufficient. If submission is uncertain, preserve the conversation and inspect it read-only; mark `uncertain_submission` and return to Codex if it cannot be resolved. If the answer shows no new evidence for five minutes, mark `stalled` without resending. After about 20 minutes, observe at low frequency; a local wait timeout does not terminate the browser task. Report login loss as `needs_login`, separate from a timeout.

## Local ledger

The script at `scripts/ledger.py` stores state only at `%LOCALAPPDATA%\gogoke\web-chatgpt-subagent\state.json`. All Chat seats using the account must feed this same ledger; if their usage since the last reconciliation is unknown, treat the available allowance as unknown and do not dispatch. Never commit state, account usage, reset timestamps, or conversation links to this public repository.

Use `python scripts/ledger.py status` to inspect caps and usage. Immediately before each send, `reserve --tier TIER --task TASK_ID --seat SEAT`; it atomically charges one message and returns a reservation ID. If the message was not sent, `release ID`. After a sent reply, `complete ID --actual-tier TIER [--url URL] [--seconds N]`; on a switch, add `--switched [--reset-at ISO_TIME]`. Record other Chat seat sends through the same reserve/complete flow. Daily defaults: 6 Pro 25, 5.6 Sol Pro 120, Extra High 12, High 20, Medium 30; the two Pro tiers together have a 200/day ceiling. The script does not guess the vendor's reset time or unpublished limits. `reset-tier` requires an observed reset timestamp and a reason. `enable-6-pro` requires Owner's explicit decision recorded in a local note. The route itself remains disabled until Owner adopts this skill.

## Return and acceptance

Retrieve the result from GitHub only. An audit result names the reviewed commit SHA; construction uses its assigned branch and a checkpoint. Inspect the precise result, branch/commit identity, and real Actions run with nonzero tests. Read logs when the run is claimed as evidence. A web reply saying “done” does not count. Record local duration, tier/message count, Controller token cost when available, rework, and any downgrade/return.

On timeout before Send, browser failure, exhausted tier, or model mismatch, stop this dispatch. Change tier only if the exact task card preauthorizes it and its task-class gates still hold; otherwise return to a Codex subagent. Do not repeat the same failed question beyond its follow-up limit. For a pre-send fallback, append a local event to `%LOCALAPPDATA%\gogoke\web-chatgpt-subagent\trial-events.jsonl` with task ID, timestamp, `web_not_sent`, exact reason, and the Codex subagent handoff identity. Release any unsent reservation. Keep the public trial summary free of account usage figures and conversation URLs. Owner alone decides formal enablement, `docs/model-routing.md` changes, and PR merge.

The pre-send composer gate and visible downgrade stop are adapted from Ivan Kwiatkowski's [MIT-licensed consultation skill](references/ivkiwi-MIT.txt). The send-once, turn-confirmation, stall, and low-frequency observation ideas were informed by [ask-web-ai](https://github.com/pgw10086/ask-web-ai); no code or text from that unlicensed repository is copied.
