# gogoke P00 接手摸排 R2：Git、Workspace 与外部进程真实链

**日期：2026-09-18｜源码基线：`19eda05136de842394f1541ec2d8c05c03170d1f`｜证据层级：SOURCE_BRANCHES_EXPANDED**

本批承接 R1 的 `RF01` 与 `RF02`。它记录实际自动入口、UI 回调、Tauri/remote 分支、shared core 顺序、磁盘/仓库/进程副作用和失败后的剩余状态。它不是代码修复、运行验证或 P00 验收。

- `P00 acceptance = NOT_ACCEPTED`
- `global orphan_sink_count = null / NOT_COMPUTED`
- 未运行 Git mutation、外部 app、CLI、用户工作区、网络、登录或模型调用。
- R1/R2 的“记录数”是观察行，不是功能数、漏洞数或通过测试数。

## 1. 本批最重要的设计输入

1. **Workspace 不等于进程。** 当前所有 workspace 可以指向同一个 `WorkspaceSession`；连接会复用 map 中任意存活 session。运行参数改变时，代码把全部 session key 先切到一个新 session，再杀旧进程。
2. **Git 和 Workspace 操作普遍不是事务。** commit+push、clone+spawn+persist、worktree branch rename+move+persist、远端 branch rename、pull fallback 都可能在报错前已经改变状态。
3. **存在自动读写入口。** Git status 每 3 秒轮询，Git log 在可见条件下每 10 秒轮询；workspace 首次 load 后自动连接。不能只盘按钮。
4. **本地、daemon 和 remote 路径并不天然等价。** `workspaces/commands.rs` 中部分命令转发 remote，部分直接执行 local core；`add_worktree` 的本地 adapter 没有提供 remote-tracking finder。
5. **外部路径与程序边界比 workspace 宽。** Git root 可以是 workspace 外的绝对目录；open-in-app 可执行用户配置的 command/app；这些是保留能力，但必须成为明确权限与目标边界。

## 2. Git 读取、轮询与内容提取

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R2-001 | `useGitStatus` → `get_git_status` → `git_ui_core::get_git_status_inner` | 有 workspace 时立即读取，并每 **3000ms** 自动轮询。workspace 变更用 request id 拒绝迟到结果；失败时可继续显示缓存并附 error。后端读取 branch/status/index/diff stats，并额外启动 `git check-ignore --stdin -z`。 |
| R2-002 | `useGitPanelController.queueGitStatusRefresh` | 多个调用被 500ms debounce；活动 workspace 变化后拒绝旧 timer。它与 3 秒轮询并存，不是同一个入口。 |
| R2-003 | `useGitDiffs` + `useGitPanelController` | diff 是否加载由 preload、选中文件、split chat/diff、panel 可见性和来源共同决定。文件状态摘要变化会再次读取；workspace+ignoreWhitespace 是 cache key。preload 只在取得非空、loading 或 error 证据后标记完成，空且无错会继续保持可重试。 |
| R2-004 | `get_git_diffs_inner` | blocking 线程读取工作树、index、文本与图片。文本上限 2MiB，图片 10MiB；SVG 按 image mime 返回 base64。patch/读取失败的单项可被跳过而不让整请求失败。运行时渲染/CSP 安全留给 WP05/T54。 |
| R2-005 | `collect_workspace_diff`（提交信息生成输入） | 先尝试 HEAD→index；只要 staged combined diff 非空就立即返回，之后不包含 unstaged/untracked。仅 staged 为空时才读 workdir+index。生成提交信息不能被描述成始终覆盖“全部当前改动”。 |
| R2-006 | `collect_ignored_paths_with_git` | 启动 `git check-ignore`，并行读 stdout；输入写失败时 kill+wait+join。exit 0/1 都算有效，其他状态返回 None 并回退 libgit2 ignore。kill/wait 结果不提升为进程树已收尾证明。 |
| R2-007 | `useGitLog` → `get_git_log_inner` | 只有 log/diff 相关 UI enabled 时立即加载并每 **10000ms** 轮询。后端为算 `total` 遍历一次完整 history，再遍历第二次取 limit；还计算 upstream ahead/behind 和两边 commit 列表。长历史性能需在 T47 验证。 |
| R2-008 | `useGitRemote` → `get_git_remote_inner` | workspace 切换时一次读取；优先 `origin`，否则取第一个 remote。没有 remote 返回 null；不证明远端可访问或认证有效。 |
| R2-009 | `useGitHubIssues`/`useGitHubPullRequests` | panel enabled 时读取。后端先执行 `gh issue/pr list --limit 50`，再单独执行 `gh api /search/issues` 取 total；第二次失败会退回已列项目数，因此 total 可能是截断后的 fallback。 |
| R2-010 | `get_github_pull_request_diff/comments` | 分别启动 `gh pr diff` 和 `gh api`；diff parser按文本 header 拆文件。此处是网络/凭据/外部进程入口，不是纯展示函数。 |
| R2-011 | `useGitRepoScan` → `list_git_roots` | 仅显式 scan；深度 1–6。walker 不 follow symlink，跳过 `.git/node_modules/dist/target/release-artifacts`，最多 200 项；发现 `.git` 文件也算仓库。 |
| R2-012 | `useGitRootSelection` → workspace settings → `resolve_git_root` | 选择在 workspace 内时存相对路径，外部选择存绝对路径；backend 只要求目标是目录，**不要求包含于 workspace**。因此 Git mutation 权限域不能默认等于 workspace root。 |

