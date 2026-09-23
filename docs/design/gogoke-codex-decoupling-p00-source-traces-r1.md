# gogoke P00 接手摸排：真实入口与副作用链路 R1

**2026-09-18｜已执行的源码摸排结果，不是新的施工总计划。**

| 字段 | 当前事实 |
|---|---|
| 任务 | Owner 要求直接承接后续摸排，纠正反复漏掉真实入口的问题。 |
| 固定源码基线 | `taiyun668/gogo-party@bc665a852833952b76d9508401193bedd2198436`；来自 `codex/gogoke-decoupling-v3-b0`。 |
| 本批输出 | 55 条入口、回调或生命周期分支记录；每条包含可核对源码、效果、失败分支和未闭合边界。它们不是 55 个新增功能，也不是 55 个已复现漏洞。 |
| 本批层级 | `SOURCE_BRANCHES_EXPANDED`。已检查所列源码窗口，不代表全部调用者、全部条件编译、全部平台或动态行为已闭合。 |
| P00 状态 | **`NOT_ACCEPTED`**。原 Stage1 census 的 `INCOMPLETE_STAGE1` 不改写；本批仅增加 Stage2 源码追踪证据。 |
| 全仓孤立 sink 数 | **`null / NOT_COMPUTED`**。不得填写 0。 |
| 实际执行 | GitHub 源码读取、证据文档写入；没有运行产品、真实 CLI、安装/签名/发布脚本、麦克风或用户工作区命令。 |
| 写入范围 | 独立摸排分支中的新增证据文件；不修改生产代码、旧账本、旧 census、V3 主计划或模型路由。 |
| 独立复核 | 未完成。记录作者不签发 P00 验收。 |

## 1. 本批纠正了什么

**文件在 manifest 中、函数名有登记、调用点数量对得上，都不等于真实入口已追到最终效果。** 当前 `census.py` 自己就明确限定为文件及引用盘点，并把 orphan sink 留空；它没有声称完成 Stage2。14 项仪器自测和稳定生成通过，只能支持那些已测试规则，不能替代下列真实链路。[S01]

本批不再扩大一张泛化的“Git/终端/更新”功能表，而是检查实际入口的条件、别名、转发、后续调用、失败和清理。还补了一个反例：`git_ui_core.rs` 在顶层 `#[cfg(test)] mod tests;` **之后**仍定义生产函数。按首个测试标记截断整文件，会漏掉公开的生产转发层；不能用这种截断证明调用闭包。[S18]

另外两类误判同时避免：

- 不能把“返回错误”解释成“什么都没发生”：Git 初始化、创建远端、同步、worktree apply、创建/删除 worktree 都可能已有前序效果。
- 不能把“看起来像安装/发布的字符串”解释成真的执行：Windows doctor 打印安装建议但不安装；相反 `publish` 不带 `-Publish` 仍会读取签名密钥并写文件。[S24][S27][S28]

## 2. 读法与闭合边界

`Sxx` 是文末固定提交的源码证据；读取窗口不等于精确函数边界。`GIT-L` 表示本批已读的本地链：`lib.rs` 注册 → `git/mod.rs` 已读命令/宏 → `git_ui_core.rs` 生产转发 → `commands.rs` 或 `github.rs`。远程分支在 `call_remote` 处仍需继续检查认证、传输、daemon 分派和回执，不能借本地链通过一起关闭。[S10][S17][S18]

`PTY-L` 表示：`services/tauri.ts` 中四个 terminal wrapper → `lib.rs` 注册 → desktop `terminal.rs`。移动端使用另一模块，本批不把 desktop 行为外推到移动端。[S09][S10][S08]

**回调、Tauri 命令、内部公共函数、生命周期钩子、脚本和 workflow 都可以成为盘点根，但须标明根的层级。** 表中没有读到的 UI 按钮/快捷键、依赖内部、注入回调或另一平台都明确保留为 frontier；不把推测的相邻函数画成已经验证的边。

## 3. 已展开的真实入口记录

### 3.1 自动恢复、脚本与 PTY

覆盖原 CF02/CF03/CF05 的一部分；能力主要关联 C04/C05/C14/C28/C33/C38，后续机制归 WP03/WP05/WP08。不是删除终端或脚本的依据。

