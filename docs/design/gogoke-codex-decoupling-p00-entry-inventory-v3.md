# gogoke Codex 去专属化 P00 V3：生产入口、Writer 与数据 Root 全量台账

> `artifact_id: GKD-P00-ENTRY-INVENTORY-V3`
> `plan_version: 3.0`
> `baseline_commit: bedc8ff7cb18f7ff2f188123950f25e27621b3c9`
> `evidence_date: 2026-09-18`
> `state: source_inventory_pending_independent_review`
> `runtime_claim: NONE`
> `source_closure: INCOMPLETE`

本文是 `gogoke-codex-decoupling-p00-v3.md` 的 companion。它记录已追踪生产入口、启动副作用、writer 和 root，供 P01、退出清单与后续测试固定范围。**当前 source closure 尚未完成，§9 的 readable-source frontier 未逐成员闭合；不能再以计数相等、关键词命中或语义组表宣称全量盘点完成。** `active/source-registered` 只表示当前 baseline 有可达源码/注册面，不表示本批次实际运行；`preserved-disabled` 仅用于已经被源码证明封口的入口。本基线 remote/Tailscale 与 dictation 仍有注册/启动路径。

## 1. 枚举方法、统计与遗漏检测

### 1.1 机械枚举

1. 从 `apps/desktop/src-tauri/src/lib.rs` 的 `tauri::generate_handler!` 提取全部命令；得到 **129** 个唯一命令（含无 module 前缀的 `is_mobile_runtime`）。
2. 对 `apps/desktop/src/**/*.{ts,tsx}` 做跨文件正则扫描，提取字面量 `invoke("...")`；得到 **129** 个唯一前端调用名。与 registry 比较：**registry_without_frontend=0，frontend_without_registry=0**。
3. 先从 `rpc/dispatcher.rs` 解析实际可达 handler 顺序：`daemon`、`workspace`、`codex`、`git`、`prompts`；只扫描这五个 `try_handle` 文件。对四个普通 handler 及 `git.rs` 分别解析字面量 `"method" =>` arms，得到 **77** 个；再解析 `git.rs` 的 **27** 个 `git_rpc::METHOD_* =>` arms，并到 `shared/git_rpc.rs` 将每个常量解析成字符串值。全部常量必须恰好解析一次，未解析或重复值即 inventory 失败。两集合 union 得到 **104** 个唯一 daemon methods，其中 **101** 个与 Tauri registry 重合，daemon-only 为 `ping`、`daemon_info`、`daemon_shutdown`，Tauri-only 为 **28** 个。
4. 固定 tracked source/platform 并集后，追 `main()`/`run()` 前置调用、platform `cfg`、`.setup()`、`on_window_event`、`RunEvent`、settings side effects、React `useEffect`/bootstrap/update hooks；同时展开 `mod`/`#[path]`/`include_str!`/TS import/HTML script/package scripts/workflow 引用。只扫 command 或关键词不构成 closure。
5. 搜索 Rust `write/fs::write/File::create/OpenOptions/rename/write_settings/write_workspaces`、路径解析与 frontend localStorage/sessionStorage/indexedDB；非测试前端得到 **43** 个 localStorage 调用（17 get、24 set、2 remove）、**0** 个 sessionStorage、**0** 个 indexedDB，归并为 **22** 个 key/family。
6. 对每个语义组人工追到 UI/hook → `services/tauri.ts` → Tauri/RPC → shared/native/disk，并核 writer、现状、V3 owner、T/D 映射。对没有 UI 的 startup/daemon-only 入口单独列项。
7. Web Storage 不是 frontend writer 全集：另扫生产 `navigator.clipboard.writeText`、别名 `clipboard.writeText`、全部 `@tauri-apps/plugin-*` imports/call sites，以及 `@tauri-apps/api/{menu,window,webview}` 的直接 mutation。直接 `navigator.clipboard.writeText` 为 **11**，另有 **1** 个经局部变量调用的 clipboard sink；plugin-opener 为 **9** 个 `openUrl` + **12** 个 `revealItemInDir`，plugin-notification 为 **1** 个实际 native sink，plugin-dialog 人工确认 **7** 个 `open`、**1** 个 `save`、**7** 个 `ask`、**3** 个 `message`；Tauri API 另有 **10** 个 native menu `popup`、**4** 个 window lifecycle/drag mutations、**4** 个 `setEffects`、**1** 个 webview `setZoom`。
8. 对生产 Rust（排除 `#[cfg(test)]`/tests）扫描 `Command::new`、`temp_dir`、create/write/rename/remove，再人工追调用可达性；每条已知 process chain 必须枚举链上全部 `Command::new`，不能只记录产生目标文件的进程。两条 transient conversion chain 共有 **3** 个 process call sites：HEIF attachment→1×`/usr/bin/sips`；open-app icon→1×`defaults read`（代码点可能调用两次 key）+1×`sips`。
9. 对生产 frontend 扫描 `AudioContext`/`webkitAudioContext` construction、`resume`、media `fetch`/decode、`destination` connect/start，并追调用者；当前 notification sound helper 有 **1** 个 AudioContext construction site、1 个 resume、1 个 fetch/decode/output chain、2 类 caller（agent turn notification 与 updater test sound）。

### 1.2 遗漏检测规则

- 后续 registry、前端 invoke、daemon method 任一集合计数或成员变化，必须更新本文；不能只改代表性链。
- 新增 `localStorage`/`sessionStorage`/`indexedDB`、文件 write/rename/remove、外部 CLI spawn、网络更新、login 或 daemon lifecycle 路径时，必须新增 root/writer 行并绑定 C/T/D。
- **反例固定：**只用 `^\s*"..."\s*=>` 字符串正则会漏掉 `git.rs` 的 27 个 `git_rpc::METHOD_*` 常量 arms，错误得到 77/74/55；因此 daemon census 必须同时解析 dispatcher reachability、literal arms、constant arms 和 constant definitions。动态命令名、宏生成、plugin 内部持久化仍不能由此完全证明；当前已知的 window-state plugin 单列 `UNKNOWN` root。fresh review 还需检查构建后实际注册面。
- tests/fixtures 被排除于生产入口计数；它们只能作为测试资产，不能提升 runtime 状态。
- frontend direct sink 计数不能用单一字符串替代：`navigator.clipboard.writeText` 的 11 个精确调用与 `const clipboard=navigator.clipboard; clipboard.writeText` 的 1 个别名调用必须分列；泛搜 `.writeText` 会混入非 clipboard 对象，泛搜 `open()` 会混入 xterm 等非 plugin 调用。
- `Command::new("sips")` 单扫会漏 icon chain 前置的 `Command::new("defaults")`；只扫 file create/write 会漏由子进程创建的 temp output。Audio side effect 也不能由 Web Storage/plugin imports 推断，必须独立扫 AudioContext/media graph。
- 本轮未运行 app/daemon、未读生产数据、未触发 CLI/login/update/Tailscale/dictation/provider；所有状态均为源码级。

## 2. 完整命令面与生产入口分类

表中 `T`=Tauri command，`D`=daemon RPC。E01–E15 的 command 组合计恰为 129；E16–E20 是不增加 registry 数量的 frontend direct OS/plugin/audio 与 Rust transient process/file surfaces。组内个别不同 surface/副作用在下方子表拆明。

