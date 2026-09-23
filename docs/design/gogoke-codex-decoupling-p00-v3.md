# gogoke Codex 去专属化 P00 V3：基线、范围与 P01 设计输入

> `artifact_id: GKD-P00-V3`
> `plan_version: 3.0`
> `baseline_commit: 23fb8098f34d7becd41c3e4ce19d44e5a1f9994a`
> `repair_base_candidate: bedc8ff7cb18f7ff2f188123950f25e27621b3c9`
> `evidence_date: 2026-09-18`
> `state: planning_input_pending_independent_review`
> `acceptance: NOT_ACCEPTED`
> `source_closure: INCOMPLETE`

本文是 WP00/P00 的版本化施工输入，也是 WP01/P01 的设计输入。它不是已冻结的公共合同、实现完成声明、测试通过报告或发布许可。需要改变目标、契约、安全边界或验收规则的事项均保留为 `UNKNOWN`/`BLOCKED`，不得由后续施工者自行补成决定。

## 1. 固定基线与证据边界

| 项 | 固定值／结论 |
|---|---|
| 施工基线 | `23fb8098f34d7becd41c3e4ce19d44e5a1f9994a` |
| P00 repair 候选基线 | `bedc8ff7cb18f7ff2f188123950f25e27621b3c9` |
| 工作树 | `%USERPROFILE%\.codex\worktrees\gogoke-v3-b0\GOGO PATTY` |
| 分支／初始状态 | `codex/gogoke-decoupling-v3-b0`；B0-04 写入前 clean |
| 启动入口摘要 | `START_HERE_CODEX_DECOUPLING_V3.md` = `C924505772C334B8C6AE9CBF072241BE0E7C3D400DA942D2EB41397351F0F156` |
| V3 主计划摘要 | `docs/design/gogoke-codex-decoupling-plan-v3.md` = `9F10891D764305D0FDAD56B7BB34B54ABF1503AA882918FABA7A1B26B67A4C9D` |
| production entry/root inventory（source closure未完成） | `docs/design/gogoke-codex-decoupling-p00-entry-inventory-v3.md` = `290159D7C16E75C1752741C33CEE3373FE09836DAD6375077ED149110FD82539` |
| 已读规则 | 根 `AGENTS.md`、`apps/desktop/AGENTS.md`、`docs/model-routing.md` |
| B0 输入 | B0-01、B0-02、B0-03 的 agent Result；三者没有提供可哈希的落盘 artifact，因此只作为主张索引，本文另行核对当前源码、Git 与只读 Provider 状态 |
| 实际执行 | 只读源码/Git/Provider 状态检查，以及写入本文；无产品运行、构建、测试、生产数据访问、服务控制、登录/注销、probe/plan/run 或真实模型请求 |

### 1.1 权威顺序

冲突按以下顺序收口：

1. Owner 当前明确指令与最终裁决；
2. `START_HERE_CODEX_DECOUPLING_V3.md` 中关于如何启动 V3 的说明；
3. `docs/design/gogoke-codex-decoupling-plan-v3.md`；
4. 当前 `AGENTS.md`、`apps/desktop/AGENTS.md`、`docs/model-routing.md` 的运行与团队边界；
5. V1、V2、Claude 审计及旧 conformance 结果仅作历史和证据。

启动说明与 V3 的技术内容冲突时以 V3 为准；任何文档与实际 runtime 权限冲突时以 runtime 强制边界为准并升级。V1、V2 和 Claude 审计不作为本专项的并行施工计划。

### 1.2 证据等级

| 标记 | 含义 |
|---|---|
| `SOURCE` | 当前固定 commit 的源码/配置/许可文本可直接观察 |
| `TEST_EXISTS` | 测试或 fixture 静态存在，但本批次未执行 |
| `RUNTIME_READONLY` | 只读状态命令直接返回当前状态，不等于功能能力已验证 |
| `UNEXECUTED` | 未构建、未运行、未做原生/生产验证 |
| `UNKNOWN` | 证据不足，不能从声明、fixture 或零请求预检推定 |
| `BLOCKED` | 到指定阶段前必须取得决定或证据，否则不得继续依赖动作 |

## 2. 当前入口链、数据链与缺口

### 2.1 实际入口链摘要与完整 companion

production entry、startup/platform side effects、writer、process与root台账固定在companion `docs/design/gogoke-codex-decoupling-p00-entry-inventory-v3.md`，SHA-256=`290159D7C16E75C1752741C33CEE3373FE09836DAD6375077ED149110FD82539`。既有机械集合仍是129 registry/129 frontend/104 business（101 shared、28 Tauri-only、3 daemon-only）；transport-only auth使wire union为105。既有43/22 Web Storage、12 clipboard、plugin/audio/platform与2 sips+1defaults仍独立成立。**companion §9 source closure未完成：777-file tracked引用范围、wrapper反扫candidate已固定，CF01-CF05仍未逐成员展开，孤立sink数NOT_COMPUTED，不能把数目相等或C/D全覆盖当完整性证明。**下表仍只是摘要，不改变全量production census合同。