## 3. Git mutation 与部分成功

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R2-013 | stage/unstage/revert file | rename 状态可能展开 old+new 两条 path。stage 用 `git add -A --`；unstage 用 `git restore --staged`。单文件 revert 先 `restore --staged --worktree`，失败就 `git clean -f`，并非无副作用探测。 |
| R2-014 | revert all | 前端有 warning confirm；backend执行 `git restore --staged --worktree -- .` 后 `git clean -f -d`。第一步成功、第二步失败时仍已丢弃 tracked changes。 |
| R2-015 | `useGitCommitController.ensureStagedForCommit` | 有 unstaged 且无 staged 时自动 `stageGitAll`；随后 commit。用户点击 commit 并不只提交当前 staged 集合。 |
| R2-016 | commit / commit+push / commit+sync | commit 成功后 push/sync 失败会分别显示后半段 error，并清空 commit message；仓库已经有新 commit。不能把返回错误解释为“没有提交”。 |
| R2-017 | push/pull/fetch/sync helpers | 有 upstream 时 push 前 best-effort fetch（fetch 错误被忽略）再 push `HEAD:<branch>`；pull 依次尝试 `--autostash`、无 autostash、`--no-rebase` 等 fallback；sync 是 pull 后 push。重试链每一步都可能已经改变 worktree/index/refs。 |
| R2-018 | init Git | UI 对非空目录有第二次确认；backend可能已完成 `git init` 和 HEAD symbolic ref，但 initial add/commit 失败时返回 `status=initialized, commitError`，前端明确提示手工补 commit。 |
| R2-019 | create GitHub repo | 可能执行 `gh repo create`、解析/新增 origin、push `HEAD`、PATCH default branch。push 或 PATCH 失败返回 `status=partial`；远端仓库及 local origin 已可能存在。 |
| R2-020 | checkout/create branch | checkout 使用 libgit2 safe checkout 后 set HEAD。create 直接 `repo.branch(..., false)` 再 checkout；该 core 没调用 `validate_branch_name`。输入约束取决于上游 UI/libgit2拒绝，需在后续合同固定。 |
| R2-021 | checkout PR | `gh pr checkout` 直接改变当前工作树/branch，成功后 UI 刷 branch/status/log。它不是只读 GitHub 查询。 |
| R2-022 | apply worktree changes | 收集 staged、unstaged、untracked binary patch，再在父 repo执行 `git apply --3way`。错误文本含 “Applied patch ... with conflicts/partially” 时返回 error，但父仓库已经改变。 |