| ID | 实际根及已读链 | 条件、实际效果 | 失败/清理事实与剩余边界 |
|---|---|---|---|
| R1-001 | `useMainAppWorkspaceLifecycle → useWorkspaceRestore` 的 effect。[S04][S05] | `hasLoaded` 后挑未恢复 workspace；先将全部候选 ID 加入 Set，再逐个对未连接项调用 `connectWorkspace`，最后对成功项列线程，最多 6 页。**无需点击连接按钮。** | 连接失败被静默捕获；此 hook 内失败 ID 不从 Set 移除。最终 list 的拒绝未在此 IIFE 内捕获。`connectWorkspace` 的具体注入、focus/poll 两条兄弟链仍需展开。 |
| R1-002 | `useWorkspaceLaunchScript.onSaveLaunchScript`。[S06] | 保存原始脚本文本到 workspace settings；空白归 null。该动作是配置写入，不是执行脚本。 | 保存失败保留编辑状态/错误；实际 settings 持久化及所有表单入口仍需接续，不把保存算成已运行。 |
| R1-003 | `onRunLaunchScript → pendingRunRef → readyKey effect → writeTerminalSession`，MainApp 传入 `launch` 终端及 restart。[S03][S06][S09] | 无 workspace 不动作；空脚本打开编辑器；否则快照 workspace/terminal/script。只有 readyKey、活动终端和工作区三者匹配才清 pending 并写 `script + 换行`。 | restart 失败清 pending；写失败显示错误，但 pending 已清。无脚本退出/成功收据。最终 UI 叶按钮未全部读到，不宣称所有启动入口闭合。 |
| R1-004 | MainApp `handleWorktreeCreated → maybeRunWorktreeSetupScript`。[S03][S07] | 仅 worktree 且未在本地 running Set；查询状态后，`shouldRun && 非空脚本` 才打开固定 `worktree-setup` 终端，写脚本，再调用 mark-ran。 | **restart 报错后仅日志，仍继续 open/write。写完即 mark，不等脚本完成。** 写成功而 mark 失败可能留下可再次触发状态。创建模态的最终回调叶及两个 status/mark IPC adapter 还需逐边核对。 |
| R1-005 | `worktree_setup_status_core`。[S15] | 查 workspace；只有 worktree 才检查 marker；`should_run = worktree && script存在 && marker不存在`。这是文件存在性读取和决策。 | 不是终端进程或脚本成功检查；文件存在性的错误语义和并发创建另待真实测试。 |
| R1-006 | `worktree_setup_mark_ran_core → worktree_setup_marker_path`。[S15][S16] | 确认是 worktree，mkdir 后写 `<data_dir>/worktree-setup/<workspace_id>.ran`，内容为 `ran_at=epoch秒`。 | 不接收 exit code、执行 ID 或成功证据。该 marker 必须独立进入迁移台账，不能命名成“脚本验收成功”。 |
| R1-007 | `useTerminalController → useTerminalSession` 的可见性/open effect → `openTerminalSession → PTY-L`。[S11][S12][S08][S09] | 面板可见且有 workspace/terminal/渲染器时打开会话；后端非空 ID、已存在 registry、workspace 查找、openpty、shell spawn、reader/writer、registry 插入按顺序执行。 | backend 返回已有 registry 项时未验证 liveness。spawn 后 reader/writer 获取失败的路径没有显式等待杀进程；依赖 Drop 语义待查。UI open 完成回调在所读 effect 内未见取消/世代守卫，竞态未复现。 |
| R1-008 | xterm `terminal.onData → writeTerminalSession → terminal_write`。[S12][S09][S08] | 读取当前活动引用；opened Set 有键才发送。后端取 Session，阻塞任务 `write_all + flush`。这是真实 shell 输入，不是普通聊天消息。 | 常见关闭错误从前端 opened Set 移除；后端闭流错误按 key 移除 registry，未用 reader cleanup 的 `Arc::ptr_eq` 守卫。不能把输入已 flush 解释成 shell 命令成功。 |
| R1-009 | `ResizeObserver → resizeTerminalSession → terminal_resize`。[S12][S09][S08] | 可见且有会话时 fit，并将 cols/rows 送后端；后端最小 2×2，调用 master.resize。 | 关闭错误会移除 registry；observer cleanup disconnect。与模型终态无关，不能为了去 Codex 一起移除。 |
| R1-010 | Controller 的 close 回调 → `closeTerminalSession → terminal_close`。[S11][S09][S08] | 前端先清缓冲/显示状态；后端先从 registry 删除，再调用阻塞 `child.kill()`。 | 后端忽略 kill 结果及 join 结果后 `Ok(())`；前端忽略“not found”，其它错误写 debug。**成功返回不证明子进程树已经退出。** `useTerminalTabs` 对回调的全调用链仍未读完。 |
| R1-011 | `spawn_terminal_reader` 原生线程 → TerminalExit event → 前端 exit 订阅。[S08][S12][S11] | reader 的 EOF 或任何读取错误都结束循环并 emit terminal-exit；随后异步仅在 registry Arc 仍同一对象时移除。前端清状态并通知关闭标签。 | **流结束事件不携带 child exit status，也不等待 child退出。** 不能据此显示所有工具已停止；event hub/emit adapter 的完整分派仍需核。 |
| R1-012 | `terminal_open` 的第二次 registry 检查。[S08] | 两个并发 open 都可能先 spawn；后插入者发现已有项，尝试杀掉自己的候选 child，并返回已有 ID。 | 此补偿也忽略 kill/join 结果；不能用“registry只有一条”证明“从没产生第二个进程”。实际并发时序及依赖子孙清理尚未验证。 |
| R1-054 | `restartTerminalSession` 被单启动脚本/多脚本/setup 使用。[S03][S11] | 先清前端本地状态，再调用 close；只将“not found”视为可忽略。它本身**不打开新会话**，后续 open 来自 effect 或 setup 显式调用。 | 不能因函数名 restart 就登记一条原子重启。其它 close 异常会上抛；setup 对这个异常的处理与手动 launch 不同，见 R1-003/004。多脚本 hook 的内部仍是 frontier。 |

### 3.2 Worktree 的磁盘、配置与模型连接顺序

