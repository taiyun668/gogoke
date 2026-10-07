# Native slow-test measurement

## Evidence and limits

Cloud run `37588203943`, job `112683631242` (`Native host Windows real process axes`), tested source `38b6b7d6` and succeeded: 737 passed, 0 failed. The test build took 3m29s; libtest reported 3448.53s total. Its log contains 35 `has been running for over 60 seconds` notices.

The log records each notice and each `... ok` completion, but no test-start, mutex-acquire, or mutex-release event. Thus it does not provide per-case elapsed time or queue duration. The notice-to-completion interval is not a test duration. Historical source shows every listed case reaches the same `cfg(test)` `route_b_test_guard` mutex, directly or through its fixture helper, so mutex wait is included in the harness time; the fraction attributable to queue versus fixture/body cannot be recovered from this log.

The case notes below distinguish source-proven work from unknown elapsed attribution. “Guard queue” applies to every row. A successful case may have completed a real process, pipe, RPC, or filesystem operation, but the warning alone does not show how long it waited for that operation.

## All 35 slow notices

The timing columns come from the later timing-only `81189de` run `37668010053`, not reconstructed historical durations. All 35 exact case names were found in its direct guard records. For cases with repeated helpers the queue and held-body values sum their non-overlapping guard intervals; columns are not comparable across shards as shared queue durations. Remainder failed one separate Job fixture, so this is measurement evidence, not a green whole-run claim.