## 4. Workspace、clone、worktree 与 shared session

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R2-023 | `useWorkspaces` mount → `refreshWorkspaces` | 首次 render 自动 list。list 失败仍把 `hasLoaded=true`；之后 restore effect看到的可能是空/旧 UI 集合。失败不是“尚未加载”。 |
| R2-024 | `useWorkspaceRestore`（R1补充） | `hasLoaded` 后自动逐项 connect，不等用户打开 workspace；连接成功后列最多 6 页线程。restore 与 Git 自动轮询是两组独立自动入口。 |
| R2-025 | add-many paths | 对输入去重并顺序处理；`~`/`~/x` 会附加从现有 Unix 路径推断的 home 候选。每个 candidate先 remote/local `is_workspace_path_dir`，再 add；可能一个批次部分成功、部分失败。 |
| R2-026 | `add_workspace_core` | 验证目录后先选任意 shared live session或 spawn Codex，再写 workspaces.json，最后 register path和 map key。write失败会移除内存 entry；仅新 spawn 才 kill。shared旧 session无需回滚，因为此前还没 register。 |
| R2-027 | `add_clone_core` | 先 mkdir copies root、`git clone` 本地 source；可能 best-effort把 origin改为 source origin；再选/spawn session并持久化。spawn/write失败会 best-effort删 clone目录；cleanup失败未升级。 |
| R2-028 | `add_workspace_from_git_url_core` | destination必须已存在；目标不存在或空目录可用。clone失败、spawn失败或写配置失败时 best-effort递归删目标。一个失败结果仍可能留下目录/部分 clone。 |
| R2-029 | `connect_workspace_core` | 全局 spawn lock；若该 key已有活 session则 no-op；否则会取 sessions map 中**任意第一个活 session**，为新 workspace register path并复用。没有按 binary/profile/auth/home/privacy 比较。 |
| R2-030 | `kill_session_by_id` | 先移除一个 workspace key并 unregister；只要同一个 Arc 仍被其他 key引用就不杀进程。最后一个引用才调用 process-tree kill；函数不返回 kill成功状态。 |
| R2-031 | runtime Codex args | 未连接时返回 `respawned=false`，但回传“applied args”只是目标值。已连接且改变时先 spawn新 session，再将**全部现有 session key**替换成新 Arc，逐个 register path，最后 kill旧 child。不是单 workspace原子重启。 |
| R2-032 | runtime args切换故障窗口 | 新 session spawn后、map替换/路径注册/旧进程kill之间没有事务或 generation；`register_workspace_with_path`无错误返回，kill结果不进入 result。P03必须定义 in-flight request和旧 writer封口。 |
| R2-033 | `add_worktree` local adapter | 本地 Tauri adapter向 core传 `git_find_remote_tracking_branch=None`；本地 branch不存在时 core直接 `git worktree add -b`，不会在此路径查 remote tracking。daemon adapter是否不同必须在 RF08 对账。 |
| R2-034 | add worktree顺序 | 创建目录/branch/worktree，可选复制 AGENTS，构造 entry，选/spawn session，写 registry，最后 register。与 add main不同，所读代码在 write失败时未见删除已建 worktree或新 spawn补偿。 |
| R2-035 | remove parent workspace | 逐 child先 kill session，再尝试删目录/worktree；允许 `continue_on_child_error` 时收集失败并继续。成功 child会先从最终 registry移除；有失败时父可保留并返回聚合 error。磁盘、session和 registry可处于部分完成状态。 |
| R2-036 | update workspace settings | 前端先 optimistic更新并在 service失败时回滚 UI。backend却先在内存 map应用 setting并传播 setup script到 children，再写 workspaces.json；**write失败没有恢复 backend内存快照**。前端回滚不等于后端回滚。 |
| R2-037 | rename worktree | 先 rename branch；如需再 move worktree；再改内存并写配置；失败时 best-effort反向 move/branch rename并恢复内存。回滚命令错误被忽略。session path仅在配置写成功后重新 register。 |
| R2-038 | rename upstream | 若旧远端 branch存在：先 push新 branch，再 delete旧 branch，最后 set-upstream；任何中间失败都可能已留下新旧远端状态变化。没有补偿。 |