覆盖 C04/C05/C24/C29/C33/C38，后续归 WP03/WP07/WP08。以下是 core 内部实证，不冒充已确认所有 UI/RPC 调用者。

| ID | 实际根及已读链 | 条件、实际效果 | 失败/清理事实与剩余边界 |
|---|---|---|---|
| R1-013 | `add_worktree_core`。[S15] | 校验父项/分支；工作区配置目录优先于全局目录，再到 data_dir；mkdir；按本地已有分支/远程跟踪/新分支执行不同 worktree add；可选复制 AGENTS；继承 setup script；再选已有共享 Session 或 spawn；再写 workspace registry 并登记连接。 | **worktree磁盘创建在模型连接和配置持久化之前。** 所读函数对后续 spawn/write 失败没有完整反向补偿。不能记为单一原子“新会话”。注入的 git/spawn/path callbacks 及 rename 后半文件尚未全展开。 |
| R1-014 | `remove_worktree_core`。[S15] | 验证 worktree/parent；先 kill_session；目录存在而父目录不存在时直接 remove_dir_all；否则 git worktree remove --force；只有特定 missing-worktree 错误才回退删目录；prune；最后移除 registry/写配置。 | prune 错误被忽略；其它删除错误传播，之前的停止可能已发生。文件删除与配置写入并非一个事务；这不是源码摸排时允许实际执行的测试。 |
| R1-015 | `copy_agents_md_from_parent_to_worktree`。[S16][S15] | 源不是文件或目标已有文件则跳过；否则 copy→`AGENTS.md.tmp`→rename 正式文件。add_worktree 把其失败作为可选项日志。 | rename 失败会尽力删 temp；copy 失败路径未见同样清理。这里只证明文件复制，不证明任一家 CLI 已加载规则或获得授权。 |
| R1-016 | `useGitActions.applyWorktreeChanges` 与 `apply_worktree_changes_core/inner_core`。[S20][S21] | UI 仅 worktree；core确认父/子根，先要求父 status clean，拼 staged/unstaged/untracked binary patch，然后 **向父仓库** stdin 执行 `git apply --3way --whitespace=nowarn -`。 | stdout/stderr 可明确表明冲突或部分应用；**Err 不等于父仓库未改变。** stdin写失败的路径未显式等待补偿；UI用原workspaceID保护回显。IPC中间adapter尚待整段补读。 |

### 3.3 Git/GitHub：逐命令，不以一个“Git 功能”替代

覆盖 C05/C29/C30/C33/C38；主要机制归 WP08，进程/授权归 WP03。共同路径 GIT-L 见 §2。下表不外推外部 git/gh 的内部 hooks、凭据助手、子进程与网络规则。