| 链 | 当前固定源码链 | 等级 | 直接缺口 |
|---|---|---|---|
| 桌面启动 | `App.tsx` → `MainApp.tsx` → `useAppBootstrapOrchestration`（重导出 `useAppBootstrap`）→ settings/dictation/debug/transparency hooks | `SOURCE` | 启动仍含大量 Codex 命名/编排；无无-Codex runtime 证据 |
| 工作区读取 | `useWorkspaces`/`useWorkspaceCrud.refreshWorkspaces` → `services/tauri.listWorkspaces` → Tauri command → `shared::workspaces_core` → `workspaces.json` | `SOURCE`, `TEST_EXISTS` | 数据损坏与真实 app-data 目录未运行核验 |
| 工作区连接 | workspace action → `services/tauri.connectWorkspace` → `workspaces::connect_workspace` → `workspaces_core::connect_workspace_core` → `spawn_workspace_session` → `codex app-server`，cwd=`WorkspaceEntry.path` | `SOURCE`, `TEST_EXISTS` | 这是 Codex 专属生产链；共享/复用条件和真实 CLI 身份未核验 |
| Session/Thread 启动 | UI thread action → `services/tauri.startThread` → Tauri `start_thread` → `codex_core::start_thread_core` → JSON-RPC `thread/start` | `SOURCE`, `TEST_EXISTS` | 裸 `workspaceId/threadId` 仍承担产品身份，尚无公共 Session/Binding |
| 消息发送 | composer/queued-send/thread messaging → `services/tauri.sendUserMessage` → Tauri `send_user_message` → `codex_core::send_user_message_core` → JSON-RPC `turn/start` | `SOURCE`, `TEST_EXISTS` | `full-access` 可映射 `dangerFullAccess`；缺公共 Execution/Delivery 权威链 |
| 插话/中断 | thread messaging → `turn_steer`/`turn_interrupt` → Tauri command → shared core → `turn/steer`/`turn/interrupt` | `SOURCE`, `TEST_EXISTS` | 原生行为、接受证据与公共状态语义未分开 |
| 事件 | app-server stdout reader → `EventSink` → Tauri `app-server-event` → `services/events.ts` hub → `useAppServerEvents`/thread reducers | `SOURCE`, `TEST_EXISTS` | 原生 frame、subagent、乱序/旧 generation 尚未归一到 PublicEvent |
| daemon | daemon JSON-RPC → `rpc/dispatcher.rs` → `rpc/workspace.rs`/`rpc/codex.rs` → 同一 shared core/session backend；事件以 `app-server-event`/terminal notifications 返回 | `SOURCE`, `TEST_EXISTS` | source parity 不等于运行 parity；旧 remote/daemon 入口是否双写未核验 |
| 原生/磁盘 | `AppState::load()` → platform `app_data_dir()` → `workspaces.json`/`settings.json`；失败会回落 cwd；Codex Home 为 `CODEX_HOME` 或用户 Home 下 `.codex` | `SOURCE` | 回落 cwd 与全局 Codex Home 均与 V3 公共 root/单一 owner 目标冲突，必须在 WP02/WP03 修复后验证 |

### 2.2 当前数据与路径台账

| 数据/状态 | 当前来源与语义 | P01/P02 输入 |
|---|---|---|
| gogoke workspaces/settings | Tauri `app_data_dir()/workspaces.json`、`settings.json`；读取失败当前可回默认值 | 保留格式但禁止新链静默回 cwd 或坏数据覆盖为空；实际 packaged Windows 路径待核 |
| 运行中 workspace/session | `AppState.workspaces`、`sessions`、`terminal_sessions`、`app_settings` | 内存 map 不是耐久 Session/Delivery 权威；需要 rootId/schema/单写者语义 |
| workspace root | `WorkspaceEntry.path`，并作为 app-server cwd/默认 writable root | 路径别名、大小写、junction、网络根和授权根需显式规范化与锁验证 |
| Codex native home | `resolve_workspace_codex_home` 当前忽略 workspace/parent，统一返回默认 `CODEX_HOME` 或 `$HOME/.codex` | 原生 Home 只能成为明确 NativeBinding/driver 扩展，不能成为公共数据根 |
| UI thread metadata | localStorage 的 `codexmonitor.threadLastUserActivity`、`pinnedThreads`、`threadCustomNames`、`threadCodexParams`、`detachedReviewLinks`；best-effort UI state | 迁移须按公共 Session 映射并保留来源；不能把 localStorage 当服务器权威 |
| workspace-local skills | `.agents/skills` 从 workspace path 发现 | 发现、展示、启用与执行必须分开；工作区文字不能扩大授权 |
| daemon data | 显式 `data-dir` 下的同名 settings/workspaces；默认监听记录存在 | app 与 daemon 必须解析到同一核验 root/owner；remote 暂关不等于本地 daemon 删除 |

### 2.3 缺口账本