| ID / 命令（数量） | UI/hook → service → IPC/RPC → shared/native-or-disk | 类别 | 当前状态 | V3 owner；测试/退出映射 |
|---|---|---|---|---|
| E01 settings（3）：`get_app_settings`, `update_app_settings`, `get_codex_config_path` | Settings/useAppSettings → `services/tauri.ts` → T+D settings → `settings_core`; update 写 `settings.json`，同时写 native `CODEX_HOME/config.toml` feature/personality，并可能 start/restart remote daemon | mixed；update=writer/process control | `active/source-registered`; runtime unknown | C01,C20,C21,C24,C27,C34,C37,C39,C41；WP09b/WP02/WP10a；T23,T37,T39,T55,T56,T58；D05,D08,D09,D11 |
| E02 gogoke update（4）：`gogoke_update_check`, `gogoke_update_install`, `gogoke_update_signal_ready`, `gogoke_update_take_failure` | `useUpdater`/MainApp → service → T → `gogoke_update.rs` → GitHub/releases, app cache update state/installer/coordinator/temp readiness/install dir | network+writer+process control | `active/source-registered`; auto-check source-present | C01,C39；WP12；T42,T46,T59,T60；无删除授权 |
| E03 file/config export（4）：`file_read`, `file_write`, `read_image_as_data_url`, `write_text_file` | settings/resources/export hooks → service → T；前两项另有 D → `files_core`/policy → workspace `AGENTS.md` 或 global `CODEX_HOME/{AGENTS.md,config.toml}`；export 写用户选择路径 | mixed；2 read/2 writer | `active/source-registered` | C07,C10,C23,C24,C29,C32,C38；WP07/WP08；T22,T23,T25,T26,T37,T49,T54；D05,D06,D11 |
| E04 workspace/worktree（20）：`list_workspaces`, `is_workspace_path_dir`, `add_workspace`, `add_workspace_from_git_url`, `add_clone`, `add_worktree`, `worktree_setup_status`, `worktree_setup_mark_ran`, `remove_workspace`, `remove_worktree`, `rename_worktree`, `rename_worktree_upstream`, `apply_worktree_changes`, `update_workspace_settings`, `set_workspace_runtime_codex_args`, `connect_workspace`, `list_workspace_files`, `read_workspace_file`, `open_workspace_in`, `get_open_app_icon` | workspace hooks/dialogs/actions → service → T+D workspace RPC → `workspaces_core`; writes app-data registry/markers, Git/worktrees/project files, spawns app-server/external opener | mixed writer/process/external | `active/source-registered` | C04,C05,C06,C14,C28,C29,C32,C33,C34,C37,C38；WP02/WP03/WP08/WP10b；T03,T10,T22,T34,T37,T41,T43,T50,T51,T55,T63；D04,D09,D11 |
| E05 thread/delivery/control（18）：`start_thread`, `send_user_message`, `turn_steer`, `turn_interrupt`, `start_review`, `respond_to_server_request`, `remember_approval_rule`, `resume_thread`, `read_thread`, `thread_live_subscribe`, `thread_live_unsubscribe`, `fork_thread`, `list_threads`, `list_mcp_server_status`, `archive_thread`, `compact_thread`, `set_thread_name`, `collaboration_mode_list` | thread/composer/review/approval hooks → service → T+D codex RPC → `codex_core` → app-server stdin/native session；approval-rule 可写 native rules/config | mixed native writer/control/read | `active/source-registered` | C06,C07,C09,C11,C12,C13,C14,C15,C16,C17,C18,C19,C26,C33,C34,C38,C42；WP02/WP03/WP04/WP06；T04,T05,T09,T10,T13-T20,T28,T29,T31,T40,T41,T44,T49,T51,T53,T64,T65,T67；D02,D03,D04,D09 |
| E06 三类 generator（3）：`generate_commit_message`, `generate_run_metadata`, `generate_agent_description` | Git/worktree/agent UI hooks → service → T+D（commit-message 由 `git_rpc::METHOD_GENERATE_COMMIT_MESSAGE` 常量 arm 可达）→ `codex_core`/native app-server → generated text/metadata returned | native model execution；无直接 durable writer in command | `active/source-registered` | C05,C23,C30,C31；WP08；T30,T62；D07 |
| E07 Codex capability/account/resources（12）：`get_config_model`, `codex_doctor`, `codex_update`, `model_list`, `experimental_feature_list`, `set_codex_feature_flag`, `account_rate_limits`, `account_read`, `codex_login`, `codex_login_cancel`, `skills_list`, `apps_list` | settings/model/account/resource hooks → service → T；除 `codex_update` 外均有 D → shared/native Codex CLI/app-server/config; login/update 可诱发 native credential/install writes | mixed read/native execution/writer | `active/source-registered`; actual login/capabilities unknown | C20,C21,C22,C25,C26,C33,C37,C39；WP09a/WP09b/WP07/WP12；T06,T23,T27,T31,T36,T40,T50,T51,T58,T59,T62；D08,D10 |
| E08 agent config（7）：`get_agents_settings`, `set_agents_core_settings`, `create_agent`, `update_agent`, `delete_agent`, `read_agent_config_toml`, `write_agent_config_toml` | SettingsAgents/useAgent hooks → service → T+D → `agents_config_core`/`config_toml_core` → `CODEX_HOME/config.toml` + managed `agents/*.toml` | mixed；6 writer-capable | `active/source-registered` | C24,C27,C38,C42；WP07；T24,T26,T27,T37,T49,T60,T67；D05,D06,D11 |
| E09 prompts（7）：`prompts_list`, `prompts_create`, `prompts_update`, `prompts_delete`, `prompts_move`, `prompts_workspace_dir`, `prompts_global_dir` | `useCustomPrompts`/prompt UI → service → T+D → `prompts_core` → app-data `workspaces/<id>/prompts` and native `CODEX_HOME/prompts` | mixed；CRUD writers | `active/source-registered` | C23,C37,C38；WP07/WP02；T03,T23,T25,T37,T49,T55；D05,D11 |
| E10 Git/GitHub（26）：`get_git_status`, `init_git_repo`, `create_github_repo`, `list_git_roots`, `get_git_diffs`, `get_git_log`, `get_git_commit_diff`, `get_git_remote`, `stage_git_file`, `stage_git_all`, `unstage_git_file`, `revert_git_file`, `revert_git_all`, `commit_git`, `push_git`, `pull_git`, `fetch_git`, `sync_git`, `get_github_issues`, `get_github_pull_requests`, `get_github_pull_request_diff`, `get_github_pull_request_comments`, `checkout_github_pull_request`, `list_git_branches`, `checkout_git_branch`, `create_git_branch` | Git hooks/panels → service → T+D via `git::try_handle` and 26 `git_rpc::METHOD_*` constants → `git_ui_core`/git/GitHub → workspace repo and remote | mixed; repo writer/network | `active/source-registered`; shared T+D | C04,C05,C10,C29,C30,C32,C38；WP08/WP06；T03,T22,T23,T28,T30,T34,T49；retained, no D07 unless AI generator |
| E11 terminal（4）：`terminal_open`, `terminal_write`, `terminal_resize`, `terminal_close` | terminal hooks → service → T → terminal session/process stdin/PTY cwd | process writer/control | `active/source-registered`; Tauri-only | C05,C14,C28,C33,C38；WP08/WP10b；T03,T21,T33,T44,T50,T52,T54；D09 for duplicate spawn paths |
| E12 dictation（8）：`dictation_model_status`, `dictation_download_model`, `dictation_cancel_download`, `dictation_remove_model`, `dictation_start`, `dictation_request_permission`, `dictation_stop`, `dictation_cancel` | dictation controller/input UI → service → T → `dictation/real.rs` → microphone permission/session + app-data model cache/network | mixed network/writer/permission/process | **`active/source-registered`; V3 target `preserved-disabled`, not yet proven sealed** | C09,C36,C38；WP10a；T37,T42,T57；不得删除模型/cache |
| E13 local usage（1）：`local_usage_snapshot` | usage hooks → service → T+D → `local_usage_core` → read native `CODEX_HOME/sessions/**/*.jsonl` | read-only scanner | `active/source-registered`; authority unsuitable for public usage without adapter | C07,C22,C37；WP09b；T23,T36,T40,T64；D10 |
| E14 Tailscale/remote daemon（5）：`tailscale_status`, `tailscale_daemon_command_preview`, `tailscale_daemon_start`, `tailscale_daemon_stop`, `tailscale_daemon_status` | SettingsServer → service → T → tailscale/daemon commands → managed `gogoke_daemon`, TCP, settings/data root | process/network/control | **`active/source-registered`; V3 target `preserved_disabled`, not yet proven sealed** | C14,C33,C34,C38,C41；WP10a/WP10b；T41,T43,T45,T56,T63；D09 |
| E15 shell/menu/tray/notification/runtime（7）：`menu_set_accelerators`, `set_tray_recent_threads`, `set_tray_session_usage`, `is_macos_debug_build`, `app_build_type`, `send_notification_fallback`, `is_mobile_runtime` | MainApp/menu/tray/notification hooks → service → T；`menu_set_accelerators`,`is_macos_debug_build`,`send_notification_fallback` 亦有 D → OS/window/tray | runtime/OS side effects, mostly non-durable | `active/source-registered` | C01,C02,C03,C06,C08,C14,C35,C39；WP05/WP12；T08,T09,T11,T12,T23,T35,T46,T66；D01 |
| E16 frontend clipboard direct writers（12 sinks） | UI/menu hooks → 11×`navigator.clipboard.writeText` + 1×aliased `clipboard.writeText` → OS clipboard；不经 `services/tauri.ts`/IPC | direct OS writer；可能写 transcript/debug/message/path/Git/script | `active/source-present`; clipboard不是应用 authority | C03,C07,C09,C11,C23,C29,C32,C35,C38；WP05/WP06/WP08/WP09b；T11,T12,T22,T23,T28,T34,T49,T54,T66；D03；其余 retained |
| E17 frontend direct plugin/Tauri OS surfaces | UI/hooks/services → plugin-dialog/opener/notification + Tauri menu/window/webview → OS picker/confirmation/browser/file-manager/notification/menu/window/compositor | direct OS/plugin side effects；dialog选择本身不写文件，save path后续由E03写 | `active/source-present` | C01,C02,C10,C15,C20,C29,C32,C35,C38,C39；WP05/WP08/WP09b/WP12；T11,T12,T19,T22,T23,T28,T35,T46,T49,T54；D01,D08 where login URL；其余 retained |
| E18 macOS HEIF attachment conversion | `apps/desktop/src-tauri/src/shared/codex_core.rs:57-105,402-429`：composer/send → `send_user_message_core` → `build_turn_input_items` → `read_image_as_data_url_core` → `%TEMP%/gogoke-image-*.jpg` → `/usr/bin/sips` → read/base64 → best-effort remove；remote/mobile `files/mod.rs:87-114` 亦可达 | transient sensitive-file writer + subprocess | `active/source-present` on macOS；runtime unverified | C10,C11,C33,C38；WP03/WP08；T21,T35,T49,T50,T52,T54；D02,D12 |
| E19 macOS open-app icon discovery/conversion | `apps/desktop/src-tauri/src/workspaces/macos.rs:65-87,149-195`，经 `workspaces/commands.rs:655-659` 与 `bin/gogoke_daemon.rs:999-1003` 可达：OpenApp/settings → T+D handler → `defaults read` 获取 `CFBundleIconFile`/`CFBundleIconName` → `%TEMP%/gogoke-icon-*.png` → `sips` → read/base64 → best-effort remove | external metadata-read process + transient app-icon writer process | `active/source-present` on macOS；runtime unverified | C01,C32,C33,C38；WP03/WP05/WP08；T12,T22,T50,T52,T54；D12 |
| E20 frontend notification audio output | `apps/desktop/src/utils/notificationSounds.ts:25-80` → lazy AudioContext create/resume → fetch bundled audio URL → decode → buffer source/gain → `ctx.destination` → start；callers `features/notifications/hooks/useAgentSoundNotifications.ts:35-38` 与 `features/app/hooks/useUpdaterController.ts:94-100` | direct browser/OS audio output + media fetch/decode；不经 Tauri IPC | `active/source-present`; permission/autoplay/device/runtime unverified | C01,C03,C35,C38；WP05；L@T11,T12,T23,T54,T66，I@WP11b；D01 only if shell binding replaced, otherwise retained |
| E21 embedded Windows update coordinator | E02 install → `gogoke_update.rs:29,577-647` `include_str!(../update/gogoke-update-coordinator.ps1)` → staged PowerShell script → dynamic installer/newexe/uninstaller/oldexe + reg.exe + file/registry rollback/cleanup（§9.2） | executable script/process/file/registry writer boundary；非普通cache写入 | `active/source-present`; 不授权运行/发布 | C14,C33,C38,C39,C40；WP12；T38,T42,T46,T50,T52,T59,T60；D12 only after replacement verified |
| E22 notification permission/fallback branches | `services/tauri.ts:1149-1221` → macOS debug query；非debug→plugin import→isPermissionGranted→requestPermission→granted send；denied与exception均尝试fallback；app `notifications.rs:28-56`与daemon `gogoke_daemon.rs:1378-1405`有两个macOS debug osascript实现 | OS permission query/request + notification output + external subprocess；拒绝不等于成功 | `active/source-present`; fallback success未报告且非macOS-debug必Err | C15,C35,C38；WP05/WP10b；T11,T19,T23,T44,T50,T52,T54；D09,D12 |
| E23 daemonctl four CLI entries | `bin/gogoke_daemonctl.rs:74-147,149-232` start/stop/status/command-preview → settings/root+token → TCP probe/auth/info/shutdown/bind → Unix listener tools/signals或non-Unix拒绝 → spawn（§9.3） | separate CLI execution/control/network/OS signal authority；不是Tauri command | `active/source-present`; 全平台源码分支见§9.3，runtime未验 | C14,C33,C34,C37,C38,C41；WP02/WP03/WP10b；T32,T39,T41,T43,T44,T50,T52,T55,T56,T63；D09,D12 |
| E24 daemon wire authentication/event subscription | `bin/gogoke_daemon/transport.rs:9-104`，进入business dispatcher之前；有token时非auth unauthorized、错token invalid-token、匹配才subscribe；no-token模式立即subscribe | permission gate + TCP writer + broadcast subscription lifecycle；business集合104之外auth加1，wire union105 | `active/source-present`; 未运行token/拒绝/重放/断线测试 | C11,C19,C33,C34,C38,C41；WP04/WP10a/WP10b；T16,T19,T24,T41,T43,T44,T54,T56,T63；D09 |