| ID | 已读入口/实现 | 副作用及条件 | 失败与剩余边界 |
|---|---|---|---|
| R1-017 | `stage_git_file → stage_git_file_core → stage_git_file_inner`；前端别名 `stageGitFileService`。[S17][S18][S19][S20] | rename status 可能把一个文件映射为旧/新两条路径，逐条 `git add -A -- path`。 | 一条失败可发生在另一条已处理之后。路径normalize/resolve_git_root需继续核，不以存在 `--` 代替根权限。 |
| R1-018 | `stage_git_all → ... → stage_git_all_inner`。[S17][S18][S19][S20] | workspace确认后 `git add -A`，是索引写入。 | 前端 finally 仅当前workspace匹配才刷新；不应当作纯 Git 状态读取。 |
| R1-019 | `unstage_git_file → ... → unstage_git_file_inner`。[S17][S18][S19][S20] | rename路径逐条 `restore --staged -- path`。 | 多路径部分结果要保留；非零传播，不自行改成清理工作区。 |
| R1-020 | `revert_git_file → ... → revert_git_file_inner`。[S17][S18][S19][S20] | 先 `restore --staged --worktree -- path`；**任意该命令失败**都会尝试 `clean -f -- path`。 | fallback不只是“文件不存在”分支。本hook中没有与撤销全部相同的确认框；更上层是否有确认尚待逐UI叶核对，不能断言全链无确认。 |
| R1-021 | `useGitActions.revertAllGitChanges → revert_git_all`。[S20][S17][S18][S19] | 前端 ask 警告会丢 staged/unstaged/untracked；确认后 backend restore 整个目录，再 clean -f -d。 | 前端确认是可见保护，不是本批已证明的 daemon/服务端权限。restore成功而clean失败不是“完全未执行”。 |
| R1-022 | `commit_git → commit_git_core → commit_git_inner`。[S17][S18][S19] | `git commit -m <message>`，cwd为解析出的repo root。 | 不能按“只调用一次wrapper”排除git hooks等外部行为；本批不声称外部 hooks 已核或已禁。 |
| R1-023 | `push_git → push_with_upstream`。[S17][S18][S19] | 有upstream时先 fetch --prune 指定remote，然后 push remote HEAD:branch；无upstream时普通push。 | **前置fetch错误被忽略后仍push**；网络/凭据及子进程边界待查；不是read-only状态操作。 |
| R1-024 | `pull_git → pull_with_default_strategy`。[S17][S18][S19] | 首先 pull --autostash；根据错误文本分别尝试 plain pull、pull --no-rebase、pull --no-rebase --autostash。 | 必须记录每条fallback，不能把它们合并为一个失败结果；原生状态/冲突/自动stash还需动态证据，未见统一事务回滚。 |
| R1-025 | `fetch_git → fetch_with_default_remote`。[S17][S18][S19] | 依据upstream选择remote，执行fetch --prune。 | 读取网络并更新本地引用，不是“纯远程只读”；无显式超时/取消证据的wrapper不能标成已有受控生命周期。 |
| R1-026 | `sync_git → sync_git_inner`。[S17][S18][S19] | pull 成功才push；两个步骤复用各自分支。 | pull已改变工作区但push失败时返回错误，不自动回滚pull，不能直接重放整条sync。 |
| R1-027 | `useGitActions.initGitRepo → init_git_repo → init_git_repo_inner`。[S20][S17][S18][S19] | 已有repo返回already；非空目录且非force返回needs_confirmation。前端ask及workspace再校验后force重提；init --initial-branch不支持时fallback init+symbolic-ref；随后add-A、allow-empty初始commit。 | add/commit失败可返回 `initialized + commitError`，前端报告“已初始化但提交失败”。不能把包装层Ok视为全部成功，也不能为凑等价删确认流程。 |
| R1-028 | `useGitActions.createGitHubRepo → create_github_repo → create_github_repo_inner`。[S20][S17][S18][S19] | 校验visibility/localrepo/origin；必要时gh api user确定owner；view/create远端；缺origin则选择ssh/https并remote add；push -u origin HEAD；PATCH默认分支。 | **push发生在显式branch验证之前；push错误不阻止后续默认分支PATCH尝试。** 部分失败返回`partial`及具体字段；前端把它显示成失败，不冒称完成。无统一远端/本地撤销。 |
| R1-029 | `checkout_github_pull_request_inner`，core转发及lib注册已读。[S18][S22][S10] | `gh pr checkout <number>`，cwd为workspace repo。 | 不是只读PR预览；更改本地checkout且可联网。git adapter后半段及前端最终按钮仍待补读，不能称全调用者闭合。 |
| R1-030 | `get_github_issues → core → get_github_issues_inner`。[S17][S18][S22] | 取origin或首remote映射repo；gh issue list limit50，再gh api search请求total。 | 第二次查询/解析失败会以已取列表长度代替total。该数字不保证真实总量，列表失败则返回错误。 |
| R1-031 | `get_github_pull_requests → core → ...inner`。[S17][S18][S22] | gh pr list open limit50，含body/作者/分支；再查search总量。 | total错误同样fallback列表长度；不能因此宣称所有PR已读取。 |
| R1-032 | `get_github_pull_request_diff → core → ...inner`。[S17][S18][S22] | gh pr diff --repo --color never，然后本地解析路径/状态。 | 原生非零返回错误；quoted/rename等路径解析及下游展示/执行权限不是本批通过项目。 |
| R1-033 | `get_github_pull_request_comments_inner`，core转发已读。[S18][S22] | 请求 `/issues/<pr>/comments?per_page=30` 并jq投影。 | 只看到该endpoint和这一页，**不能称为全部review/inline comments**。adapter尾部及UI触发仍保留frontier。 |
| R1-034 | `list_git_branches_core → list_git_branches_inner`。[S18][S19] | 用libgit2遍历本地分支，按最后commit时间排序。 | **没有shell Command也有真实库调用**；归入读取，不因为命令扫描没命中就缺失。 |
| R1-035 | `checkout_git_branch_core → checkout_git_branch_inner`。[S18][S19] | libgit2打开repo，再调用 `git_utils::checkout_branch`。 | helper的具体checkout策略未读，不能以wrapper完成断言没有写文件/没有force；明确可读后续文件。 |
| R1-036 | `create_git_branch_core → create_git_branch_inner`。[S18][S19] | libgit2取HEAD commit、创建分支，再checkout helper。 | 创建分支后checkout失败可能保留新分支；不是一个shell sink，也不是单一原子动作。 |
| R1-055 | `git/mod.rs::call_remote_if_enabled` 与 `try_remote_*` 宏。[S17] | remote模式时call_remote；得到Some就返回；local模式才None并进入本地core。 | **远程RPC失败经 `?` 传播，不是失败后静默本地执行。** 必须解析宏再谈调用闭包；remote实现与daemon各分支仍未在本批复核。 |

补充已读 helper：`git_core.rs` 中 local branch/remote存在性的`.status().success()`会把非零合并成false；live remote `ls-remote --heads` 是网络调用，非零返回错误；`run_git_diff`将0或1作为合法结果。它们的全部反向caller未统计，不能把文件中的7个构造点等同7个业务入口。[S23]

### 3.4 更新、窗口生命周期及独立前端网络

覆盖 C01/C35/C37/C39/C41，关联 WP05/WP09b/WP10a/WP10b/WP12。**更新产品为 gogoke，发布仓库是 `taiyun668/gogoke`，不是本施工仓库 `gogo-party`。**

