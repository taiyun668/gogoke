TASK RESULT: IMPLEMENTED_UNVERIFIED
CURRENT_STATE: B2.1 adapter source is present on the isolated branch; shared host wiring is pending.
FILES_CHANGED: third_party/t3code/apps/server/src/gogoke/adapters/claude/{adapter.ts,index.ts,protocol.ts,session.ts,REFERENCES.md,SHARED_INTEGRATION.md}; artifacts/gogoke-37/parallel/B2/TASK-RESULT.md
IMPORTANT_DIFF: Added the pinned Claude Code 2.1.286 stream-json boundary, session-ID resume arguments, stream event/result decoding, LPAC admission refusal, isolated instance environment with documented auto-memory-off flag, and explicit login/status command descriptors. In-turn stdin append reports UNKNOWN. Native QCard and manual compaction are not claimed.
VALIDATION: NOT_RUN (real CLI protocol golden samples and end-to-end host wiring unavailable); typecheck/cloud CI pending.
FAILURES: None observed.
INVARIANTS_CHECKED: No credential reads/copies, no automatic login, no CLI/system setting mutation, no non-LPAC model launch fallback; only allowed adapter/artifact paths edited.
RISKS: No real CLI was run; installed binary pin, effective memory-off, LPAC, login, session resume/interjection behavior, question cards and compaction are unverified.
DEVIATIONS_FROM_PLAN: Native question card and manual compaction are reported unsupported on the documented CLI path; automatic compaction is documented but runtime-unverified.
OPEN_QUESTIONS: None for the adapter code; shared wiring and runtime evidence belong to Controller/H and Owner-held login/installed-product stages.
RECOMMENDED_NEXT_ACTION: Controller reviews SHARED_INTEGRATION.md, wires only the required shared seams, then runs exact-branch cloud checks and schedules one real pinned-CLI/LPAC end-to-end evidence pass.