### 2.1 Clipboard call-site inventory

| # | 精确调用位置 | 写入内容类别 | 失败语义／边界 |
|---:|---|---|---|
| 1 | `features/app/components/MainHeader.tsx:315` | workspace/repo `cd` command | 当前 await 未本地 catch；路径暴露到 OS clipboard，非应用 authority |
| 2 | `features/debug/hooks/useDebugLog.ts:79` | 全量 debug entries/payload | 可能含诊断/私有内容；调用方负责错误，不能视为脱敏导出 |
| 3 | `features/messages/components/Markdown.tsx:373` | code block raw/fenced text | try/catch 控 UI copied 状态；clipboard 内容不形成 Delivery |
| 4 | `features/messages/components/useMessagesViewState.ts:149` | single message text | catch 后不置 copied；可能含私聊，受 C38/T49 |
| 5 | `features/messages/hooks/useFileLinkOpener.ts:232` | resolved file URL/path+line/column | failure non-fatal；路径可能越隐私域，须按原 link authority |
| 6 | `features/workspaces/components/ClonePrompt.tsx:159` | suggested copies folder path | permission failure ignored；只复制建议路径，不授权 clone/write |
| 7 | `features/app/hooks/useSidebarMenus.ts:58` | native/current thread ID | failure non-fatal；裸 ID 不得被当公共 Session authority |
| 8 | `features/git/components/GitDiffPanel.tsx:302` | Git commit SHA | 当前 menu action 未本地 catch；固定对象文字不授权 Git 操作 |
| 9 | `features/git/components/GitDiffPanel.tsx:497` | Git file basename | 当前未本地 catch；文件名不等于内容/完整路径 |
| 10 | `features/git/components/GitDiffPanel.tsx:503` | project-relative Git path | 当前未本地 catch；不得提升为外部文件授权 |
| 11 | `features/threads/hooks/useCopyThread.ts:20` | rendered thread transcript | catch 写 debug；高隐私导出，必须受 Session/domain 与 T11/T49 |
| 12 | `features/settings/components/sections/SettingsEnvironmentsSection.tsx:184` | environment draft script；经局部 `clipboard` alias | 明示 unavailable/error toast；复制脚本不等于执行或批准 |

精确 `navigator.clipboard.writeText` 为 #1–#11 共 11 个；#12 解释为何 alias-aware sink 扫描总数为 12。两种计数必须同时保留。

### 2.2 Other direct plugin surface inventory

- plugin-opener：9 个 `openUrl`（About GitHub、Markdown链接2、update链接2、login auth URL、Git commit/PR/issue）和 12 个 `revealItemInDir`（Files/message link/open-app/prompt/worktree/clone/config/agent 等路径）。这些动作把 URL/path 交给外部 OS app；失败有的 await、有的 fire-and-forget，不能证明用户看见或目标安全。
- plugin-notification：`services/tauri.ts:1213` 是 1 个实际 native sink；agent completion、response-required、test/update controller 经 wrapper 调用。通知正文进入 OS notification center，须按 C35/T11/T23 防私聊泄露；send failure 由各 caller 捕获/记录。
- plugin-dialog：人工限定 imported plugin symbols 后为 7×`open`、1×`save`、7×`ask`、3×`message`。open/save 只选择路径，真实文件写由 E03；ask/message 是 UI 确认/告知，不是 Owner 权限、HumanActionRequest 或 durable receipt。
- direct Tauri API：10×native menu `popup`（Git 3、Files 1、Sidebar 4、Prompts 1、file-link 1）；`startDragging` 1、window `minimize/toggleMaximize/close` 各 1；Liquid Glass `setEffects` 4；webview `setZoom` 1。它们修改 OS/window/UI 状态但不是数据 authority；fire-and-forget 失败不能算操作成功，菜单确认也不授予超出对应 handler 的权限。