| ID | 实际根及已读链 | 条件、实际效果 | 失败/清理与剩余边界 |
|---|---|---|---|
| R1-037 | MainApp mount effect → `signalGogokeUpdateReady`。[S03][S09] | `hasNativeBackendTransport()` 为真就调用readiness IPC。 | 错误warn；此effect并不等待其它产品模块全部初始化。Rust receipt写入/校验链仅继承旧账本索引，本批未重新读完，不提升证据等级。 |
| R1-038 | `useUpdater` 启动自动检查effect。[S13][S14] | enabled、非DEV、isTauri且本hook只试一次；先取上次failure，有failure显示rollback错误并return，否则按autoCheckOnMount调check。 | failure读取异常显示error；自动检查与下列release-notes是独立effect，不能用一个开关结论覆盖全部网络。 |
| R1-039 | updater menu event → `useUpdaterController` → `checkForUpdates`。[S13][S14] | 菜单事件订阅在enabled时触发，允许announceNoUpdate；空offer显示latest或idle。 | 菜单原生发送端/event hub仍需追；这里只证明消费者。check服务失败进入error/debug，未自动安装。 |
| R1-040 | `useUpdater.startUpdate`。[S14][S09][S30] | enabled；没有offer只check后return；有offer**先localStorage保存pending版本，再调用install IPC**。 | install失败留error，所读catch没有删除pending标记；外部安装动作须单独授权，不能在摸排中触发。 |
| R1-041 | pending版本effect → `fetchReleaseNotesForVersion`。[S14][S30] | enabled、isTauri、pending规范化后等于当前版本；没有autoCheckOnMount或DEV限制。前端直接fetch release tags，先v版本，404再试无v版本。 | **关闭自动更新检查不等于关闭这条网络。** generation/cancelled只抑制UI回写，没有传AbortSignal中止请求；非404错误终止。payload.html_url非空即保留，真实渲染/opener端仍需检查。 |
| R1-042 | `dismissPostUpdateNotice`、版本不匹配清理与storage helper。[S14][S30] | dismiss提升generation、删pending key、清notice；版本不匹配也删key。key为 `gogo-party.pendingPostUpdateVersion`。 | localStorage读/写/删均best effort；返回空不能证明此前无状态。marker是迁移对象，不是签名信任根。 |
| R1-052 | Rust `lib.rs.setup` 自动daemon分支。[S10] | desktop+TCP：remote模式直接start；**local模式也会在daemon status为Running时调用start**以检查/处理版本。 | start/status错误多处被忽略；不能认为切local模式就没有daemon启动/重启链。Tailscale下层start/stop与系统身份仍是待核边界。 |
| R1-053 | `RunEvent::ExitRequested → stop_managed_daemons_for_exit`。[S10] | 未在cleanup且不keep-daemon时prevent_exit，设置全局标志，异步调用tailscale stop，再app.exit(0)。macOS CloseRequested另走hide，不等于退出。 | stop结果被忽略；此处不能证明所有本地CLI/PTY/工具子孙均停止。保留多种关闭语义，禁止因去Codex删掉宿主生命周期能力。 |

### 3.5 构建、自动脚本、签名与发布

覆盖 C01/C32/C33/C38/C39/C40；后续归 WP03/WP08/WP12。这里读取的是源码，不执行这些动作。工具入口同样是P00范围，不能统统归成“非生产，无副作用”。

| ID | 实际根及已读链 | 条件、实际效果 | 失败/清理与剩余边界 |
|---|---|---|---|
| R1-043 | package生命周期 `postinstall/predev/prebuild/pretauri:* → sync:material-icons → sync-material-icons.mjs`，也可显式调用。[S24][S25] | 从node_modules的generated/icons复制到public/assets/material-icons；source存在时先mkdir父目录、**recursive rm目标，再recursive cp**。 | source缺失warn并exit0；删除后复制失败没有原子回滚。故npm ci/dev/build并非只读摸排命令，生命周期边须独立登记。 |
| R1-044 | `doctor:win → doctor.mjs --strict`。[S24][S26] | stat/access PATH、Windows PATHEXT探测cmake/clang；打印安装建议；strict缺项exit1。 | **没有执行choco/brew/apt、没有下载或spawn。** 不能将建议文字当安装sink。doctor.sh未读，不能类推。 |
| R1-045 | `prepare:gogoke-icons`，及Windows build的pre脚本。[S24] | 显式命令为tauri icon，输入PNG，输出icons/generated。 | 外部tauri CLI实现和锁定包身份未在本批核；分类为构建时文件生成边界，不伪造本地已执行证据。 |
| R1-046 | `.github/workflows/gogoke-desktop.yml`。[S27] | push限定gogoke-shell及paths，PR按paths，另有dispatch。web任务npm ci/test/build；Windows任务安装cmake/llvm、写环境、清缓存exe/bundle、cargo定向test/build、打包。local-only exe **先于smoke上传**，smoke通过才上传smoke-verified包。 | **b0分支仅docs/census工具的push不满足这个workflow的push条件。** 不是“仓库没有CI”。该workflow未调用公开Release发布；local-only与smoke-verified产物不能混称。外部Actions/CLI行为仍待锁版本核验。 |
| R1-047 | `publish-gogoke-release.ps1` **不带 `-Publish`**。[S28][S29] | 检查installer/portable/manifest版本hash和unsigned身份；临时解包并限制3个文件；finally删inspect目录；随后仍调用签名helper并验证sig。 | **仍读私钥并写manifest/sig，不是只读dry-run。** 未授权不得运行；临时路径清理并不抵消其它写入。 |
| R1-048 | 同脚本带 `-Publish`。[S28] | 候选检查/签名成功后要求NotesFile；gh release view版本；不存在时gh release create到taiyun668/gogoke。 | 当前view任意非零都走create尝试，不区分不存在/鉴权/网络错误；create非零throw。不能用这个分支推断远程Release尚不存在。 |
| R1-049 | `sign-gogoke-release-manifest.ps1` 常规签名。[S29] | 读指定私钥3行D/X/Y；校验manifest及版本；缺version行会先重写manifest；随后写`.sig`。 | 多version或不匹配拒绝；属于secret读+artifact写。不能把发布公钥/允许主机当可从用户数据迁入的普通配置。 |
| R1-050 | 签名脚本 `-NewKey`。[S29] | 私钥已存在则拒绝；否则建目录、生成P256、写默认USERPROFILE下.gogoke私钥，并写repo中的public key。 | 所读NewKey分支只有私钥存在拒绝，没有等价的public-key已存在拒绝。不是本任务授权动作，不擅自执行或改许可证/信任根。 |
| R1-051 | `smoke-gogoke-update.ps1`。[S31] | 在临时目录启动PowerShell短父进程和真实coordinator/installer；成功后按exe路径找进程并强停；写rollback marker；故意错版本再跑coordinator；finally实际运行uninstaller/S并清临时目录。 | **这是安装、进程和卸载实测，不是无副作用的仪器自测。** uninstaller非零仅记stderr；StopTargetProcess用PID，未见创建时间核对。协调器/NSIS的详细系统写入仍待重新展开，本批没有执行。 |