| Gap | 状态 | 所有者 | 最迟阻塞阶段 |
|---|---|---|---|
| G00-01 packaged Windows 实际 app-data 路径与 fallback 行为未运行核验 | `UNKNOWN` | WP02 / Sol-high | WP02 完成前 |
| G00-02 一个 app-server 可登记多个 workspace；其复用边界是否等于产品实例未定 | `BLOCKED` | WP03 / Sol-high；疑难交 Astra-high | P03 合同前 |
| G00-03 global `CODEX_HOME` 与公共 root/NativeBinding 的所有权关系未定 | `BLOCKED` | WP02+WP03 / Sol-high | P02/P03 合并前 |
| G00-04 native history、gogoke public content 与 localStorage metadata 的权威关系未定 | `BLOCKED` | WP02+WP06 | P02 schema 前 |
| G00-05 subagent/background/plan 事件如何映射 PublicEvent/公共活动未定 | `BLOCKED` | WP01+WP04+WP06 / Sol-high | P01 schema 冻结前 |
| G00-06 daemon/app runtime parity 与旧入口双发风险未运行核验 | `UNKNOWN` | WP10b / fresh Sol；Astra 风险轴 | P10b/WP11b |
| G00-07 当前 `full-access` 到 `dangerFullAccess` 的授权、审计与 UI 来源未核验 | `BLOCKED` | WP04 / Sol-high+Astra | P04 前 |
| G00-08 五家 binary/version/profile/mode 与实际 native capabilities 均未固定 | `UNKNOWN` | WP09a / Sol-high | 各家 N1 准入前 |
| G00-09 V3 正文提及 companion execution JSON/validator；当前 tracked tree 与内容检索均未找到 V3 companion JSON 或专用脚本 | `GAP` | WP01 / Sol-high 定 schema，Luna-max 实现 | T61.L/T68.D；不得伪造为 WP00 已有 |
| G00-10 B0 禁止生产数据访问，因此没有制作旧生产数据副本或行为样本 | `BLOCKED` | Controller 授权后由 WP02 使用隔离副本 | P02 迁移夹具前 |
| G00-12 CodexMonitor 派生文件的逐项版权/分发义务尚未闭合 | `BLOCKED`（仅相关采用/分发） | Owner + Astra/必要专业审核 | P12 分发候选前 |
| G00-13 完整 inventory 证明当前 remote/Tailscale auto-start/restart 与 dictation permission/download/start 入口仍 source-active，尚不能称 `preserved_disabled` | `BLOCKED` | WP10a / Sol-high；Astra 权限轴 | T56.L/T57.L |
| G00-14 standalone daemon 在 HOME 缺失时从 cwd 构造 `.local/share/gogoke_daemon`，Windows daemonctl 在 APPDATA/USERPROFILE 均缺失时从 cwd 构造相对 app-data root；两者违反 V3 禁止 fallback cwd | `BLOCKED`（源码存在，未修复） | WP02 固定 root/lock；WP10b 固定 managed launch/self-report | P02/P10b acceptance；T39,T41,T43,T55,T63 |
| G00-15 Web Storage 并非 frontend writer 全集：clipboard 可直接写 transcript/debug/message/path/Git/script，opener/notification/dialog 直接触达 OS/plugin surface | `GAP`（已入 inventory，runtime/privacy 未验） | WP05/WP06/WP08/WP09b | T11,T22,T23,T28,T35,T49,T54,T66 |
| G00-16 macOS HEIF 与 app-icon conversion 会在独立 OS temp namespace 写文件；两条链共启动2个`sips` call sites，icon链还先启动1个`defaults read` call site；cleanup 为 best-effort，HEIF read-error 可提前返回，残留/碰撞/process identity/打包未验 | `BLOCKED` | WP03 subprocess/lifecycle；WP08 attachment/path；WP05 icon UI | 对应 L/I 前；T21,T22,T35,T49,T50,T52,T54,T60 |
| G00-17 Linux startup 最多写3个进程级渲染环境变量，后续原生/terminal/daemon等 spawn 可能继承；当前未捕获每条 final env | `BLOCKED` | WP03 env/launch owner；WP05视觉协同 | T50.L@WP03，T12/T50.I@WP11b |
| G00-18 macOS close/reopen/tray与Windows decorations/menu均为 non-command lifecycle mutation；窗口长期 Session、focus、退出/重连与实机视觉未验 | `GAP` | WP05 shell/visual；WP10b lifecycle；WP12 packaging | T09,T12,T44,T46 |
| G00-19 browser AudioContext 会 fetch/decode并输出到OS audio destination；权限/autoplay/device、活动时序隐私与失败语义未运行验证 | `GAP` | WP05；隐私/content轴由Astra审计 | T11,T12,T23,T54,T66 |
| G00-20 全量source closure尚未形成entry→call→permission/reject/error/cleanup→sink/root与反向caller双向闭包；companion CF01-CF05为可读源码未展开frontier，不是Owner未决UNKNOWN | `BLOCKED`（施工未完成） | Sol-high WP00继续source closure；Controller收口；Astra复核仪器 | P00 acceptance与依赖其完成的P01 Capsule；T48,T61,T68 |
| G00-21 embedded PS coordinator包含HKCU Uninstall/gogoke及registry backup、四类动态程序、rollback/cleanup；通知拒绝/异常可尝试fallback但非macOS-debug拒绝；daemonctl另有四CLI控制入口和全平台cwd-root；main fix_path_env与iOS native mutation已展开到源码/外部lock边界 | `GAP`（已证分支登记，runtime不声明） | WP03/WP05/WP10b/WP12；源闭包仍由WP00负责 | 对应T19,T39,T41,T43,T46,T50,T52,T55,T59,T60,T63；非新授权 |

## 3. 资产处置与来源/许可边界

每个资产的具体 C01–C42 与 D01–D12 映射固定在 companion §5；下表保留处置结论，不以概括性描述替代机械追踪。

| 资产 | P00 处置 | 依据与边界 |
|---|---|---|
| `apps/desktop` 布局、消息、设计系统、IME/键盘、Git/Files/PTY | 保留 | 保留现有产品能力与组件；公共数据/操作绑定后续替换，静态 lint 不代替 Windows 实机 |
| Rust Codex app-server 接入/shared cores | 修复后复用 | 成熟单家适配器；应包入 driver/host 边界，不得继续作为公共身份、公共 root 或第二生产 writer |
| `packages/seat-runtime/src/seat-runtime.ts` | 修复后复用 | workspace/access、隔离 env、provider 适配有价值；provider/Codex 假设及运行权威须拆除 |
| `packages/seat-runtime/src/seat.ts` | 修复后复用 | native input 校验和 Delivery phase 可借鉴；三布尔 capability 不是 V3 CapabilitySnapshot |
| `packages/seat-runtime/src/close.ts` | 复用候选 | PID+创建时间、树停止与超时结果明确；需与 Room `proc.ts` 合并成单一实现并补未知身份/残留验证 |
| `packages/seat-runtime/src/test-fixtures/*` | 测试复用 | Codex/Claude/Grok 假原生链可构造故障；fixture 永远不是 native provider 证据 |
| `packages/room/src/accounts.ts` | 修复后选择性复用 | instance/account 分离、secret masking、timeout 区分有价值；含真实 CLI login/probe/spawn 路径，不得整体导入 |
| `packages/room/src/proc.ts` | 机制复用、实现待整合 | 与 `close.ts` 重复 PID identity；不允许两套生产语义长期并存 |
| `packages/room/src/cli-input-fixture.ts` | fixture 复用 | 明示 fake native CLI、isolated temp roots、readonly seat；会创建临时 Git/data，不能当被动证据或真实 native 通过 |
| Room server/调度权威 | 不整体采用 | V3 禁止重新接上 Room 总服务或组织调度权；只按符号和传递依赖选用 helpers |
| `packages/protocol` generated types/validators | 复用 | 生成源、public exports 与 validator 有界；必须保持 schema/codegen 单一权威，不自动等于 P01 新合同 |
| `crates/gogo-protocol/core/store` | 证据/机制复用，非批量导入 | 属于另一 kernel/control-plane 线；可借具体测试/存储/协议机制，不导入其调度权威或整套 schema |
| `tools/protocol-conformance` | 历史工具/方法输入 | README 明示 checked-in results 是 immutable historical evidence，不是当前 aggregate verdict |
| draft v1alpha1 schema fixtures | 合同 fixture 输入 | manifest 明示 `DRAFT_SCHEMA_ONLY` 与 `no real Harness conformance` |
| remote/Tailscale/dictation | `preserved_disabled` 目标 | 封全部生产入口但保留代码、设置、缓存和恢复说明；本批次未验证封存完成 |