### 2.3 Browser audio side-effect inventory

- `notificationSounds.ts:35` 懒创建进程内共享 AudioContext；`45-62` 在 suspended 时 fire-and-forget `resume()`，异步 fetch/decode，连接低增益到 OS audio destination 并 `start()`。初始化异常与异步 load/play 异常只进入 debug logger；无 runtime 证据证明权限、autoplay、设备路由或实际可听。
- agent caller 只在启用、窗口不聚焦、持续时间满足等条件下由 app-server activity 触发；声音本身不含正文，但时间/成功失败类型可泄露活动事实。test caller 由设置动作触发。不得把声音播放当通知送达、已读、任务完成或用户批准。

### 2.4 daemon-only RPC

| Entry | 链/副作用 | 状态 | Owner / tests |
|---|---|---|---|
| `ping` | TCP client → daemon dispatcher → constant health response | read-only | WP10b；T43,T44 |
| `daemon_info` | TCP client → daemon state → version/mode/process identity info | read-only | WP03/WP10b；T31,T43,T50,T51,T58 |
| `daemon_shutdown` | TCP client → daemon dispatcher → shutdown signal/process exit | process control | WP03/WP10b；T17,T32,T41,T43,T52,T63；D09 |

## 3. 非 command 的启动、退出与自动副作用

| ID | 触发链 | Writer/control | 当前状态 | Owner；测试/退出映射 |
|---|---|---|---|---|
| S01 AppState load | Tauri `.setup()` → `AppState::load` → `app_data_dir`（失败回 cwd）→ read `workspaces.json/settings.json` → manage state | read；错误时改变 root 选择 | `active/source-present`; runtime path unknown | C01,C04,C06,C33,C37；WP02；T02,T37,T39,T55；D04,D05,D11 |
| S02 startup daemon auto start/restart | Tauri `.setup()` async task → settings `remote_backend_provider=Tcp`; remote mode直接 `tailscale_daemon_start`; local mode若 daemon running 也再次 start 以执行 version-current/restart logic | process/network/control | **active source path；与 V3 preserved-disabled 目标未闭合** | C33,C34,C39,C41；WP10a/WP10b；T41,T43,T56,T63；D09 |
| S03 settings-triggered daemon start | `update_app_settings` → `ensure_remote_runtime_for_settings` → remote mode `tailscale_daemon_start`；同时清 remote backend connection | writer/process/network | **active source path** | C33,C34,C37,C41；WP10a/WP10b；T43,T56,T63；D09 |
| S04 exit daemon stop/keep | `RunEvent::ExitRequested` → `keep_daemon_running_after_app_close`; false 时 `stop_managed_daemons_for_exit` → daemon stop | process control | active source path | C14,C33,C34；WP10b；T17,T32,T44,T52；D09 |
| S05 update readiness startup signal | MainApp mount → `signalGogokeUpdateReady` → `gogoke_update_signal_ready` → temp readiness receipt/state | writer | active source path | C39；WP12；T42,T46,T59 |
| S06 updater auto check/failure/post-update | `useUpdater(autoCheckOnMount=true)` → take failure → update check；pending localStorage version triggers post-update release fetch | network/localStorage/update-state mutation | active source path outside dev | C01,C39；WP12；T23,T42,T46,T59 |
| S07 app bootstrap | MainApp → `useAppBootstrap` → settings, dictation controller, debug, transparency/material hooks | mixed; may reach dictation/settings surfaces through later effects | active source path | C01,C09,C36；WP05/WP10a；T01,T02,T12,T54,T57；D01 |
| S08 single-instance/window-state plugin | Tauri builder plugins → single-instance focus + `tauri_plugin_window_state` | OS/window; plugin-managed persistence root not explicit in repo | active; durable root `UNKNOWN` | C01,C02,C39；WP05/WP12；T12,T37,T46；D01 |
| S09 Linux startup process environment mutation | `lib.rs:72-92`：NV仅在 `__NV_PRIME_RENDER_OFFLOAD` 缺失时置1，**不要求NVIDIA/Wayland**；DMABUF仅在Wayland+`/proc/driver/nvidia/version`存在+变量缺失时置1；COMPOSITING仅在非Wayland且DISPLAY存在（X11）+变量缺失时置1。后续spawn可继承进程env | process-global env writer；三个 guard 各自不同，不允许用同一泛化 guard 替代 | `active/source-present`; 每条spawn final env未捕获 | C01,C33,C38；WP03 owner，WP05视觉协同；T50.L@WP03、T12/T50.I@WP11b；若抽取重复env helper则D12 |
| S10 macOS close/reopen lifecycle | `lib.rs:112-121,334-339`：main window CloseRequested → prevent_close+hide；RunEvent::Reopen → show+focus | non-command OS window lifecycle mutation | `active/source-present`; runtime/focus/long-session semantics unverified | C01,C02,C14,C35；WP05/WP10b；T09,T12,T44；D01,D09 only when replacing shell/old lifecycle paths |
| S11 platform shell setup | `lib.rs:126-137`：macOS setup initializes native tray；Windows setup sets main-window decorations false and hides native menu while retaining accelerators | non-command OS tray/window/menu mutation | `active/source-present`; startup errors/visual identity未实机验证 | C01,C02,C35；WP05/WP12；T09,T12,T46；D01 |
| S12 main PATH prelude | `src-tauri/src/main.rs:5-8` → `fix_path_env::fix()`，Err仅stderr后仍run；external implementation固定Cargo.lock source `fix-path-env-rs#c4c45d503ea115a839aae718d02f79e7c7f0f673` | process env/可能shell执行的外部依赖边界；未读取依赖缓存源码，不凭名称推实现 | external boundary fixed；WP03需取该revision源码/打包身份与final env证据 | C01,C33,C38,C40；WP03/WP12；T50,T59,T60；D12 if consolidated |
| S13 iOS native setup | `lib.rs:166-170` → `window.rs:80-125` with_webview/native pointers → scrollView两个inset mutations、viewController两个extended-layout mutations；null pointer各自跳过；with_webview错误返回 | native UI mutation 4处；无磁盘writer；非command入口 | source-present iOS only；本次Windows主线不背书iOS | C01,C02,C09,C34；WP05/WP10b；T12,T45,T54；D01 |
| S14 browser mobile bootstrap | `index.html:16` → `src/main.tsx` → mobile-only gesture preventDefault与viewport/focus listeners →CSS `--app-height`/dataset mutation→React root；非mobile纯guard-return | DOM/event subscription/lifecycle；无OS权限授予 | source-present；移动平台未runtime验 | C01,C02,C09,C34；WP05/WP10b；T12,T45,T54；D01 |

## 4. 数据 Root、Reader 与 Writer 台账