## 4. 已形成的具体摸排结论，而非自动批准的修复

### 4.1 入口判据需要扩充，但不必另造一套大仪器

本批有可复核的几种漏项形状：

1. **无点击自动执行**：恢复workspace、启动daemon、pending release-notes、npm pre/post钩子。
2. **一个功能名后有多个副作用**：create GitHub repo含remote修改、push、default branch PATCH；worktree创建含磁盘、规则、模型连接、配置写入。
3. **非Command构造点**：portable_pty、libgit2、前端fetch、localStorage、PowerShell内嵌命令。
4. **成功与事实不等价**：terminal-exit不等于process exit；mark-ran不等于脚本成功；Ok(partial)不等于全部成功；无Publish不等于只读。
5. **静态结构误截断**：cfg(test)只修饰其项，不是“以下整文件全部为测试”。
6. **错误路径可继续执行或已有影响**：restore失败转clean、pull换参数重试、push错误后仍PATCH、setup restart失败后继续写入。

这些是已有代码的具体来源事实，不能直接推出已遭利用、安全边界已被攻破或必须删掉相应能力。修复应进入相应WP并保持原有工具能力，本批不改生产实现。

### 4.2 本批对现有 CF 的增量，不是宣布关闭

| 旧 frontier | 本批实际推进 | 仍不能关闭的原因 |
|---|---|---|
| CF01 全文件/平台来源闭包 | 找到cfg(test)截断反例；识别配置/生命周期边与Command以外的来源。 | 未逐一读完原manifest成员、全部条件编译/动态include/包exports/最终配置。没有完整邻接图。 |
| CF02 Git/gh/PTY/kill | 展开commands的具体操作、GitHub查询/checkout、PTY所有本地命令、脚本输入、worktree部分core和错误补偿。 | Git diff/log/context/git_utils、git_core尾部与所有caller、worktree rename尾部、daemon/remote对应分支、native依赖仍未全覆盖。 |
| CF03 前端来源及别名 | 直接追到restore、manual launch、setup、terminal DOM事件/ResizeObserver、updater菜单消费者与独立fetch。 | MainApp和Git adapter仅指定窗口；所有UI叶、快捷键、事件发送端、tabs、多脚本、资源/媒体/clipboard等剩余文件未逐个核。 |
| CF04 构建发布 | 展开package隐式钩子、icon sync、Windows doctor、desktop workflow、publish/sign/smoke全部所读脚本。 | doctor.sh、codemods、iOS脚本、其它workflow、Tauri base/unsigned/config+Cargo+build.rs及coordinator/NSIS传递链未完成本批复核；第三方构建工具需锁身份。 |
| CF05 全部效果反扫 | 在已读链中登记进程/输入/事件/存储/文件/网络/密钥/发布/退出；记录partial和清理失败。 | dictation权限/录音/download、file/registry所有成员、远程鉴权、原生插件和环境/OS边界尚未全量反向归属。全仓orphan数仍null。 |

## 5. 尚未读完的源码是待做工作，不是外部 UNKNOWN

以下是明确可继续读取的前沿，**不得将它们变成Owner待裁决的问题，也不得以“接口边界”名义直接关闭**：