### 3.1 已核来源快照

| 项 | 当前 Git/许可证据 | 边界 |
|---|---|---|
| desktop | 路径最新 commit `50a1c4f45bdd9d20e5e5c649a25e3ea6d148f10f`；`THIRD_PARTY_NOTICES.md` 记录 CodexMonitor base `dd61b9a`、import `8a2dd1f`、MIT | 必须保留 notice/MIT 条件；逐文件采用与当前 AGPL 分发组合需 item-level 审核 |
| Room | 路径最新 commit `95d0a45f150b8a359a3df8494d3dced3026bb170`；package `AGPL-3.0-or-later` | 同仓库不自动证明可改标注或可整体导入 |
| seat-runtime | 路径最新 commit `41679d33b503ff592ab32b9d25af830b79af0eed`；package `AGPL-3.0-or-later` | 复用须记录具体文件/commit/修改与分发义务 |
| protocol | 路径最新 commit `f3a30749537bce55757e52677e31fca4ecb7c324`；package `AGPL-3.0-or-later` | generated source-of-truth 与输出摘要需固定 |
| gogo protocol/core/store crates | 分别为 `a252ba7b...` / `5f3e128...` / `1bc7693...` 的路径最新证据 | 另一工程线，不是本专项默认 runtime authority |
| repository root | `LICENSE` 为 AGPLv3 文本 | 本文不作法律结论、不改变任何许可标注 |

## 4. 团队与 Provider 的有效能力矩阵

施工角色与产品要接入的 Codex/Claude Code/Grok Build/OpenCode/Antigravity CLI 是两套概念，不能互相代替。

| 角色/通道 | V3 路由意图 | 本基线有效证据 | 当前授权结论 |
|---|---|---|---|
| Owner | 最终权威 | 当前明确启动 V3、保留最终裁决 | 可裁决目标/超范围/最终 acceptance；本文不代替 |
| Controller | Sol/medium | 当前任务由 Controller 派发限定 Capsule | 负责集成、升级和范围内技术结论；不得自证独立验收 |
| Luna exploration/construction | medium/high/max 按形状 | B0-01～03 为只读 evidence routes；项目路由存在 | 仅按新 Capsule 与不重叠 ownership 执行 |
| Sol/high | 规划、根因、复杂核心 | B0-04/P01 设计路线 | 可定范围内技术设计；实现者不兼 fresh acceptance |
| fresh Sol/high | 普通独立复核 | 路由规则存在；尚未对本文执行 | 本文必须由 fresh Sol 复核后才可标 P00 accepted |
| Astra/high/xhigh | Specialist / 高风险全轴 | 路由规则存在；本批次未调用 | 只在架构/安全/并发矛盾或高风险审计阶段使用，不代 Owner |
| 外部 Claude | 最终异构审查 | 本批次未调用 | 仅审最新最终候选；V2 审计是历史证据 |
| Grok Worker | Owner/V3 允许合格独立整包，固定 `grok-4.6/high/standard` | `grok-worker 1.0.2`；doctor pass；4 个 OAuth-ready profile 的旧快照声明 4.6，但 `identityStatus=unknown`；pool 显示 active/workloadEligible、provider healthy、`realRequests=0`；本 worktree `registered=false`、`reparseFree=true`；installed skill/CLI 声明 `readonly|workspace-write` interface | **当前不得派 Grok 写 capsule**。零请求状态不能证明 write/terminal/subprocess/test；模型快照非本日且身份 unknown；未执行 probe/task init/plan/run；acceptance 不适合。需要新授权和新能力证据后才能重新判定 |

Grok 路由冲突的裁定是：Owner/V3 的“可承接整包”是意图；installed skill 的 `workspace-write` 是接口声明；当前 repo 路由仍记录旧只读边界，而 B0-03/本次只读检查没有产生一次真实请求或写/终端/子进程/测试证明。因此 runtime 能力保持 `UNKNOWN`，不以文档或接口声明提升权限。当前施工改派合格 Luna/Sol；Grok 不得输出建议后被登记成整包施工完成。

## 5. P01 公共合同设计输入（未冻结）

依赖方向固定为：现有 UI/工具 → 公共服务 → Rust host → 可选 driver/受限 protocol worker。以下只规定 P01 必须表达的语义和拒绝条件，不决定字段编码、持久化 schema 或 API 名称。

| 对象 | P01 必须表达 | 必须保持的 invariant | 尚未决定 |
|---|---|---|---|
| Session | gogoke 稳定 ID、project/root scope、privacy domain、可选 role、title、lifecycle、version | 不是 PID、native thread、view 或 provider account；归档/关闭视图不默认删除/停止；跨域原生 ID 不碰撞 | ID 编码、archival/version state machine、旧 thread 映射规则（WP01/WP02） |
| NativeBinding | driver、instance、profileRevision、authRevision、nativeSessionId、generation、continuationMode、evidence time | native ID 仅在绑定内有效；binary/profile/auth/mode 改变使旧能力/绑定证据失效；在途不静默换目标 | revision 获取/递增规范、同一进程多 workspace 是否允许（WP01/WP03/WP09a） |
| Execution / Delivery | Execution 与一次 Delivery 分离；operationId、request fingerprint、固定 target/binding generation、accepted evidence、native correlation、terminal/unknown status | 先耐久登记再发送；相同 ID 异内容拒绝；接受不明不盲重放；queue/steer/stop/exit-0 不互相冒充 | per-provider receipt/correlation、恢复与保留期、背压预算（WP01/WP04） |
| ViewBinding / ComposeTarget | view/composer 固定 Session、privacy domain、version、intent、draft/attachment identity | 后端重新鉴权，不信前端“当前项目”；双视图不双执行；切换/迟到事件不改旧 target | focus/read-version 与 draft persistence shape（WP01/WP05） |
| PublicEvent | schema/version、source driver、subject Session/Execution/Delivery、binding generation、sequence/cursor、bounded payload、raw diagnostic reference | unknown/half/out-of-order/old-generation frame 不授权、不虚报完成；重放幂等；gap 可恢复 | event taxonomy、raw retention/redaction、subagent/plan mapping（WP01/WP04/WP06） |
| CapabilitySnapshot | binary canonical path/digest/version、platform、mode、instance、profile/auth revision、observedAt、evidence source、每能力 true/false/unknown | PATH 命中或静态 booleans 不等于 supported；unknown 不作 false/zero；关键 identity 变化使 snapshot 失效 | capability vocabulary、freshness/expiry、无副作用 probe 与 native probe 分界（WP01/WP09a） |
| ContextPackage | explicit recipient、task、fixed material refs+digests、source/provenance、visibility、permission scope、expiry/parent link | 不带父对话全文；正文 `@`/模型自称不能授予权限；worker 结果回 controller；原生 subagent/MCP/hook 不绕宿主 | material manifest、projection/redaction format、结果关联（WP01/WP04） |
| HumanActionRequest | original request ID/continuation、Session/Execution target、generation、expiry、allowed answers/permission scope、secret handling | 回答是原 continuation，不是新普通消息；错人/错代/过期/已结束拒绝；文本不能伪造真实批准控件 | provider-specific option encoding、secret answer storage/log policy（WP01/WP04+Astra） |