| Root ID / 解析 | Readers | Writers/删除者 | 当前 authority 与迁移处理 | Owner；测试 |
|---|---|---|---|---|
| R01 desktop app data：Tauri `app.path().app_data_dir()`；当前失败回 cwd | `AppState::load`, workspace/settings commands | `storage::write_workspaces`, `write_settings`; CRUD/settings | 当前 registry/settings authority，但 fallback 不合格；P02 保留格式、固定 rootId/schema/OS lock，禁止回 cwd/坏数据清空 | C04,C06,C33,C37；WP02；T37,T39,T55；D04,D05,D11 |
| R02 `R01/workspaces.json` | desktop/managed daemon workspace list | workspace/worktree CRUD, ordering/settings, rename/remove | 现有 workspace registry；迁移需保留 ID/path/settings/source，不与 Session 混同 | C04,C05,C06,C37；WP02/WP08；T03,T34,T37,T39,T55；D04,D11 |
| R03 `R01/settings.json` | app/daemon/settings/startup | settings update/path normalization | 当前含 backend/token/update/UI settings；秘密引用和信任策略分层迁移，不能由旧数据改 trust root | C01,C20,C21,C34,C37,C39,C41；WP02/WP09b/WP12；T23,T37,T39,T46,T59；D08,D09,D11 |
| R04 `R01/workspaces/<workspaceId>/prompts/*.md` | prompt list/read | prompt create/update/delete/move | 当前 gogoke workspace prompt root；可作为迁移输入，不等于 P01 最终 public resources schema | C23,C24,C37,C38；WP07/WP02；T25,T37,T55；D05,D11 |
| R05 `R01/worktree-setup/<workspaceId>.ran` 与默认 `R01/worktrees/<parentId>` | setup status/worktree resolver | mark-ran, worktree create/rename/remove | operational markers/worktrees；保留来源并与真实 Git worktree identity核对 | C04,C05,C29,C39；WP08/WP12；T34,T37,T46；D11 |
| R06 dictation model root：`app_data_dir`（失败回 cwd）`/models/whisper/{model,.partial}` | model status/dictation runtime | download/create/rename/remove | 语音资产目标 `preserved_disabled`；保留 path/version/hash/cache，不再允许 fallback cwd | C36,C39；WP10a/WP12；T37,T42,T57；D11 |
| R07 app cache：`app.path().app_cache_dir()/update-cache` | updater check/install/failure recovery | installer, `update-state.json/.part`, coordinator, failure logs, lock | update runtime state；迁移可保留状态引用但重新验证 source/hash/trust，不导入覆盖 trust | C39,C40；WP12；T42,T46,T59,T60；D11 |
| R08 OS temp：`%TEMP%/gogoke-update-<token>.ready` 及 partial | update coordinator/app readiness | coordinator/app create/rename/remove | 短期 receipt，必须与 build/version/owner 绑定；不是 durable product data | C39,C40；WP12；T46,T59 |
| R09 installed app dir：current exe parent 或 `%LOCALAPPDATA%/Programs/gogoke` | updater/uninstaller/readiness | update coordinator replaces install, rollback artifacts | 发布 authority；仅正式签名/hash链可写，旧 DB 不能改信任根 | C39,C40；WP12；T46,T59,T60 |
| R10 daemon/daemonctl data roots：managed path显式传入；standalone daemon 优先 `XDG_DATA_HOME/gogoke_daemon`，否则 `HOME/.local/share/gogoke_daemon`，**HOME 缺失时从 `.` 构造相对 `.local/share/gogoke_daemon`**；Windows `gogoke_daemonctl` 优先 `APPDATA/app.gogoke.desktop`，否则 `USERPROFILE/AppData/Roaming/app.gogoke.desktop`，**APPDATA 与 USERPROFILE 均缺失时从 `.` 构造相对 root** | daemon/daemonctl state/RPC/settings | daemon workspace/settings/prompt writers | 两个 cwd-relative fallback 均违反 V3“禁止 fallback cwd”；当前仅发现未修复。WP02 固定 root/lock，WP10b 固定 managed launch/self-report，完成前阻塞 P02/P10b acceptance | C33,C34,C37,C38,C41；WP02/WP10b；T39,T41,T43,T55,T63；D09,D11 |
| R11 workspace/project root：`WorkspaceEntry.path` | files/Git/skills/app-server/terminal | Git/worktree/apply, terminal/scripts, workspace `AGENTS.md`, native tools | 用户内容 authority；只按固定授权写，迁移不复制/删除 root；路径 alias/junction需核 | C04,C05,C10,C24,C28,C29,C32,C33,C38；WP08/WP03；T03,T21,T22,T24,T34,T49,T50,T54,T55；D05,D09 |
| R12 global Codex Home：`CODEX_HOME` 或 user home `/.codex` | config/account/model/skills/apps/prompts/local usage/app-server | `config.toml`, `AGENTS.md`, `agents/*.toml`, `prompts/*.md`, `rules/default.rules`；Codex CLI login/update/runtime native writes | native source，不是 public gogoke root；公共资源预览/显式导入，原件保留；不复制 token | C15,C20,C21,C23,C24,C25,C26,C27,C37,C38；WP07/WP09a/WP09b；T19,T23,T25-T27,T37,T40,T49,T50,T58；D05,D06,D08 |
| R13 `CODEX_HOME/sessions/**/*.jsonl` | `local_usage_core`, native history/list | native Codex CLI（gogoke scanner只读） | native history/usage evidence；不得当公共 usage/content唯一 authority；导入需来源与隐私域 | C06,C07,C16,C17,C22,C37,C38；WP02/WP09b；T11,T23,T36,T37,T40,T49,T64；D10 |
| R14 user-selected export/save path | export/dialog flows | `write_text_file` | 用户显式目标；不得被默认 root 或迁移扫描吞入 | C07,C10,C32,C38；WP08；T22,T23,T49 |
| R15 localStorage 22 key families | frontend hooks listed in §6 | frontend setters/removers | best-effort UI metadata，不是 server authority；按 Session/root 映射迁移，坏值不覆盖 authoritative data | C02,C03,C06,C08,C09,C16,C18,C19,C35,C37,C39；WP02/WP05/WP12；T07-T12,T37,T39,T42,T66 |
| R16 window-state plugin managed root | `tauri_plugin_window_state` | plugin | exact path/format未在 repo 调用面解析，`UNKNOWN`; WP00不得虚构，P12前需实测/源码固定 | C01,C02,C39；WP05/WP12；T12,T37,T46 |
| R17 native install/credential locations induced by `codex_update`/`codex_login` | native CLI | native CLI | 外部原生 authority；产品只保留引用/状态，不直接迁移 credential、不自主 update/login | C20,C21,C33,C38,C39；WP09a/WP09b/WP12；T23,T40,T50,T58,T59；D08 |
| R18 GitHub/remote Git/external opener/Tailscale system state | GitHub/Git/tailscale/open-app adapters | create repo/push/pull/fetch/sync, daemon process; opener may mutate externally | 外部 authority，不属于本地迁移 DB；动作需显式授权，V3 remote入口待封，系统 Tailscale identity不改 | C05,C29,C30,C32,C34,C38,C41；WP08/WP10a；T23,T28,T34,T45,T56；D09 |
| R19 transient conversion OS temp（独立于 R08 update readiness）：`%TEMP%/gogoke-image-<safeStem>-<millis>.jpg` 与 `%TEMP%/gogoke-icon-<safeName>-<millis>.png` | HEIF send/read-image chain；open-app icon Tauri+daemon chain，icon链先经 `defaults read` 读取 bundle metadata | HEIF `/usr/bin/sips`与icon PATH `sips`创建，host读取，正常/已知失败分支尝试 remove；`defaults`只读 bundle metadata但属于同一外部 process chain | 非 authority、不得迁移；可能含用户附件像素或本机 app icon。HEIF 读失败分支可在 cleanup 前返回，两链 remove 都忽略失败，故可能残留；毫秒命名、`defaults`/`sips` binary identity、PATH与权限需核。WP03管3个process call sites/lifecycle，WP08管附件/path，WP05管icon UI | C01,C10,C32,C33,C38,C40；WP03/WP05/WP08；T21,T22,T35,T49,T50,T52,T54,T60；D02,D12 |
| R20 browser notification audio source/destination | bundled `success-notification.mp3`/`error-notification.mp3` URL → frontend fetch/decode → process-local AudioContext/gain/source → OS default audio destination | browser fetch/read/decode and ephemeral media graph；无 durable product writer | OS device/mixer是外部 authority；不得从播放请求推定可听/送达/已读。失败只写 debug，声音时序可能暴露活动类型；不进入迁移 | C01,C03,C35,C38；WP05；T11,T12,T23,T54,T66；D01 only for shell decoupling |
| R21 Windows uninstall registry+backup | embedded PS `$uninstallKey=HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke`；`$registryBackup=$FailureLog.registry.reg`；`$regExe=$env:SystemRoot\System32\reg.exe` | query/export before install；rollback import/delete；backup/failure/target cleanup | installer registration外部authority，不能由import改变产品trust；PS源码已展开但registry/process实际身份未验；WP12阻塞正式候选 | C33,C38,C39,C40；WP12；T38,T42,T46,T50,T52,T59,T60；D12 |

## 5. 资产处置与 C/D 映射

本表与 P00 §3 的资产处置逐项对应；它只补机械追踪，不改变处置结论。