| Frontier | 紧接本批的读取对象 | 要补的证据 |
|---|---|---|
| RF01 | `src-tauri/src/git/mod.rs` 530行之后；`shared/git_ui_core/{diff,log,context}.rs`；`git_utils.rs` | 其余adapter、libgit2 checkout、diff子进程的kill/error、repo根与路径授权、全部反向caller。 |
| RF02 | `src-tauri/src/workspaces/commands.rs`、`shared/workspaces_core/{connect,crud_persistence,io,runtime_codex_args}.rs`、worktree.rs 370行后 | 注入函数实参、rename/copy/cleanup、模型连接重启、setup status/mark桥及恢复来源。 |
| RF03 | `useTerminalTabs`、`useWorkspaceLaunchScripts`、`useMainAppModals`、MainApp未读窗口、布局按钮叶 | 多脚本与保存/执行分离、close callbacks、UI/菜单/热键完整入度。 |
| RF04 | `useWorkspaceRefreshOnFocus`、`useRemoteThreadRefreshOnFocus`、事件发送端与services/events | focus、interval、reconnect、订阅和取消造成的自动执行与迟到事件链。 |
| RF05 | release-notice展示和opener、全部clipboard/audio/media/storage相关源码 | 外部body/html_url实际使用、内容安全、浏览器/native分支，不能只依据返回类型断言安全。 |
| RF06 | `doctor.sh`、codemods、iOS脚本、base/unsigned Tauri配置、Cargo/build.rs、其它workflows | 真实工作目录、隐式脚本、生成产物、平台门和传递引用；配置被分类并不等于内容已分析。 |
| RF07 | `update/gogoke-update-coordinator.ps1`、NSIS定义、gogoke_update.rs完整函数 | 安装/ready/回滚/卸载/registry所有分支、失败后留下的数据及信任校验。旧账本摘要不提升成本批实读。 |
| RF08 | `dictation`、`files`、`tailscale`、`remote_backend`、daemon transport/rpc及锁定插件依赖 | 全部权限/网络/文件/线程/进程效果、auth到业务分派、停止和清理；暂缓政策是否真的在后端生效。 |

对于RF01–RF08，目前不能填写“只有运行时才知道”：它们首先还有可读源码未展开。外部CLI/OS/plugin内部则另列真实boundary，登记lock身份、入口、owner和后续验证阶段。**没有把本机账号、真实CLI或安装包实测当成本次源码摸排的替代。**

## 6. 后续复核如何使用本批产物

1. 使用固定bc665源码逐条抽查表内路径、调用和条件，尤其错误/清理分支。若发现不符，修正该行，不用整体口头评价覆盖证据。
2. 原E分组仍可作导航，但本批R1成员不能只折成一句“Git已核”。新发现caller继续追加稳定成员，不以计数凑齐原129/104作为完成。
3. P00闭合要最终同时具备：根集合、传递来源、逐效果sink、正反向引用、平台条件、外部boundary、仍未知的枚举和复核证据。每个root到效果、每个效果到root都要可解释。
4. 本批没有改原Stage1 census。新增报告会改变后续完整工作树manifest；旧census仍按其原绑定解读，不得宣称新增文件后原稳定生成检查仍然通过。重新生成须同时核定source basis与当前候选，不能只改hash使检查变绿。
5. 只读审计/阶段收口与生产修复分开。由Controller组织独立fresh复核；P00未验收前不能把本报告当成P01已准入。本批也不启动或停止Owner本地正在运行的Codex进程。

## 7. 源码证据索引

所有源均固定在 `bc665a852833952b76d9508401193bedd2198436`。以下“全文”表示读取请求覆盖至文件末尾；“窗口”表示只据已显示内容得出结论，未显示尾部不作已读声明。固定提交已唯一确定blob，表中再给blob前缀方便交叉检查。