### 5.1 跨对象最低规则

- Session 是公共身份；NativeBinding 是可替换、可版本化的原生关联。
- Execution 表示一次工作；Delivery 表示一次投递事实；ViewBinding 只表示视图/输入目标。
- 所有可改变状态的请求必须带固定 Session、binding generation、operation identity 与后端授权上下文。
- PublicEvent 只能推进它明确指向且 generation 匹配的对象；未知事件可诊断，不可授权。
- CapabilitySnapshot 是带证据时间的观察，不是永久承诺；`unknown` 必须是一等状态。
- ContextPackage 与 HumanActionRequest 是两条独立授权/continuation 接缝，均不能从普通模型文本推导权限。
- P01 必须生成 machine-readable execution/trace 数据及 validator；当前仓库不存在该 V3 companion，不能引用旧 `docs/plan/validate_plan.py` 或旧 protocol validator 冒充。

## 6. T01–T68 预期输入/输出与阶段台账

除 T68.P 的本文追踪输入外，本表全部状态为 `PLANNED_UNEXECUTED`。`TEST_EXISTS` 只表示仓库有相关旧测试/fixture，不表示对应 V3 T 已实现或运行。阶段：`L` 当前主责包，`N1` 首批真实 provider，`I` 完整集成，`M` 迁移/安装/回滚，`H` WP03 双宿主。

| 测试组 | 预期输入 | 预期输出/拒绝结果 | 阶段/截止包 | 本批状态 |
|---|---|---|---|---|
| T01,T02,T03,T04,T05,T06 无 Codex/基础兼容 | 无 CodexDriver build；无 binary/login/Home；离线 Git/Files/PTY/resources/history；真实首批与多协议异形/错误 frames；unsupported/unknown snapshots | 公共层不导入原生协议；产品不暗装/兜底；离线能力可用；至少 Codex+一非 Codex 受控样本；未知/错误不虚报成功或零用量 | T01-02 L@WP11b；T03 L@WP08,I@WP11b；T04 L@WP11a,I@WP11b；T05 L@WP01,I@WP11b；T06 L@WP09a,I@WP11b | `PLANNED_UNEXECUTED` |
| T07,T08,T09,T10,T11,T12 Session/UI/隐私 | 双 composer、A/B project、view close/reopen、相同 native ID 的合成与真实实例、private side chat、现有窗口/IME/主题 | target/draft/attachment/focus 不串；Session 不误建/杀；复合身份不碰撞；私聊不外泄；视觉/交互不退化 | T07-09,T12 L@WP05,I@WP11b；T10 L@WP02,N1@WP11a,I@WP11b；T11 L@WP04,I@WP11b,M@WP12 | `PLANNED_UNEXECUTED` |
| T13,T14,T15,T16,T17,T18,T19,T20 Delivery/事件/批准 | operationId 重放/冲突、丢响应、queue/steer、乱序/gap/旧代、stop priority、EOF/half-frame/tool error、错/过期 action answer | 不双执行/盲重放；stale/冲突拒绝；事件不复活旧状态；控制不饿死；失败内容保留；continuation 精确归属 | T13-15,17-20 L@WP04,N1@WP11a,I@WP11b；T16 L@WP04,I@WP11b | `PLANNED_UNEXECUTED` |
| T21,T22,T23,T24 权限/路径/秘密/正文 | host readonly 与 native/OS readonly、中文/空格/父目录/link/junction/hardlink、logs/export/notifications/migration、正文中的 owner/@ 文本 | 不扩大权限/逃根/泄密；模型文本不授予执行权 | T21 L@WP03,N1@WP11a,I@WP11b；T22 L@WP08,I@WP11b；T23 L@WP09b,I@WP11b,M@WP12；T24 L@WP04,I@WP11b | `PLANNED_UNEXECUTED` |
| T25,T26,T27,T28,T29,T30 资源/审查/生成 | prompt CRUD/import corruption；template/native config；Skills/MCP/Apps；fixed Git review；fork/snapshot/compact；真实非 Codex 生成 | 原件/未知字段保留，加载不越权；review 材料固定；概念不混称；生成使用所选真实实例，失败留草稿 | T25-27 L@WP07,I@WP11b；T28-29 L@WP06,I@WP11b；T30 L@WP08,I@WP11b | `PLANNED_UNEXECUTED` |
| T31,T32,T33,T34,T35,T36 host/process/tools/usage | 多配置容量、失败/超时/crash、多 PTY、临时 Git/worktree、attachments、usage windows/units | instance/context 分离；进程可正确收尾；工具不重复/不伤改动；附件内容语义不降级；unknown 不作 0 | T31-32 L@WP03,N1@WP11a,I@WP11b；T33-35 L@WP08,I@WP11b；T36 L@WP09b,I@WP11b | `PLANNED_UNEXECUTED` |
| T37,T38,T39,T40,T41,T42 migration/owner/writer | 全量旧数据、七点故障、坏 JSON/db/重复导入、native Home absent、新旧 writer、升级后新增数据回滚 | 关系/来源保全；坏数据不清空；获准历史可读；不双发；新增内容回滚不丢 | T37-38,T42 L@WP12；T39 L@WP02,I@WP11b,M@WP12；T40 L@WP09b,I@WP11b,M@WP12；T41 L@WP03,I@WP11b,M@WP12 | `PLANNED_UNEXECUTED` |
| T43,T44,T45,T46,T47,T48 reconnect/release/performance/closure | 旧 RPC、disconnect/reconnect、移动遗产、Windows installer/update/uninstall、同机同数据 perf、C/entry/D ledger | 旧入口同服务或拒绝；不重发/伪停；暂缓平台不背书；信任/回滚正确；性能可比；删除闭环不靠品牌检查 | T43-45 L@WP10b,I@WP11b；T46 L@WP12；T47 L@WP11b；T48 L@WP13 | `PLANNED_UNEXECUTED` |
| T49,T50,T51,T52,T53,T54,T55 隔离/进程/安全合并 | 高熵上下行正反控制、全 spawn env、revision 变化、PID reuse/残留、delivery crash/backpressure、CSP/asset/IPC/content、root alias/two-lock hosts | 私域不可达；环境无旁路；binding 不静默换；不误杀/虚停；终态不丢；内容/批准不越域；单 owner | T49 L@WP04,N1@WP11a,I@WP11b；T50-52 L@WP03,N1@WP11a,I@WP11b（T50 M@WP12）；T53 L@WP04,I@WP11b；T54 L@WP05,I@WP11b；T55 L@WP02,H@WP03,I@WP11b,M@WP12 | `PLANNED_UNEXECUTED` |
| T56,T57,T58,T59,T60,T61,T62,T63,T64 封存/能力/供应链/完整矩阵 | remote/voice entry matrix、snapshot invalidation、trust/worker tamper、asset provenance、trace validator、五家真实矩阵、lost lease、多 topology | remote/voice 保持关闭且资产保全；capability stale；信任根不被导入改写；来源可追；计划缺陷被拒；五家证据不冒充；旧写域不并启；身份/计量可核 | T56-57 L@WP10a,I@WP11b,M@WP12；T58 L@WP09a,N1@WP11a,I@WP11b,M@WP12；T59 L@WP12；T60 L@WP03,I@WP11b,M@WP12；T61 L@WP01,I@WP11b；T62 L@WP11b；T63 L@WP03,N1@WP11a,I@WP11b,M@WP12；T64 L@WP11b | `PLANNED_UNEXECUTED` |
| T65,T66,T67,T68 活动/未读/委托/工程交接 | plan vs execution/subagent events；read versions/background/late events；structured delegation/clean review/continuation；Task Capsule/Provider capability/candidate evidence | 计划不冒充执行；未读归属正确；委托无旧旁路且材料最小；Grok 缺能力不启动、自审拒绝、错版证据拒绝、Owner 裁决可追 | T65 L@WP06,I@WP11b；T66 L@WP05,I@WP11b；T67 L@WP04,N1@WP11a,I@WP11b；T68.P@WP00,D@WP01,L@WP13 | `T68.P EVIDENCE_PRODUCED_PENDING_FRESH_REVIEW`; 其余 `PLANNED_UNEXECUTED` |