| Asset ID / P00 资产 | 处置 | 具体能力/退出映射 |
|---|---|---|
| A01 `apps/desktop` layout/messages/design system/IME/Git/Files/PTY | 保留 | C01,C02,C03,C08,C09,C10,C11,C28,C29,C32,C35；D01,D02,D03 仅替换专属 binding，不删 UI |
| A02 Rust Codex app-server/shared cores | 修复后复用为 adapter | C11,C12,C13,C14,C15,C16,C17,C20,C21,C33,C34,C38；D02,D03,D04,D08,D09,D10 |
| A03 `seat-runtime.ts` | 修复后复用 | C12,C14,C20,C21,C33,C38；D12 在单一生产实现闭合后处理重复 helper |
| A04 `seat.ts` native input/delivery | 修复后复用 | C12,C15,C20,C21,C38；D12 只删被新合同替代的重复/死实现 |
| A05 `seat-runtime/close.ts` | 单一 process-control 候选 | C14,C33,C38；T52/T63；D12 明确映射，与 Room `proc.ts` 只能保留一套已验 production semantics |
| A06 seat-runtime provider fixtures | 测试复用 | C11,C15,C21,C33,C38,C40；fixture 不作 native evidence；D12 不误删验收夹具 |
| A07 Room `accounts.ts` | 修复后选择性复用 | C20,C22,C37,C38；真实 login/probe 路径不整体导入；D12 仅在替代已验后移除重复 helper |
| A08 Room `proc.ts` | 机制复用、实现待整合 | C14,C33,C38；T52/T63；D12 与 A05 的 process identity/termination 重复闭合 |
| A09 Room `cli-input-fixture.ts` | fixture 复用 | C15,C38,C40；显式 fake/read-only；D12 不把 fixture 当 dead code误删 |
| A10 Room server/调度权威 | 不整体采用 | C19,C33,C34,C40,C42；不得恢复 Room 组织调度/第二 writer；D09,D12 |
| A11 `packages/protocol` generated types/validators | 复用 | C11,C12,C40,C42；保持 schema/codegen 单一权威；D12 只去重复协议实现 |
| A12 `gogo-protocol/core/store` crates | 证据/机制复用，非批量导入 | C12,C33,C37,C40；不导入另一 kernel authority；D12 |
| A13 `tools/protocol-conformance` | 历史工具/方法输入 | C40；旧结果非当前 PASS；D12 不误删可复现实验资产 |
| A14 draft v1alpha1 schema fixtures | 合同 fixture 输入 | C11,C12,C18,C40,C42；`DRAFT_SCHEMA_ONLY`；D12 不冒充 production schema亦不误删 |
| A15 remote/Tailscale/dictation | V3 目标 `preserved_disabled`，当前尚 source-active | C34,C36,C41；T56/T57；不删除缓存/恢复资产，D09 封旧执行入口 |
| A16 macOS HEIF/icon conversion helpers | 修复后复用 | C01,C10,C32,C33,C38,C40；固定 temp lifecycle、2个sips+1个defaults call-site identity、cleanup failure 与 packaging；T35,T50,T52,T54,T60；D12 在重复/死 helper 已证明后才清理 |
| A17 browser notification audio helper/assets | 保留、修复后验收 | C01,C03,C35,C38；保留声音体验但补权限/autoplay/device/failure/privacy证据；T11,T12,T23,T54,T66；非专属实现，不直接删除 |

## 6. localStorage Writer 全表

| Key/family | Reader/writer | 内容/authority | 迁移与测试 |
|---|---|---|---|
| `open-workspace-app` | settings/open-app menu get+set | 外部打开应用偏好；UI-only | 按设置保留；T12,T37 |
| `composerEditorExpanded` | composer editor get+set | 编辑器展开偏好 | UI-only；T07,T12,T37 |
| `codexmonitor.promptHistory.<scope>` | prompt history get/set/remove | 输入历史，可能含私有文本 | 按 privacy domain/Session迁移并脱敏导出；T11,T23,T37,T49 |
| `gogo-party.pendingPostUpdateVersion` | updater set/get/remove | post-update notice/version hint | 不作为安装成功 authority；T42,T46,T59 |
| `codexmonitor.threadLastUserActivity` | thread storage get+set | 每 workspace/thread activity | 映射公共 Session/read version；T08,T37,T66 |
| `codexmonitor.pinnedThreads` | thread storage get+set | pin metadata | 映射 Session；T09,T37 |
| `codexmonitor.threadCustomNames` | thread storage get+set | custom title | 映射 Session并保留用户值；T09,T37 |
| `codexmonitor.threadCodexParams` | thread storage get+set | model/effort/access/collaboration/Codex args override | provider-specific fields进 NativeBinding/extension，不成公共 authority；T10,T37,T51,T58 |
| `codexmonitor.detachedReviewLinks` | thread storage get+set | review link mapping | 固定 Git/material identity后迁移；T28,T37 |
| `codexmonitor.collapsedGroups` | Sidebar/useCollapsedGroups get+set | UI grouping | UI-only；T08,T12,T37 |
| `codexmonitor.sidebarWidth` | resizable panels get+set | layout | T12,T37 |
| `codexmonitor.rightPanelWidth` | resizable panels get+set | layout | T12,T37 |
| `codexmonitor.chatDiffSplitPositionPercent` | resizable panels get+set | layout | T07,T12,T37 |
| `codexmonitor.planPanelHeight` | resizable panels get+set | layout | T12,T37 |
| `codexmonitor.terminalPanelHeight` | resizable panels get+set | layout | T12,T33,T37 |
| `codexmonitor.debugPanelHeight` | resizable panels get+set | layout | T12,T23,T37 |
| `codexmonitor.sidebarCollapsed` | sidebar toggles get+set | layout | T12,T37 |
| `codexmonitor.rightPanelCollapsed` | sidebar toggles get+set | layout | T12,T37 |
| `codexmonitor.threadListSortKey` | thread-list sort get+set | sort preference | T08,T37 |
| `codexmonitor.threadListOrganizeMode` | thread-list organize get+set | grouping preference | T08,T37 |
| `reduceTransparency` | transparency preference get+set | accessibility/material preference | T12,T37 |
| `mobile-remote-workspace-recent-paths` | workspace dialogs get+set | remote path history | remote target preserved-disabled；保留来源，不自动恢复 remote；T23,T37,T45,T56 |

## 7. Tauri/daemon surface census

### 7.1 Tauri-only 28

`app_build_type`, `codex_update`, `dictation_cancel`, `dictation_cancel_download`, `dictation_download_model`, `dictation_model_status`, `dictation_remove_model`, `dictation_request_permission`, `dictation_start`, `dictation_stop`, `gogoke_update_check`, `gogoke_update_install`, `gogoke_update_signal_ready`, `gogoke_update_take_failure`, `is_mobile_runtime`, `read_image_as_data_url`, `set_tray_recent_threads`, `set_tray_session_usage`, `tailscale_daemon_command_preview`, `tailscale_daemon_start`, `tailscale_daemon_status`, `tailscale_daemon_stop`, `tailscale_status`, `terminal_close`, `terminal_open`, `terminal_resize`, `terminal_write`, `write_text_file`.

### 7.2 daemon-only 3

`ping`, `daemon_info`, `daemon_shutdown`。

### 7.3 shared Tauri+daemon 101

其余 registry 命令均在 §2 各组列明并与 daemon method 集合交集一致。后续 validator 应从 dispatcher 可达 handler 重算 literal arms，再解析 `git_rpc::METHOD_*` arms/definitions，最终核 129/104/101/28/3，而不是把数字写成永不变化的常量。

## 8. 当前封存判定与后续使用

- 当前没有源码证据允许把 remote/Tailscale 或 dictation 标为已经 `preserved_disabled`：命令注册仍在，settings/update/startup 仍可 start/restart daemon，dictation UI/service/backend 仍可申请权限、下载模型和启动录音。
- WP10a 必须消费本文逐项封 UI、startup、settings side effect、direct IPC/RPC、old config/deep link/auto-start；本地 daemon 能力与 remote exposure 分开处理。
- WP01 的 machine-readable execution/trace artifact 必须把本 inventory 的 entry/root IDs、C/T/D 映射与遗漏检测纳入 T61/T68.D。本文不是缺失 execution JSON 的替代品。
- fresh review 必须从同一 commit 重跑集合提取，确认 129/129/104/101/28/3、localStorage 43-call/22-family、11 个精确 navigator clipboard sinks + 1 个 alias sink、opener 9/12、notification native sink 1；platform startup 还须核 3 个 Linux `set_var` sites、macOS close/reopen 3个window mutations+prevent-close、tray initialize 1、Windows chrome mutations 2。已知 transient conversion process census 必须为 2×sips + 1×defaults，browser audio 必须核 1 construction/resume/fetch-output chain 与2类 caller。最后机械确认 C01-C42、D01-D12 无覆盖空洞；任何差异先修 inventory，不可把缺项标 N/A。

## 9. 双向源码闭包台账（尚未完成）