| ID | 文件 | 本批读取范围 | blob前缀 |
|---|---|---|---|
| S01 | `tools/gogoke-p00-census/census.py` | 1–388全文 | b3a84b1623ec |
| S02 | `docs/design/gogoke-codex-decoupling-p00-entry-inventory-v3.md` | 多窗口；280–309 frontier完整 | bbd168a41b8e |
| S03 | `apps/desktop/src/features/app/components/MainApp.tsx` | 1–180、360–640、780–1050 | cd0f8022e988 |
| S04 | `apps/desktop/src/features/app/hooks/useMainAppWorkspaceLifecycle.ts` | 全文 | b0318433561b |
| S05 | `apps/desktop/src/features/workspaces/hooks/useWorkspaceRestore.ts` | 全文 | 9c86f9e5469f |
| S06 | `apps/desktop/src/features/app/hooks/useWorkspaceLaunchScript.ts` | 全文 | 9ed7bcb39e3c |
| S07 | `apps/desktop/src/features/app/hooks/useWorktreeSetupScript.ts` | 全文 | c65a4fe23742 |
| S08 | `apps/desktop/src-tauri/src/terminal.rs` | 1–430覆盖全文 | 4f7cc4395b69 |
| S09 | `apps/desktop/src/services/tauri.ts` | 930–1145窗口 | 752c806abab8 |
| S10 | `apps/desktop/src-tauri/src/lib.rs` | 1–375覆盖全文 | f7b2a5d65d1a |
| S11 | `apps/desktop/src/features/terminal/hooks/useTerminalController.ts` | 1–245覆盖全文 | 33d915cacc73 |
| S12 | `apps/desktop/src/features/terminal/hooks/useTerminalSession.ts` | 1–440覆盖全文 | ef68be8304cf |
| S13 | `apps/desktop/src/features/app/hooks/useUpdaterController.ts` | 全文 | ae25a35ea0f5 |
| S14 | `apps/desktop/src/features/update/hooks/useUpdater.ts` | 1–330覆盖全文 | 52dbacbe8bbd |
| S15 | `apps/desktop/src-tauri/src/shared/workspaces_core/worktree.rs` | 1–370窗口 | 824e4aecdadb |
| S16 | `apps/desktop/src-tauri/src/shared/workspaces_core/helpers.rs` | 1–170窗口 | d2579af84e9e |
| S17 | `apps/desktop/src-tauri/src/git/mod.rs` | 1–530窗口 | b4321a5d5c53 |
| S18 | `apps/desktop/src-tauri/src/shared/git_ui_core.rs` | 1–300覆盖全文 | 2697484de1f2 |
| S19 | `apps/desktop/src-tauri/src/shared/git_ui_core/commands.rs` | 首段至截断处；375–810显式窗口；未声明测试尾全文 | 90400d74bc64 |
| S20 | `apps/desktop/src/features/git/hooks/useGitActions.ts` | 1–410覆盖全文 | 63863dcd70f9 |
| S21 | `apps/desktop/src-tauri/src/shared/workspaces_core/git_orchestration.rs` | 1–270覆盖全文 | 6ee25e29bc72 |
| S22 | `apps/desktop/src-tauri/src/shared/git_ui_core/github.rs` | 1–405覆盖全文 | a3ba384b0c6a |
| S23 | `apps/desktop/src-tauri/src/shared/git_core.rs` | 1–240窗口 | 682e116dea2b |
| S24 | `apps/desktop/package.json` | 全文 | 427175a53cc7 |
| S25 | `apps/desktop/scripts/sync-material-icons.mjs` | 全文 | d215bb138973 |
| S26 | `apps/desktop/scripts/doctor.mjs` | 全文 | e351a1105a9d |
| S27 | `.github/workflows/gogoke-desktop.yml` | 1–260覆盖全文 | 06dcc343f112 |
| S28 | `tools/publish-gogoke-release.ps1` | 1–260覆盖全文 | b8ad887da919 |
| S29 | `tools/sign-gogoke-release-manifest.ps1` | 1–240覆盖全文 | b00c834b14aa |
| S30 | `apps/desktop/src/features/update/utils/postUpdateRelease.ts` | 全文 | dde26f55bc17 |
| S31 | `tools/smoke-gogoke-update.ps1` | 1–220覆盖全文 | 7c20733701c5 |
| S32 | `apps/desktop/src-tauri/tauri.windows.conf.json` | 全文；不等于最终合并配置 | 8e53dc8d7b1e |
| S33 | `apps/desktop/src/features/app/hooks/useMainAppGitState.ts` | 1–210窗口 | b0088dca9676 |
| S34 | `apps/desktop/src-tauri/src/shared/workspaces_core.rs` | 全文 | 90eb5f91170c |

[S01]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/tools/gogoke-p00-census/census.py
[S02]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/docs/design/gogoke-codex-decoupling-p00-entry-inventory-v3.md#L280-L309
[S03]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/app/components/MainApp.tsx
[S04]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/app/hooks/useMainAppWorkspaceLifecycle.ts
[S05]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/workspaces/hooks/useWorkspaceRestore.ts
[S06]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/app/hooks/useWorkspaceLaunchScript.ts
[S07]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/app/hooks/useWorktreeSetupScript.ts
[S08]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/terminal.rs
[S09]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/services/tauri.ts#L930-L1145
[S10]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/lib.rs
[S11]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/terminal/hooks/useTerminalController.ts
[S12]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/terminal/hooks/useTerminalSession.ts
[S13]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/app/hooks/useUpdaterController.ts
[S14]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/update/hooks/useUpdater.ts
[S15]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/workspaces_core/worktree.rs#L1-L370
[S16]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/workspaces_core/helpers.rs#L1-L170
[S17]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/git/mod.rs#L1-L530
[S18]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/git_ui_core.rs
[S19]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/git_ui_core/commands.rs
[S20]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/git/hooks/useGitActions.ts
[S21]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/workspaces_core/git_orchestration.rs
[S22]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/git_ui_core/github.rs
[S23]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/git_core.rs#L1-L240
[S24]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/package.json
[S25]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/scripts/sync-material-icons.mjs
[S26]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/scripts/doctor.mjs
[S27]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/.github/workflows/gogoke-desktop.yml
[S28]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/tools/publish-gogoke-release.ps1
[S29]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/tools/sign-gogoke-release-manifest.ps1
[S30]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/update/utils/postUpdateRelease.ts
[S31]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/tools/smoke-gogoke-update.ps1
[S32]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/tauri.windows.conf.json
[S33]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src/features/app/hooks/useMainAppGitState.ts#L1-L210
[S34]: https://github.com/taiyun668/gogo-party/blob/bc665a852833952b76d9508401193bedd2198436/apps/desktop/src-tauri/src/shared/workspaces_core.rs

**交接状态：已有逐成员源码证据增量；没有签发全仓闭合或P00完成；没有授权生产修复或执行脚本。**