T68.P 本次只证明：Owner 路由意图、repo 旧路由、installed skill 声明和实际零请求证据已分开；基线、计划、worktree、输入摘要及不自验要求可追。它不证明 T68.D validator、Grok 写能力、最终候选审计或 Owner 最终 acceptance。

## 7. Windows T47 性能比较合同

以下门槛由 Controller 在 `GKD-P00-REPAIR-01` 中固定，是 T47 首次运行前的 P00 合同输入。baseline 与 candidate 必须在同一物理 Windows 机器、OS build、CPU/RAM、power mode、显示缩放、存储位置、后台进程策略与网络条件下，使用各自正式 build 比较；fixture/dev server 不得代替正式候选，冷启动不能与热启动比较。

### 7.1 固定数据集类别与规模

首轮 T47 Capsule 生成并冻结实际脱敏数据集及 SHA-256；baseline 与 candidate 必须使用同一数据集摘要。固定类别/规模为：

- 空库；
- 10 个 workspaces；
- 500 个 Sessions；
- 每个 Session 200 条 messages、20 条 tool-or-diff events；
- 单条最大 message 为 1 MiB；
- 至少 10,000 条 file index；
- 中文 IME 脚本 1,000 次输入；
- 同一套脱敏附件集。

### 7.2 固定运行次数和记录

- 冷启动：2 次预热只用于仪器检查，随后 7 次正式测量。
- 热启动、历史加载、workspace 切换、composer+IME、流式 UI：各 3 次预热，随后各 15 次正式测量。
- 稳态与峰值内存：连续采样至少 10 分钟。
- 正式 clean build：3 次。
- 每个候选记录 plan revision、commit、build digest、effective config digest、依赖锁、runtime/worker/CLI path+digest+version、数据集 digest 与 root identity。运行组件必须自报实际加载身份，不能只看源码路径。
- 保存全部原始值、median、p95、失败样本及环境漂移记录；外部 wall clock/process counters 与应用阶段计时/loaded identity 必须先交叉验证仪器。

### 7.3 排除与 PASS/FAIL

- 只允许排除 T47 Capsule 在运行前列明的环境失效；任何产品失败、身份不符或 skip 都不能被排除为 PASS。
- 每个适用场景必须同时满足：latency median `<= baseline * 1.10`，latency p95 `<= baseline * 1.15`。
- composer/IME event-to-visible p95 必须同时 `<= 50 ms` 且 `<= baseline * 1.10`。
- steady memory 与 peak memory 各自必须同时 `<= baseline * 1.15` 且相对 baseline 的绝对增加 `<= 150 MiB`。
- clean build median 必须 `<= baseline * 1.20`。
- 正式安装包体积必须 `<= baseline * 1.10`。
- 必须为零身份错版、零数据丢失、零崩溃。
- 任一适用项不满足即 T47 `FAIL`。阈值只能由 Owner/Controller 在运行前正式修订 V3/P00；施工者、测试执行者或审计者不得临时放宽。

