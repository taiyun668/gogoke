# Native slow-test measurement

## Evidence and limits

Cloud run `37588203943`, job `112683631242` (`Native host Windows real process axes`), tested source `38b6b7d6` and succeeded: 737 passed, 0 failed. The test build took 3m29s; libtest reported 3448.53s total. Its log contains 35 `has been running for over 60 seconds` notices.

The log records each notice and each `... ok` completion, but no test-start, mutex-acquire, or mutex-release event. Thus it does not provide per-case elapsed time or queue duration. The notice-to-completion interval is not a test duration. Historical source shows every listed case reaches the same `cfg(test)` `route_b_test_guard` mutex, directly or through its fixture helper, so mutex wait is included in the harness time; the fraction attributable to queue versus fixture/body cannot be recovered from this log.

The case notes below distinguish source-proven work from unknown elapsed attribution. “Guard queue” applies to every row. A successful case may have completed a real process, pipe, RPC, or filesystem operation, but the warning alone does not show how long it waited for that operation.

## All 35 slow notices

| Historical test | Source-visible work beyond the shared guard |
|---|---|
| `store::product_database::v37_holder_disappearance_tests::actual_partial_holder_recovery_cold_reuses_original_capture_at_each_durable_boundary` | Actual child-process exit and Windows parent-exit wait; recovery checked at durable cut points. |
| `store::product_database::v37_holder_disappearance_tests::actual_two_disappeared_holders_recover_in_one_call_replay_without_acl_effect_and_admit_cold_source` | Two actual holder processes disappear after Job close; exact identity/exit evidence and recovery/replay assertions. No STOPPED fact is created. |
| `store::product_database::v37_holder_disappearance_tests::composed_disappearance_rejects_actual_live_identity_and_wrong_physical_source_without_resource_effects` | Actual process identity/creation-time checks and physical-source rejection; process fixture also performs real stop/exit work. |
| `store::product_database::v37_inbox::tests::user_inbox_read_replay_collision_stale_and_cancel_cas_use_original_store` | Native store/SQLite dispatcher, replay, collision, stale, and CAS state; no explicit timed wait. |
| `store::product_database::v37_ledger_user::tests::native_user_ledger_scopes_replay_and_subscription_survive_reopen` | Native database and subscription/reopen state; no explicit timed wait. |
| `store::product_database::v37_login::login_cache::tests::generated_cache_junction_unlinks_only_entry_and_preserves_target` | Real Windows junction creation/removal (`cmd.exe`/Win32 handles) and filesystem assertions; no timed wait. |
| `store::product_database::v37_login::tests::cancelled_pinned_cli_preserves_stderr_before_confirmed_release` | Pinned CLI child, original stderr, process wait (bounded at 15s), and confirmed stop/release. |
| `store::product_database::v37_login::tests::owner_first_cli_factory_failures_keep_original_results_and_custody` | CLI factory/custody failure branches and original-result checks; some branches fail before child launch, so elapsed cannot be assigned wholly to process wait. |
| `store::product_database::v37_login::tests::owner_instance_list_reads_only_registered_native_state_and_rejects_other_frames` | Registered native state and frame validation; no child-process wait. |
| `store::product_database::v37_login::tests::owner_login_preflight_errors_settle_original_request_without_process_custody` | Dispatch/SQL preflight failures and settled receipts; explicitly no process custody. |
| `store::product_database::v37_login::tests::owner_login_retains_second_cli_custody_until_its_own_stop_is_confirmed` | Two pinned CLI/process-custody stages and stop confirmation for the corresponding child. |
| `store::product_database::v37_login::tests::pinned_cli_ordinary_oauth_callback_reaches_exact_owned_child` | Actual CLI loopback callback/listener ownership checks; 30s polling deadline with 50ms polls. No credentials or real authorization are supplied. |
| `store::product_database::v37_login::tests::pinned_codex_empty_home_reports_native_logout_and_durable_stop` | Pinned CLI account observation, real process exit (bounded at 15s), and durable stop. |
| `store::product_database::v37_login::tests::pinned_codex_isolated_credential_file_lifecycle_uses_cli_without_host_reads` | Pinned CLI credential-file operation, real process wait (bounded at 15s), and filesystem metadata checks. |
| `store::product_database::v37_login::tests::pinned_codex_owner_login_stop_failure_reconciles_same_proof_and_stderr` | Pinned CLI parser child and stderr, process wait (bounded at 15s), then injected durable-stop write failure and reconciliation. |
| `store::product_database::v37_runtime::tests::actual_pinned_codex_product_open_records_rpc_and_durable_stop_without_model_call` | Real pinned CLI credential setup, Codex app-server pipe/RPC startup, session work, and durable process stop. |
| `store::product_database::v37_runtime::tests::actual_pinned_codex_two_scope_file_history_and_stopped_revocation_without_model` | Real CLI/app-server RPC and process-stop evidence; two-scope credential/file history, database reopen, and resume. |
| `store::product_database::v37_runtime::tests::health_compact_late_original_ack_continues_without_second_write_or_new_request` | One real health fixture setup/session-open/stop; later A-source/ACK controls are synthetic store/SQL inputs, not a real provider reply or model call. |
| `store::product_database::v37_runtime::tests::health_terminal_and_ordinary_ack_orders_keep_one_original_seal_and_no_work_resend` | Two complete health fixture setups; actual CLI/session startup and stop, with synthetic ACK/source ordering assertions. |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_later_work_append_and_unacknowledged_write_suppress_original_cause` | Four complete health fixture executions; real setup/session-open/stop each time, synthetic health/RPC association controls. |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_new_generation_suppresses_original_physical_cause` | One complete real health fixture setup/session-open/stop; health-generation evidence is state-driven. |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_original_malformed_typed_response_cannot_become_unsupported` | Two complete real health fixture executions; malformed response bytes are synthetic SQL/source controls. |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_original_stop_intent_and_route_changes_suppress_without_new_identity` | Two complete real health fixture executions; stop-intent/route state is controlled in the store, not awaited on a clock. |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_original_unsupported_receipt_prefix_and_readonly_owner_notice` | Two complete real health fixture executions; receipt/notice source controls are synthetic. |
| `store::product_database::v37_runtime::tests::stalled_health::stalled_health_unknown_plain_remote_code_and_unassociated_unsupported_have_no_authority` | Two complete real health fixture executions; unknown/unsupported association cases are synthetic store/source state. |
| `store::product_database::v37_seat::tests::owner_configuration_and_user_seat_share_the_verified_product_store` | Native database/configuration and seat assertions; no explicit timed wait. |
| `store::product_database::v37_session::tests::product_admission_enforces_persisted_caps_and_rolls_back_busy_on_denial` | Native dispatch/admission and persisted state; no explicit process wait. |
| `store::product_database::v37_session::tests::product_merge_history_rechecks_current_grant_without_git_and_preserves_unknown_cause` | Native store/history and grant recheck; test explicitly avoids Git. |
| `store::product_database::v37_session::tests::product_reopens_exact_previous_worktree_schema_preserving_unpinned_sources` | Temporary SQLite/filesystem close and reopen plus schema assertions; no explicit timed wait. |
| `store::product_database::v37_session::tests::product_worktree_source_reopens_and_original_requests_never_reissue_unknown` | Product source/request persistence and reopen assertions; no explicit timed wait is present in the test body. |
| `store::worktree::f2::tests::host_seal_receipt_failure_retains_child_and_freezes_old_and_new_requests` | Real Git/worktree child operations and receipt-failure state checks. |
| `store::worktree::f2::tests::host_seal_refuses_attribute_and_hardlink_before_host_reads_content` | Real filesystem attributes/hardlink/reparse fixtures and Git/worktree checks. |
| `store::worktree::f2::tests::host_seal_requires_persisted_original_intent_before_child_effect` | Real Git index/worktree state plus injected persistence failure before child effect. |
| `store::worktree::f2::tests::host_seal_uses_actual_stopped_rebound_instance_not_creation_or_merger` | Real child Git operation tied to actual stopped/rebound process identity. |
| `store::worktree::f2::tests::merge_preintent_denial_retains_guard_and_writes_no_intent` | Real worktree fixture and pre-intent denial; no merge commit is expected. |

The 8 health test functions call `health_control_product` 16 times in total. That helper holds the shared guard through private database/root creation, fixed-CLI staging/probe/migration, credential setup, Git fixture creation, real Codex session open, synthetic health assertions, durable stop, checked database close, and fixture cleanup. This is repeated real setup/teardown cost, not clock-based “stalled” waiting. Successful output does not identify the time spent in each phase.

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

At the time of this checkpoint, the runtime job `112952142589` was still `IN_PROGRESS`; its logs and timing results were unavailable. Record no before timing numbers from that run until it completes. An after-optimization run is `NOT_RUN`.

## Reference points

- Shared lock: `apps/desktop/native-host/src/store/same_open.rs` (`route_b_test_guard`).
- Runtime setup/cleanup: `apps/desktop/native-host/src/store/product_database/v37_runtime_tests.rs` (`health_control_product`, `qualify_synthetic_file_backend`).
- Health control cases: `apps/desktop/native-host/src/store/product_database/v37_stalled_health_tests.rs`.
- Fixed CLI fixture: `apps/desktop/native-host/src/store/product_database/managed_cli_test_setup.rs`.
- 30s response loops: `apps/desktop/native-host/src/store/product_database/v37_runtime.rs`.

This evidence note grants no new test or product authority. No local native test was run; after-optimization results remain NOT_RUN.