| Historical test | Source-visible work beyond the shared guard | Measured guard queue | Measured guard-held body |
|---|---|---:|---:|
| `store::product_database::v37_holder_disappearance_tests::actual_partial_holder_recovery_cold_reuses_original_capture_at_each_durable_boundary` | Actual child-process exit and Windows parent-exit wait; recovery checked at durable cut points. | 132.321s | 1466.827s (5 guards) |
| `store::product_database::v37_holder_disappearance_tests::actual_two_disappeared_holders_recover_in_one_call_replay_without_acl_effect_and_admit_cold_source` | Two actual holder processes disappear after Job close; exact identity/exit evidence and recovery/replay assertions. No STOPPED fact is created. | 1754.266s | 359.488s |
| `store::product_database::v37_holder_disappearance_tests::composed_disappearance_rejects_actual_live_identity_and_wrong_physical_source_without_resource_effects` | Actual process identity/creation-time checks and physical-source rejection; process fixture also performs real stop/exit work. | 1472.618s | 215.714s |
| `store::product_database::v37_inbox::tests::user_inbox_read_replay_collision_stale_and_cancel_cas_use_original_store` | Native store/SQLite dispatcher, replay, collision, stale, and CAS state; no explicit timed wait. | 1682.513s | 0.592s |
| `store::product_database::v37_ledger_user::tests::native_user_ledger_scopes_replay_and_subscription_survive_reopen` | Native database and subscription/reopen state; no explicit timed wait. | 575.594s | 0.583s |
| `store::product_database::v37_login::login_cache::tests::generated_cache_junction_unlinks_only_entry_and_preserves_target` | Real Windows junction creation/removal (`cmd.exe`/Win32 handles) and filesystem assertions; no timed wait. | 0.000s | 0.080s |
| `store::product_database::v37_login::tests::cancelled_pinned_cli_preserves_stderr_before_confirmed_release` | Pinned CLI child, original stderr, process wait (bounded at 15s), and confirmed stop/release. | 0.079s | 91.625s |
| `store::product_database::v37_login::tests::owner_first_cli_factory_failures_keep_original_results_and_custody` | CLI factory/custody failure branches and original-result checks; some branches fail before child launch, so elapsed cannot be assigned wholly to process wait. | 142.305s | 124.872s |
| `store::product_database::v37_login::tests::owner_instance_list_reads_only_registered_native_state_and_rejects_other_frames` | Real managed-CLI ready/probe fixture, then registered native state/frame validation; no additional login/account process wait in those assertions. | 267.163s | 132.616s |
| `store::product_database::v37_login::tests::owner_login_preflight_errors_settle_original_request_without_process_custody` | Real managed-CLI fixture, then dispatch/SQL preflight failures and settled receipts; no login process custody is created by the refused requests. | 331.338s | 63.168s |
| `store::product_database::v37_login::tests::owner_login_retains_second_cli_custody_until_its_own_stop_is_confirmed` | Two pinned CLI/process-custody stages and stop confirmation for the corresponding child. | 320.657s | 93.223s |
| `store::product_database::v37_login::tests::pinned_cli_ordinary_oauth_callback_reaches_exact_owned_child` | Actual CLI loopback callback/listener ownership checks; 30s polling deadline with 50ms polls. No credentials or real authorization are supplied. | 223.451s | 101.900s |
| `store::product_database::v37_login::tests::pinned_codex_empty_home_reports_native_logout_and_durable_stop` | Pinned CLI account observation, real process exit (bounded at 15s), and durable stop. | 262.182s | 87.716s |
| `store::product_database::v37_login::tests::pinned_codex_isolated_credential_file_lifecycle_uses_cli_without_host_reads` | Pinned CLI credential-file operation, real process wait (bounded at 15s), and filesystem metadata checks. | 256.675s | 96.986s |
| `store::product_database::v37_login::tests::pinned_codex_owner_login_stop_failure_reconciles_same_proof_and_stderr` | Pinned CLI parser child and stderr, process wait (bounded at 15s), then injected durable-stop write failure and reconciliation. | 286.601s | 70.193s |
| `store::product_database::v37_runtime::tests::actual_pinned_codex_product_open_records_rpc_and_durable_stop_without_model_call` | Real pinned CLI credential setup, Codex app-server pipe/RPC startup, session work, and durable process stop. | 673.539s | 804.846s |
| `store::product_database::v37_runtime::tests::actual_pinned_codex_two_scope_file_history_and_stopped_revocation_without_model` | Real CLI/app-server RPC and process-stop evidence; two-scope credential/file history, database reopen, and resume. | 0.000s | 423.450s |
| `store::product_database::v37_runtime::tests::health_compact_late_original_ack_continues_without_second_write_or_new_request` | One real health fixture setup/session-open/stop; later A-source/ACK controls are synthetic store/SQL inputs, not a real provider reply or model call. | 423.450s | 250.089s |
| `store::product_database::v37_runtime::tests::health_terminal_and_ordinary_ack_orders_keep_one_original_seal_and_no_work_resend` | Two complete health fixture setups; actual CLI/session startup and stop, with synthetic ACK/source ordering assertions. | 1478.383s | 302.063s (2 guards) |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_later_work_append_and_unacknowledged_write_suppress_original_cause` | Four complete health fixture executions; real setup/session-open/stop each time, synthetic health/RPC association controls. | 2196.518s | 601.992s (4 guards) |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_new_generation_suppresses_original_physical_cause` | One complete real health fixture setup/session-open/stop; health-generation evidence is state-driven. | 1106.909s | 233.102s |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_original_malformed_typed_response_cannot_become_unsupported` | Two complete real health fixture executions; malformed response bytes are synthetic SQL/source controls. | 535.164s | 300.877s (2 guards) |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_original_stop_intent_and_route_changes_suppress_without_new_identity` | Two complete real health fixture executions; stop-intent/route state is controlled in the store, not awaited on a clock. | 533.978s | 305.541s (2 guards) |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_original_unsupported_receipt_prefix_and_readonly_owner_notice` | Two complete real health fixture executions; receipt/notice source controls are synthetic. | 1358.383s | 302.007s (2 guards) |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_unknown_plain_remote_code_and_unassociated_unsupported_have_no_authority` | Two complete real health fixture executions; unknown/unsupported association cases are synthetic store/source state. | 1209.541s | 299.791s (2 guards) |
| `store::product_database::v37_seat::tests::owner_configuration_and_user_seat_share_the_verified_product_store` | Native database/configuration and seat assertions; no explicit timed wait. | 360.583s | 0.375s |
| `store::product_database::v37_session::tests::product_admission_enforces_persisted_caps_and_rolls_back_busy_on_denial` | Native dispatch/admission and persisted state; no explicit process wait. | 0.000s | 123.892s |
| `store::product_database::v37_session::tests::product_merge_history_rechecks_current_grant_without_git_and_preserves_unknown_cause` | Native store/history and grant recheck; test explicitly avoids Git. | 123.892s | 0.419s |
| `store::product_database::v37_session::tests::product_reopens_exact_previous_worktree_schema_preserving_unpinned_sources` | Temporary SQLite/filesystem close and reopen plus schema assertions; no explicit timed wait. | 124.310s | 0.433s |
| `store::product_database::v37_session::tests::product_worktree_source_reopens_and_original_requests_never_reissue_unknown` | Product source/request persistence and reopen assertions; no explicit timed wait is present in the test body. | 124.743s | 2.835s |
| `store::worktree::f2::tests::host_seal_receipt_failure_retains_child_and_freezes_old_and_new_requests` | Real Git/worktree child operations and receipt-failure state checks. | 49.931s | 17.183s |
| `store::worktree::f2::tests::host_seal_refuses_attribute_and_hardlink_before_host_reads_content` | Real filesystem attributes/hardlink/reparse fixtures and Git/worktree checks. | 50.212s | 13.706s |
| `store::worktree::f2::tests::host_seal_requires_persisted_original_intent_before_child_effect` | Real Git index/worktree state plus injected persistence failure before child effect. | 48.468s | 14.375s |
| `store::worktree::f2::tests::host_seal_uses_actual_stopped_rebound_instance_not_creation_or_merger` | Real child Git operation tied to actual stopped/rebound process identity. | 45.264s | 16.734s |
| `store::worktree::f2::tests::merge_preintent_denial_retains_guard_and_writes_no_intent` | Real worktree fixture and pre-intent denial; no merge commit is expected. | 44.816s | 14.104s |