## 8. 未决事项、Owner 与阻塞点

| Unknown/Decision | 建议所有者 | 阻塞点 |
|---|---|---|
| Session ID、lifecycle、旧 thread mapping 的精确定义 | Sol-high WP01/WP02 | P01 schema/P02 storage |
| app-server 多 workspace 复用是否允许及匹配键 | Sol-high WP03；冲突交 Astra-high | P03 launch contract |
| native history/public content/localStorage 的权威与导入边界 | Sol-high WP02/WP06 | P02 schema、P12 migration |
| PublicEvent 对 native subagent/plan/background job 的分类 | Sol-high WP01/WP04/WP06 | T05.L/T65.L |
| per-provider accepted receipt、continuation、steer/stop 非等价行为 | Sol-high WP04，WP09a 提供证据 | P04/N1 |
| `full-access` 谁可请求/批准/审计 | Owner 定产品权限；Sol-high+Astra 定机制 | P04 前 |
| Windows app-data 实际路径、root alias/lock 与 runtime self-identification | WP02/WP03 | T55.L/H |
| V3 execution JSON schema、生成源、validator/CI 路径 | Sol-high WP01；Luna-max 实现 | T61.L/T68.D |
| Grok write/terminal/subprocess/test 的实际能力 | Controller 在另行明确授权下取新 Provider 证据 | 任一 Grok write Capsule |
| CodexMonitor 派生项的逐文件采用/分发结论 | Owner+Astra/必要专业审核 | 相关 P12 分发 |
| 生产旧数据隔离副本授权与脱敏规则 | Owner/Controller；WP02 执行 | P02 migration fixtures |

## 9. 删除边界与非声明

### 9.1 删除边界

- 本批次不删除或禁用任何产品代码、配置、license、用户数据、`.codex`/CODEX_HOME、CLI、历史、worktree、第三方 notice、开发 AGENTS/.codex。
- D01–D12 仍是退出计划，不是删除授权。每项删除前必须绑定替代实现 commit、对应最新测试证据、旧数据读取方、全部入口迁移、回滚需要和独立复核。
- 不追求仓库零 Codex 字符串；adapter/schema/fixture/migration/provenance 可以保留，公共本体不得依赖 Codex 专属身份、root、隐形执行路。
- remote/voice 的目标是 `preserved_disabled`，不是删除；local daemon 不随 remote 封存被删除。

### 9.2 本文明确不声明

- 不声明 P01 合同已冻结或 companion execution JSON/validator 已存在。
- 不声明任何 T01–T68 runtime test 通过；T68.P 也待 fresh Sol 独立复核。
- 不声明 desktop/daemon parity、无 Codex 启动、五家 native 支持、真实只读、停止、恢复、性能或安装链已验证。
- 不声明 Grok 可写/可运行终端/子进程/测试，也不授权 Grok write Capsule。
- 不声明 fixture、静态 test、historical conformance result、doctor pass、OAuth-ready 或 active profile 等于 native capability/acceptance。
- 不声明任何许可重授、法律合规完成或 CodexMonitor 派生项可无条件分发。
- 不声明生产数据已复制、迁移或检查；本批次未访问生产数据。
- 不声明 daemon/daemonctl 的 cwd-relative fallback 已修复；当前只完成源码定位、owner 与阻塞阶段登记。
- 不声明 clipboard/plugin OS side effects 已经过隐私或权限验证，也不把 clipboard 成功当作 Delivery、授权或持久化成功。
- 不声明 macOS transient HEIF/icon 文件一定清理、无残留/碰撞，或当前 `sips` 路径/打包身份已经验证。
- 不声明 Linux startup env 不会传播到后续 spawn；未捕获 final env 前不能通过 T50。
- 不声明 macOS/Windows platform lifecycle 已经满足 focus、长期 Session、退出或视觉验收。
- 不声明 AudioContext 请求实际可听、已获权限或构成通知送达/已读；声音时序隐私尚未验证。
- 不声明source census已完整；CF01-CF05尚未解释，未计算孤立sink数不得写0。registry/clipboard/process计数相等不是双向closure。
- 不声明通知permission拒绝后fallback成功；non-macOS-debug fallback源码返回Err，wrapper返回不证明用户可见。
- 不声明PS coordinator的registry备份/安装/动态程序/rollback/cleanup安全或成功；已读脚本不等于运行取证。
- 不声明fix_path_env外部实现已经读过或loaded binary与lock匹配；只固定git revision和owner/stage。
- 不声明施工自测、Controller 技术收口、Astra/Claude 的历史计划审查等于 Owner 最终裁决。

## 10. P00 后允许准备的下一批 Capsule

以下只表示依赖形状允许准备，不是自动授权；每次仍须固定新 HEAD、artifact digest、worktree、允许路径、runtime 权限和 stop conditions。

1. **P00 source closure continuation**：当前CF01-CF05未完成，Sol/high继续逐member展开并反扫zero-orphan proof；不能派发依赖P00完成的P01施工。闭合后再发fresh Sol/high复核，Astra核closure仪器；不得边审边改后给同一候选盖章。
2. **B1-01 / WP01 public contract**：Sol/high 在 P00 accepted 后定义八类公共对象、错误/version semantics、边界例外台账，并创建真正的 V3 machine-readable execution/trace source；身份/授权设计交 Astra 首次风险审计。
3. **B1-02 / WP01 trace validator**：只在 B1-01 schema 固定后给 Luna/max；实现 T61.L/T68.D 的正负检查，不得为让 validator 变绿修改行为承诺。
4. **B1-03 / WP10a sealed entries**：P00 accepted 后给 Luna/high/max，使用明确 UI/backend/旧 config/RPC/auto-start 清单；保持 local daemon 与既有缓存，不触碰系统 Tailscale 身份。
5. **WP02 design/construction preparation**：只能消费 fixed P01；生产旧数据副本仍需单独授权。WP03 及后续不得越过 P01/P02 前置。