## 5. Adapter、外部程序和本地/远程差异

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R2-039 | `workspaces/commands.rs` remote分支 | list/read/add/add-from-url/worktree/status/mark/remove/rename/settings/connect/files 等多项会经 `remote_backend::call_remote`；path有的先 normalize。remote error直接传播。 |
| R2-040 | adapter非统一remote覆盖 | `add_clone`、`apply_worktree_changes`、`open_workspace_in`、`get_open_app_icon` 在所读 adapter无 remote mode分支；会走本机 core/OS。remote UI中是否可触发以及 workspace state来源必须单独验证，不能用其余命令的remote支持外推。 |
| R2-041 | open workspace in app/command | command或app必须非空；按 Code/Cursor/Zed构造line/column参数。Windows `.cmd/.bat`经 `cmd /D /S /C`；macOS可能用CLI或`open -a`；其余直接spawn配置字符串。等待 process output并回传截断 stdout/stderr。 |
| R2-042 | external-open权限边界 | path、args、command/app来自上游设置/动作，core不验证目标包含于 workspace，也不做 allowlist。该能力应保留，但必须在C32/C38中定义“用户明确配置的外部程序”而非公共模型任意执行。 |
| R2-043 | workspace path与home耦合 | `normalize_workspace_path_input` 对 `~`调用 `codex::home::resolve_home_dir()`；这是通用 workspace选择对 Codex module的真实依赖，应替换为产品/platform home resolver，不删除 `~`能力。 |
| R2-044 | connected UI语义 | `list_workspaces_core` 仅以 sessions map是否含 key标 connected，不重新检查 child liveness。直到 connect path清死 session之前，UI connected可能是陈旧映射。 |

## 6. 当前闭合状态

- `RF01`：**源码批次 substantially expanded，尚未全闭。** 仍需把所有 Git frontend叶组件、菜单/快捷键、daemon Git handler和 remote transport/auth逐项对账；运行行为未验。
- `RF02`：**源码批次 substantially expanded，尚未全闭。** 仍需 workspace dialog/modal叶、daemon workspace adapter、clone/worktree UI所有入口、file read/list/open icon细分和 settings字段副作用完整对账。
- `CF01–CF05`：全部保持 OPEN。
- P00 仍为 `NOT_ACCEPTED`；本批不得用于启动生产修改。

## 7. 已读源码索引

固定提交均为 `19eda05136de842394f1541ec2d8c05c03170d1f`：

- `apps/desktop/src/features/git/hooks/useGitStatus.ts`
- `useGitDiffs.ts`, `useGitLog.ts`, `useGitRemote.ts`, `useGitBranches.ts`, `useGitHubIssues.ts`, `useGitHubPullRequests.ts`, `useGitRepoScan.ts`
- `apps/desktop/src/features/app/hooks/useGitPanelController.ts`, `useGitCommitController.ts`, `useGitRootSelection.ts`
- `apps/desktop/src/features/git/hooks/useGitActions.ts`
- `apps/desktop/src-tauri/src/git/mod.rs`, `git_utils.rs`
- `apps/desktop/src-tauri/src/shared/git_core.rs`, `shared/git_ui_core.rs`, `shared/git_ui_core/{context,diff,log,commands,github}.rs`
- `apps/desktop/src/features/workspaces/hooks/useWorkspaces.ts`, `useWorkspaceCrud.ts`
- `apps/desktop/src/features/app/hooks/useWorkspaceController.ts`, `useWorkspaceRestore.ts`
- `apps/desktop/src-tauri/src/workspaces/commands.rs`
- `apps/desktop/src-tauri/src/shared/workspaces_core/{connect,crud_persistence,git_orchestration,helpers,io,runtime_codex_args,worktree}.rs`

后续必须保留逐项链路，不能把本批再压缩成“Git已盘、Workspace已盘”。