### 9.1 固定 tracked 范围、引用规则与仪器状态

当前基线 `bedc8ff7cb18f7ff2f188123950f25e27621b3c9`。初始manifest为 `git ls-files apps/desktop tools/publish-gogoke-release.ps1 tools/sign-gogoke-release-manifest.ps1 tools/smoke-gogoke-update.ps1 .github/workflows/gogoke-desktop.yml`，共777文件；按Git输出顺序、UTF-8、LF连接并末尾LF的filename manifest SHA-256=`6AA9226AD720F9A48807315FC9850A6BE988E1D546C7F830A728878BFF3AF185`。其中desktop为773：95 Rust、347 TS、218 TSX、1 HTML、1 PS、5 SH、8 MJS等。计数包括测试/静态资源，不是777条production入口。后续发现跨范围引用必须扩展manifest，不得以此目录边界截断真实调用。

正向种子是HTML script、`src/main.tsx`、Rust main/lib/bin、CLI parse、transport认证与business dispatcher、plugin/lifecycle、package/build/release workflow、嵌入PS。逐平台并集展开mod/`#[path]`/include/import/script references，不以Windows编译时不可达为由漏macOS/Linux/iOS。终点必须是已读的纯计算/读取、明确副作用/root，或带lock identity的外部依赖；仓库内文件仍未展开只能登记 `READABLE_FRONTIER_NOT_EXPANDED`，不能用UNKNOWN掩盖。

反向种子包括file create/write/rename/remove、registry、storage、clipboard、media、network、permission、OS mutation、spawn/PTY/kill/env。必须从sink反找全部caller与wrapper alias，和正向集合逐成员对账，包含正向、拒绝、异常、cleanup与platform guards。数字相等只是仪器一致，**不是closure证据**。

实际仪器仍有未解释frontier：现有whole-scope反扫没有形成所有sink的逐成员zero-orphan proof。下列已展开链和wrapper-candidate集合不能替代§9.6未完成项。P00必须保持 `NOT_ACCEPTED`，本Capsule不能报completed。

### 9.2 F01：嵌入PS与动态program/registry全分支

`gogoke_update.rs:29` include_str固定嵌入 `src-tauri/update/gogoke-update-coordinator.ps1`；脚本SHA-256=`D09E54621735557176AB19C1F688595F79BE2DF44DBF1E2170D5456B0EB108A4`。Rust写stage script并启动PowerShell，不是external未知源码。已逐分支读取：

- `$Installer/$CurrentExe/$TargetDir/$ReadyFile/$LockFile/$FailureLog`先绝对路径校验；installer存在、digest格式、OS排他FileShare.None锁；等待父进程后再次校验installer hash。
- 拒绝reparse target与已有backup；`Invoke-Reg`动态 `$regExe` query；已有uninstall registration则export到 `$FailureLog.registry.reg`；target必须含gogoke.exe才Move-Item到`$TargetDir.update-backup`。
- installer通过动态 `Start-Process -FilePath $Installer`执行；非零throw。新exe为`$TargetDir/gogoke.exe`，带ready参数启动；missing exe、wrong version、早退、超时均throw。
- 正向ready闭合后删除ready/旧target backup/registry backup/failure log；释放锁。
- catch先写failure（含phase），若newProcess存活则Stop-Process+wait；rollback在无旧target且安装已开始时可启动`$TargetDir/uninstall.exe /S`，移除new target；有旧backup则Move回来。
- registry backup缺失且本来有registration时throw；有backup则reg import+remove backup；本来无registration但安装创建了则reg query/delete；install未开始仅清理已导出backup。
- 若旧app已停、未NoRestartOnRollback且CurrentExe存在，则动态Start-Process旧exe；rollback错误另写failure；exit1，finally dispose锁。Start-Process调用4个（installer/newexe/uninstaller/oldexe），reg operation类型4个（query/export/import/delete，query两分支共5调用点），Stop-Process1。
- 这些是源码分支，不证明PID创建时间、registry snapshot一致、cleanup安全或安装/回滚成功。C14/C33/C38/C39/C40→WP12；T38/T42/T46/T50/T52/T59/T60；D12。不得在本Capsule运行或读取真实registry/秘密。

### 9.3 F02/F03：权限、daemon CLI与wire分支

**Notification** `services/tauri.ts:1149-1221`：`is_macos_debug_build` query失败按false；macOS debug分支直接尝试fallback并return。非debug导入plugin后isPermissionGranted；false则requestPermission；非granted（包括denied）warn并尝试fallback后return；import/query/request/send exception warn后尝试fallback。fallback捕获自身错误返回false，但caller忽略boolean。app和daemon各一个 `osascript` call site，只在macOS+debug cfg下执行；其余平台/正式build返回Err。不得声称拒绝权限后绕过成功，更不得把wrapper resolved Promise当可见通知。C15/C35/C38，WP05/WP10b，T11/T19/T23/T44/T50/T52/T54，D09/D12。

**daemonctl** `bin/gogoke_daemonctl.rs:74-147`四个CLI种子：command-preview只读settings/path并打印命令；status有TCP+listener process观察，非无副作用文件读取；stop调用shutdown/可能Unix signals再probe；start可shutdown/restart/port bind/spawn。parse未知命令/参数、listen parse、missing token等错误向stderr/exit1。

- data root：Windows APPDATA非空优先，其次USERPROFILE（缺失回`.`）；macOS HOME（缺失回`.`）/Library/Application Support/identifier；其它平台XDG非空优先，其次HOME（缺失回`.`）/.local/share/identifier。daemon standalone同样XDG优先、HOME缺失回`.`。全部cwd-relative fallback违反V3，仍未修复，WP02/WP10b、T39/T41/T43/T55/T63。
- TCP probe：connect timeout/不可达、ping/daemon_info、auth retry、wrong/missing token/invalid result各自分类；shutdown另connect/auth后请求daemon_shutdown，等待再probe，不能把ack当stopped。
- 启动有token要求（insecure-no-auth为显式CLI分支）；已运行但wrong ownership/auth拒绝；version/mode不匹配会shutdown/restart，只有auth+managed daemon允许fallback signals；port非daemon占用拒绝；TcpListener::bind确认可用再spawn选定daemon binary+listen+data-dir+token/insecure args。
- Unix listener解析：lsof先行；Linux额外ss→netstat，缺binary/非零/坏输出None；PID必须>1、地址loopback/unspecified、expected PID如有须匹配。kill(0)观察，SIGTERM等待→SIGKILL等待；ESRCH视不存在，其它errno报错，残留报still-running。non-Unix find_listener=None且external PID kill直接Err。**这些guard不包含PID创建时间，不能升级成精确process identity证明。**
- 对应Tailscale adapter也有独立lsof/SIGTERM/SIGKILL路径；macOS `/bin/launchctl asuser <uid> <tailscale binary>`先行、失败再direct tailscale；TERM按env或xterm-256color设置。不能把两实现当同一已验收实现。

**wire** `bin/gogoke_daemon/transport.rs`在dispatcher之前处理auth：token=None立即authenticated并subscribe；token=Some时非auth回unauthorized，错token回invalid-token且不subscribe，匹配token回ok后subscribe；read EOF退出后abort events/write任务。JSON parse失败continue、无ID错误无法response；request semaphore和业务task在认证后使用。business方法104 + transport-only auth1 = wire union105；事件subscription不是第106个method，是wire lifecycle side effect。C11/C19/C33/C34/C38/C41→WP04/WP10a/WP10b，T16/T19/T24/T41/T43/T44/T54/T56/T63，D09。

### 9.4 依赖/平台/transport引用边界