当前不允许的下一项：任何 Grok 写施工、五家真实 provider 调用、production data migration、服务控制、license 修改、大规模 Codex 删除或发布动作。

## 11. 原始证据索引

- `START_HERE_CODEX_DECOUPLING_V3.md`
- `docs/design/gogoke-codex-decoupling-plan-v3.md`
- `docs/design/gogoke-codex-decoupling-p00-entry-inventory-v3.md`，SHA-256 `290159D7C16E75C1752741C33CEE3373FE09836DAD6375077ED149110FD82539`
- `AGENTS.md`; `apps/desktop/AGENTS.md`; `docs/model-routing.md`
- `apps/desktop/src/App.tsx:43,62`
- `apps/desktop/src/features/app/components/MainApp.tsx:69-105`
- `apps/desktop/src/features/app/bootstrap/useAppBootstrap.ts:8-28`
- `apps/desktop/src/features/workspaces/hooks/useWorkspaceCrud.ts:98-115`
- `apps/desktop/src/services/tauri.ts:376-399,448-470`
- `apps/desktop/src/services/events.ts:28-89`
- `apps/desktop/src/features/app/components/MainHeader.tsx:315`; `features/app/hooks/useSidebarMenus.ts:58`; `features/debug/hooks/useDebugLog.ts:79`
- `apps/desktop/src/features/messages/components/Markdown.tsx:373`; `features/messages/components/useMessagesViewState.ts:149`; `features/messages/hooks/useFileLinkOpener.ts:232`
- `apps/desktop/src/features/workspaces/components/ClonePrompt.tsx:159`; `features/git/components/GitDiffPanel.tsx:302,497,503`; `features/threads/hooks/useCopyThread.ts:20`; `features/settings/components/sections/SettingsEnvironmentsSection.tsx:175-184`
- `apps/desktop/src/features/threads/utils/threadStorage.ts:3-16,39-69,72-103,109-180`
- `apps/desktop/src-tauri/src/state.rs:33-68`
- `apps/desktop/src-tauri/src/codex/home.rs:6-20`
- `apps/desktop/src-tauri/src/workspaces/commands.rs:27-35,589-614`
- `apps/desktop/src-tauri/src/backend/app_server.rs:648-699,751-840`
- `apps/desktop/src-tauri/src/shared/codex_core.rs:204-267,476-528,530-580`
- `apps/desktop/src-tauri/src/shared/codex_core.rs:57-105,156-180,402-429`
- `apps/desktop/src-tauri/src/workspaces/macos.rs:149-195`; `workspaces/commands.rs:655-659`; `bin/gogoke_daemon.rs:999-1003`
- `apps/desktop/src-tauri/src/lib.rs:72-92,112-137,334-339`
- `apps/desktop/src/utils/notificationSounds.ts:25-80`; `features/notifications/hooks/useAgentSoundNotifications.ts:35-38`; `features/app/hooks/useUpdaterController.ts:94-100`
- `apps/desktop/src-tauri/src/workspaces/macos.rs:65-87` (`defaults read` before icon `sips`)
- `apps/desktop/src-tauri/src/main.rs:5-8`; `src-tauri/Cargo.lock:1240-1247`；fix-path-env revision `c4c45d503ea115a839aae718d02f79e7c7f0f673`
- `apps/desktop/src-tauri/src/bin/gogoke_daemon/transport.rs`; `src/bin/gogoke_daemonctl.rs:74-147,246-289,569-749,754-1249`
- `apps/desktop/src-tauri/update/gogoke-update-coordinator.ps1`; `src/gogoke_update.rs:29,577-647`; `src/notifications.rs:28-56`; `src/bin/gogoke_daemon.rs:1378-1405`
- `apps/desktop/src/services/tauri.ts:1149-1221`; `src-tauri/src/window.rs:17-77,80-125`; `src/main.tsx`; `index.html`
- `apps/desktop/src-tauri/src/shared/process_core.rs`与companion §9.5逐membercandidate列表；companion §9.6未展开frontier。
- `apps/desktop/src-tauri/src/event_sink.rs:16-26`
- `apps/desktop/src-tauri/src/bin/gogoke_daemon/rpc/dispatcher.rs:3-29`
- `apps/desktop/src-tauri/src/bin/gogoke_daemon/rpc/codex.rs:13-20,33-107,159-210`
- `apps/desktop/src-tauri/src/bin/gogoke_daemon/rpc.rs:38-53`
- `packages/seat-runtime/src/seat.ts:1-5,13-67,68-139,141-214`
- `packages/seat-runtime/src/seat-runtime.ts:132-167,191-224`
- `packages/seat-runtime/src/close.ts:1-30,54-80`
- `packages/room/src/proc.ts:1-50`; `packages/room/src/accounts.ts:1-100`
- `packages/room/src/cli-input-fixture.ts:1-79`
- `packages/protocol/package.json:1-45`
- `tools/protocol-conformance/README.md:7-34`
- `spec/draft/v1alpha1/schema-tests/manifest.json:1-30`
- `apps/desktop/THIRD_PARTY_NOTICES.md:1-16`; `apps/desktop/LICENSE:1-20`; root `LICENSE`
- Git read-only HEAD/status/hash/log；tracked filename/content search for V3 companion；`grok-worker version/doctor/profiles list/pool status/roots inspect`。未运行 `probe/task init/plan/run`，所有只读 pool 输出仍为 `realRequests=0`。

## 12. 独立复核入口

fresh reviewer 应从固定 baseline、本文、两份输入 digest 和上述源码重新取证，特别反证：

1. 是否有本文漏掉的真实生产入口、第二 writer、数据 root 或 V3 companion；
2. 是否把 fixture/test existence/Provider interface 错写成 runtime capability；
3. 八类 P01 输入是否能表达 C01–C42 的关键身份、交付、权限、隐私与 continuation 行为；
4. T01–T68 的输入、输出、阶段是否有遗漏、倒置或未来阶段反向阻塞；
5. 资产处置和许可边界是否把“同仓库/独立进程”误当成权利结论；
6. 本文后续如有修改，是否重新绑定 fresh review，而非沿用旧结论。