The 8 health test functions call `health_control_product` 16 times in total. That helper holds the shared guard through private database/root creation, fixed-CLI staging/probe/migration, credential setup, Git fixture creation, real Codex session open, synthetic health assertions, durable stop, checked database close, and fixture cleanup. This is repeated real setup/teardown cost, not clock-based “stalled” waiting. Historical output does not identify the time spent in each phase; the timing-only rerun below measures it directly.

## Session-shard direct timing

The test-only timing run `37668010053`, source `81189de`, completed session job `112952143012`: 4 passed, 0 failed, 745 filtered; libtest finished in 127.58s. The guard instrumentation recorded queue and guard-held body times directly:

| Session case | Guard queue | Guard-held body |
|---|---:|---:|
| `product_admission_enforces_persisted_caps_and_rolls_back_busy_on_denial` | 19µs | 123.891681s |
| `product_merge_history_rechecks_current_grant_without_git_and_preserves_unknown_cause` | 123.891742s | 0.418706s |
| `product_reopens_exact_previous_worktree_schema_preserving_unpinned_sources` | 124.310485s | 0.432939s |
| `product_worktree_source_reopens_and_original_requests_never_reissue_unknown` | 124.743434s | 2.835189s |

This directly explains the four session-shard over-60s notices: the admission case spent about 123.9s inside the guarded body; the other three spent about 124s waiting for the mutex and under 2.9s in the guarded body. These measurements apply only to this session shard and run; they do not estimate runtime-shard queue or body time.

## Runtime shard follow-up evidence

The runtime shard has 11 passing cases in both runs below. Ten cases take the shared guard; `rpc_read_failure_retains_only_bounded_incomplete_stdout_in_private_error` is a pure bounded-string diagnostic test without the guard.

| Cloud run / source | Runtime job result | Libtest result |
|---|---|---|
| `37640163780` / `8cc7c5d6` | Job `112856679517` succeeded; 75m47s job wall time. | 11 passed; 4007.97s test time. |
| `37628898381` / `129e94e7` | Job `112817728614` succeeded; 75m07s job wall time. | 11 passed; 3974.94s test time. |