| Node | 已展开/边界 | Owner/stage与映射 |
|---|---|---|
| main→fix_path_env | `main.rs:5-8` Err打印后继续run；Cargo.lock fix-path-env v0.0.0 source=`git+https://github.com/tauri-apps/fix-path-env-rs#c4c45d503ea115a839aae718d02f79e7c7f0f673`。源码不在tracked tree；不虚构其shell/env实现 | external source/actual-load boundary；WP03/WP12；C01,C33,C38,C40；T50,T59,T60；D12 |
| iOS setup→window | `lib.rs:166-170`→`window.rs:80-125`：native scrollView inset adjustment+indicator insets，controller extended edges+opaque bars，共4mutation；null分别skip，with_webview error返回 | WP05/WP10b；C01,C02,C09,C34；T12,T45,T54；D01；Windows不背书iOS |
| get/update settings→window theme | `settings/mod.rs:10-34`→`window.rs:17-77` set_theme；macOS main-thread NSWindow.setAppearance（system None、light/dark指定）。run_on_main_thread错误返回，inside appearance error忽略 | WP05；C01,C02；T12,T54；D01；不等于实机通过 |
| client TCP/IO | `remote_backend/tcp_transport.rs` connect→`transport.rs` boundedout queue512、newline writes，reader response pending remove、3类notification app.emit；bad帧discard，EOF/write failure drain pending为disconnected | WP10a/WP10b；C11,C28,C33,C34,C38,C41；T16,T41,T43,T44,T54,T56；D09 |
| plugin native boundary | npm lock固定API2.10.1/dialog2.6.0/notification2.3.3/opener2.5.3/liquid-glass-api0.1.6/xterm5.5.0（完整resolved/integrity在锁文件）；调用面可读已展开，native实现/OS permission为外部依赖，不能由版本号声称行为 | WP03/WP05/WP12；C01,C28,C35,C38,C39,C40；T12,T23,T50,T54,T59,T60；D12 |

lock identity：Cargo.lock SHA-256=`FD5914500E6F636331025943C85A0F090F603CCD0539902E0F9F60F8F93B49B6`；package-lock.json SHA-256=`068033413BC5C8801BBF826A644693BC81981B9D3021D12262C50A5E31DFFBD8`。固定lock≠加载版本、native permission或runtime验证。

### 9.5 process_core alias/wrapper逐成员candidate census

读了 `shared/process_core.rs`：tokio/std wrapper只构建Command+Windows CREATE_NO_WINDOW，不过滤env、不创建授权；kill_child_process_tree在Windows taskkill `/PID /T /F`忽略status后child.kill，其它平台仅child.kill，不保证子孙全停。必须追最终program、env/cwd、stream和stop caller，不可仅扫Command::new。

以下从tracked Rust每文件第一处顶层`#[cfg(test)]`前扫描，49个constructor/call candidates（51行含2个wrapper函数声明；**prefix-cut仅candidate仪器，不能证明cfg之后没有production定义**）：

| Source / call-site成员 | program→entry/side effect | 映射与闭包状态 |
|---|---|---|
| backend/app_server.rs:675,683,691 | Windows cmd/resolved executable或nonWindows selectedCodex；check version、app-server/doctor/background generation调用 | C11,C13,C14,C20,C21,C33,C38；E04-E07；WP03/WP09；T50,T51,T52,T58；D02,D04,D08,D09；部分实际env/caller分支仍frontier |
| bin/gogoke_daemon.rs:1388; notifications.rs:38 | 两个osascript notification impl | C35,C38；E22；WP05/WP10b；T23,T50,T54；D09,D12；分支已读 |
| bin/gogoke_daemonctl.rs:861,890,904,1098 | lsof/ss/netstat/selected daemon binary | C14,C33,C34,C38,C41；E23；WP10b；T43,T50,T52,T56,T63；D09,D12；平台/拒绝分支已读 |
| gogoke_update.rs:598 | powershell动态路径→embedded PS | C33,C38,C39；E21；WP12；T42,T46,T50,T59；D12；PS全分支已读 |
| shared/codex_aux_core.rs:322 | node --version（doctor）；PATH来自codex path helper，原生help/version另经app-server builder | C20,C21,C33,C38；E07；WP09a；T06,T50,T58；D08；已读doctor分支 |
| shared/codex_core.rs:86; workspaces/macos.rs:66,170 | HEIF sips；icon defaults+sips | C01,C10,C32,C33,C38；E18/E19；WP03/WP08；T35,T50,T52,T54；D12；已读 |
| shared/codex_update_core.rs:38,68,103,128 | brew check→upgrade、npm list→install-g latest；before/after版本check；notfound/timeout/非零各自返回 | C20,C21,C33,C39；E07；WP09b/WP12；T06,T50,T58,T59；D08；分支已读，操作未授权 |
| shared/git_core.rs:23,52,67,86,98,114,140 | dynamic git_bin，各call `.env(PATH,git_env_path())`；repo operations | C05,C29,C33,C38；E10；WP08；T22,T34,T50；D12；逐member错误/cleanup仍frontier |
| shared/git_ui_core/commands.rs:20,46; github.rs:152,178,206,235,265,298,337 | git或gh；command wrappers与GitHub remote观察/mutation | C29,C30,C33,C38；E10；WP08；T23,T28,T34,T50；D12；逐member权限/错误仍frontier |
| shared/git_ui_core/diff.rs:151; workspaces_core/git_orchestration.rs:121 | std/tokio dynamic git，stdin/path/patch；diff child.kill和read thread | C11,C29,C33,C38；E10/E04；WP08；T22,T34,T50,T52；D12；逐memberstop/error仍frontier |
| shared/process_core.rs:23,29,38 | wrapper constructors；taskkill（调用2个wrapper不能重复记第二执行链） | C14,C33,C38；WP03；T50,T52,T63；D12；原实现不含创建时间guard |
| shared/workspaces_core/io.rs:206,214,222,246,252,266 | windows cmd/resolved command、nonwin usercommand、macOS app-cli/open -a、nonmac app | C32,C33,C38；E04/E17；WP08；T22,T50,T54；D12；program/path/empty/exiterror分支已读 |
| tailscale/daemon_commands.rs:215; tailscale/mod.rs:40,47,345 | selected daemon；direct tailscale/launchctl asuser/lsof | C14,C33,C34,C38,C41；E14/E23；WP10a/WP10b；T43,T50,T52,T56,T63；D09,D12；adapter完整caller/cleanup仍frontier |

非CommandBuilder遗漏检测：`terminal.rs` portable_pty CommandBuilder/native_pty_system/openpty、TERM/LANG/LC_*、spawn_command与child.kill必须独立追；不是上述49个Command candidates的一部分。kill wrapper production caller候选为app_server:1093、daemon:236、connect:122、crud_persistence:87/241/406、runtime_codex_args:120、tailscale daemon_commands:250。裸child.kill另见terminal:269/369与diff:193；Unix libc::kill另见daemonctl与Tailscale观察/TERM/KILL。这些现已定位，但尚无逐成员zero-orphan closure证明。

### 9.6 readable-source frontier与孤立sink状态（不得报完成）

| Frontier ID | 尚未解释的可读范围/工作 | Owner / 阻塞与映射 |
|---|---|---|
| CF01 | 777-file引用manifest未生成逐文件mod/include/import/script adjacency与platform reachability矩阵；静态资源/tests尚未逐成员分类成pure/read/not-production | Sol-high WP00；阻塞P00 acceptance/P01依赖；C40/T48/T61/T68；非runtime未知 |
| CF02 | Git/gh/wrapper、PTY/kill的candidate全caller反向解析和每member permission/reject/error/cleanup尚未全部展开，见§9.5精确文件；49仅候选数 | Sol-high WP00，WP03/WP08机制owner；P00 acceptance；C14,C28,C29,C30,C33,C38；T22,T34,T50,T52,T63；D09,D12 |
| CF03 | frontend全TS/TSX sink→alias→caller closure尚未完成；已有clipboard/plugin/audio/storage计数并非全部DOM/media/network/import分支的proof；postUpdateRelease.ts fetch等需逐member收口 | Sol-high WP00/WP05；P00 acceptance；C01,C03,C35,C38,C40；T11,T23,T54,T66/T68 |
| CF04 | package scripts/tauri config/doctor/icon sync/iOS build-release 与workflow/tool publication链尚未逐branch对账；发布脚本是可读source不是UNKNOWN：publish解包临时portable检查→签名tool→gh release，NewKey写secret/public，smoke启动installer/coordinator/uninstaller等均需最终双向sink台账 | Sol-high WP00/WP12；P00 acceptance/相关发布冻结；C39,C40；T42,T46,T59,T60/T68；D12 |
| CF05 | file/registry/network/permission/OS/env反扫候选未形成逐sink入度证明，**孤立sink数当前NOT_COMPUTED，不得写0**；dictation内录音/permission/download/native threads、remote RPC auth调用、plugin native lock边界等需逐branch核 | Sol-high WP00；P00 acceptance；C33,C34,C36,C38,C40,C41；T49,T50,T54,T56,T57/T68 |

这是施工未完成frontier，不是待Owner决定的UNKNOWN或改变范围。已读仓库分支在§9.2-9.4具体记录；未读仓库分支必须继续展开。外部依赖只有在lock identity+owner/stage固定后才可作为boundary。现阶段不得以局部已修、C/D零空洞或registry counts吻合签发“P00 completed”。