The 8 health test functions account for 16 `health_control_product` executions: terminal/ordinary order (2), compact ACK (1), later-work controls (4), new generation (1), malformed reply controls (2), stop/route controls (2), unsupported-prefix controls (2), and unknown/unassociated controls (2). Each execution repeats the real setup and stop path described above.

`v37_runtime.rs` contains three separate 30s monotonic response-deadline loops: Claude initialize, ACP metadata RPC, and Codex `native_rpc_observation`. These runtime-shard cases use the Codex driver, so only the Codex loop can be on their exercised path; the Claude and ACP loops are not triggered by this shard. The logs do not show any loop consuming its full 30s allowance. The health/stalled-health assertions use synthetic store/source controls and do not sleep to create “stalled” time.

Clock injection is suitable only for tests of deadline arithmetic/expiry semantics. It must not replace successful real pipe/RPC replies, process exit, durable STOPPED, CLI, Git, or callback evidence. No test result here justifies reducing those real waits.

## Test-only timing run and comparison state

Run `37668010053`, source `81189de`, is the timing-only measurement run. Its source diff changes only `same_open.rs` and `v37_runtime_tests.rs` under test configuration: `route_b_test_guard` records `queue_us` and guard-held `body_us`; `health_control_product` records root-open, CLI-ready, credential, seat, Git fixture, session-open, health body, stop, and cleanup phase durations. No production optimization was included.

Runtime job `112952142589` completed: 11 passed, 0 failed, 738 filtered; 3823.76s. This is a measurement baseline, not an optimization result. Login passed 32 cases in 1485.41s; session passed 4 in 127.58s. Remainder passed 701 and failed 1 in 2659.22s, so the complete run is not green. Its real Job stop fixture used a 20ms close deadline and recorded `CLOSE_BINDING_DEADLINE_EXCEEDED`; the fixture now uses the existing production stop protocol, preserving all kernel/identity/forced-stop assertions. That correction is awaiting cloud validation.

| Runtime health helper phase | Calls | Total seconds | Mean seconds |
|---|---:|---:|---:|
| Actual session open | 16 | 1040.933 | 65.058 |
| Actual CLI credential setup | 16 | 697.510 | 43.594 |
| Managed CLI ready/stage/probe | 16 | 600.810 | 37.551 |
| Health assertion body | 16 | 177.844 | 11.115 |
| Checked close/cleanup | 16 | 36.959 | 2.310 |
| Git fixture | 16 | 32.136 | 2.009 |
| Root open | 16 | 7.355 | 0.460 |
| Actual stop | 16 | 1.594 | 0.100 |
| Seat setup | 16 | 0.236 | 0.015 |

These repeated helper stages consume about 2595s while holding the guard. The other two real runtime cases held it for 423.450s and 804.846s. A guard wait can explain individual over-60s notices, but does not eliminate this actual serialized work. Stop is not the measured bottleneck. No evidence yet attributes the startup costs to expiry of a 30s response deadline. The existing test package already uses opt-level=3; adding that setting is not a new optimization.

| Same 11-case runtime shard | Test time | Interpretation |
|---|---:|---|
| Historical source 8cc7c5d6 | 4007.97s | Successful historical run, without phase timing |
| Historical source 129e94e7 | 3974.94s | Successful run; relevant source blobs equal |
| Timing-only source 81189de | 3823.76s | Baseline with direct queue/body/stage timing; no optimization |
| After optimization source 73cabd2d | 963.01s | Same 11 cases pass; about 74.8% less libtest time; see direct producer and four-shard results below |

## Direct producer timing and bounded optimization

Run `37679041089`, source `57b1e37a`, job `112990395538`, executed exactly the original compact health case: 1 passed, 0 failed, 251.60s. This diagnostic run did not run the full library or other formal gates.

| Producer | Calls | Time | Direct attribution |
|---|---:|---:|---|
| Existing store SHA | 166 | 194.065641s | SHA computation on 39,101,108,600 bytes |
| Windows file SHA | 48 | 18.885717s | Combined open, read and SHA computation; not pure CPU time |
| Managed/program file reads | 144 | 11.732697s | File-read time, separate from store SHA above |
| Login + native RPC responses | 13 | 2.203842s total; 0.586535s maximum | Response-loop elapsed time; not additive with work nested inside a response loop |

The dominant measured cost is repeated hashing, not response-deadline expiry. Direct `rustc --test` output from the saved listing confirms native-host `-C opt-level=3` and debug assertions enabled; the generic “unoptimized” profile banner is not proof that the package override was ignored.

The bounded candidate reuses `sha2 = "=0.10.9"`, already locked and used by the LPAC shim build for vendor/module byte verification. It changes only the three existing production hash producers in F's managed CLI/registry and the shared Windows process SHA wrapper. All actual per-call reads, metadata/reparse/identity checks, pins, error returns and protocol/STOPPED inputs remain. No cache, deadline change or clock substitute is used. The general store digest is left unchanged because it is outside this plan's listed write scope. An early diagnostic prototype that changed it was withdrawn before Root integration.

The SHA package's test-only opt3 setting retains debug assertions and overflow checks; this additional compilation factor is explicit. Release profile and CPU flags are unchanged. Independent static risk review of exact `73cabd2d` found no new static blocker across scope, dependency/lock/license, streaming, error/memory, actual callers, measurement, gates and side effects. Dynamic verification is pending in four-shard run `37681827946`; the runtime test and guard source remains the same 11 cases as `81189de`. The original streaming wrapper also loses buffered bytes when consecutive short updates do not fill a block. New independent standard/partial-chunk vectors check the actual wrapper; correctness is standard SHA-256, not preservation of that old wrong result.

The unchanged four-way partition run `37681827946` at exact `73cabd2d` completed successfully, including the partition verifier. Runtime has the same original 11 cases and all pass without skips. The additional SHA safety case belongs to remainder, not runtime.

| Shard | Timing-only before (`81189de`) | After (`73cabd2d`) | Count/result |
|---|---:|---:|---|
| Runtime | 3823.76s | 963.01s | Same 11 cases: all pass, no ignored; about 74.8% less time |
| Login | 1485.41s | 436.51s | Same 32 cases: all pass, no ignored |
| Session | 127.58s | 56.08s | Same 4 cases: all pass, no ignored |
| Remainder | 2659.22s | 1156.98s | Before 701 pass/1 fail; after 703 pass/0 fail, including one added SHA safety case. This is not an exactly equal case-count comparison. |

The runtime job is `113000212947`; login `113000213044`, session `113000212733`, remainder `113000213109`. Both before/after numbers are libtest execution duration, excluding compilation and queue. The after run also passed 2 compatibility shim cases. A directed compiled mutation step in run `37682377202` succeeded: unchanged original and restored vectors passed, disabling the actual production streaming update compiled and failed the unchanged vectors. The rest of that directed workflow remains in progress; it is not complete formal gate evidence.

Root integrated only the five permitted production/dependency/safety-test files, without diagnostic timers or workflow changes. The actual integration SHA still needs its own affected cloud checks; benchmark success is not Root/candidate/Win11 acceptance.

## Reference points

- Shared lock: `apps/desktop/native-host/src/store/same_open.rs` (`route_b_test_guard`).
- Runtime setup/cleanup: `apps/desktop/native-host/src/store/product_database/v37_runtime_tests.rs` (`health_control_product`, `qualify_synthetic_file_backend`).
- Health control cases: `apps/desktop/native-host/src/store/product_database/v37_stalled_health_tests.rs`.
- Pinned SHA reference: `apps/desktop/native-host/src/process/session/compat/build.rs` and its `Cargo.toml`; official RustCrypto one-shot/incremental API. Historical gogo-party/NaveHQ/LoomOS and repo research were checked first; no closer runtime implementation was found than this existing in-repo build dependency.
- Fixed CLI fixture: `apps/desktop/native-host/src/store/product_database/managed_cli_test_setup.rs`.
- 30s response loops: `apps/desktop/native-host/src/store/product_database/v37_runtime.rs`.

This evidence note grants no new test or product authority. No local native test was run; exact-source cloud benchmark results do not replace actual integration gates or Owner Win11 end-to-end evidence.
