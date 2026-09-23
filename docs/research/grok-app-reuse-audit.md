# Grok App 源码复用审计与非接入边界

- 审计日期：2026-08-11
- 审计对象：本机已安装 Grok App、公开仓库 `RongleCat/grok-app`、本机公众版 Grok CLI
- 结论性质：实现前设计输入，不代表已经复制源码或接受 ADR

## 1. 固定证据

| 对象 | 固定版本 | 用途 |
|---|---|---|
| 本机 Grok App | `0.2.8` | 本机对照证据；不得成为产品运行时或接入对象 |
| Grok App `v0.2.8` | `686b4c1a614936c44b5480b6540d34438c0d63fe` | 对照已安装版本的公开源码 |
| Grok App `main` | `4789623e00a5b40b25c11cb7094c747bf97a65f4` | 审计当前架构；仓库版本为 `0.2.14` |
| 本机公众版 Grok CLI | `grok 1.0.0 (3cd0d0cbce)` | 确认 GOGO PARTY 的 Grok 主路径仍可面向公众发行物 |

公开仓库声明自己是非官方桌面客户端，通过真实 `grok agent stdio` ACP 接入 Grok CLI；源码采用 MIT License。MIT 允许复制、修改和再发布，但派生文件必须保留版权与许可声明。

本机 `<local>\grok-app` 是已安装发行物，不是开发仓库。`<local>\Grok UI` 是另一个已冻结项目，本次未读取为复用来源、未修改。

## 2. 值得直接复用的能力

当前源码已经解决了大量不应由 GOGO PARTY 从零重写的 Grok 专属运行时问题：

| Grok App 能力 | 主要源码边界 | GOGO PARTY 处理 |
|---|---|---|
| ACP JSON-RPC、stdio/TCP、反向 RPC、流解析 | `src-tauri/src/acp_client.rs` | 抽成 Grok Adapter 私有 transport，不进入通用 Controller |
| Session 状态机 | `session_fsm.rs` | 复用状态迁移与负向测试，映射到统一 Adapter 状态 |
| 多会话宿主、恢复、路由、后台流 | `session_manager/` | 选择性抽取；保留 GOGO 的 Project/Agent/Attempt 身份映射 |
| Prompt 完成与迟到工具事件判定 | `turn_complete.rs` | 直接借鉴 terminal-event fence，禁止以进程退出代替任务回执 |
| Stall、watchdog、进程上限与空闲回收 | `stream_stall.rs`、`session_manager/watchdog.rs`、`process_limits.rs` | 复用机制；阈值和处置由 GOGO Policy 决定 |
| 权限请求、项目外路径判断 | `permission.rs`、`permission_rules.rs`、`path_scope.rs` | 复用解析器和测试；最终授权仍由 GOGO Gate 决定 |
| 原子写、锁、审计、诊断包 | `store_lock.rs`、`audit_ledger.rs`、`support_bundle.rs` | 复用底层工具，不复用其单应用事实模型 |
| CLI 探测、安装、升级 | `cli_probe.rs`、`cli_install.rs`、`cli_update.rs` | 借鉴公众发行物探测；供应链策略需由 GOGO 单独收紧 |
| 上下文用量、续会话与 compact 提示 | `acp_client.rs`、`session-continuity.md` | 接入 Context Governor，但不以 Token 百分比单独决定换 Session |
| 手机镜像 RPC | `mirror/` | 仅作为源码结构参考；不得连接 Grok App 实例 |

这些模块拥有大量 Rust 单元测试、ACP golden fixture、路由/卡死/权限测试。复用时应连同对应 fixture 和负向测试迁移，不能只复制 happy path。

## 3. 禁止直接接入 Grok App

Grok App 是非官方社区项目。即使其源码包含可调用的 HTTP/WebSocket Mirror Host，GOGO PARTY 也不得把 Grok App 作为产品组件或外部运行时接入。

产品硬边界如下：

1. 不检测、启动、托管或关闭用户安装的 Grok App。
2. 不连接 Grok App Mirror RPC、Tauri IPC、本地存储或会话目录。
3. 不导入、展示、操作或声称管理 Grok App 原生 Session。
4. 不要求用户安装 Grok App，不把它列为兼容组件、插件或可选依赖。
5. 不使用 Grok App 名称或界面制造官方合作、官方兼容或 xAI 背书的暗示。

“复用源码”仅表示：在 MIT License 条件下，把经过审计的通用或 Grok CLI 适配代码复制、修改并纳入 GOGO 自己的代码库，由 GOGO 自己构建、测试、发布和承担运行时 authority。

## 4. 不应直接复用的部分

- 不把 Grok App 的 `automation_runner` 当作 GOGO Workflow Engine。它是进程内定时轮询，并以 Grok App Session 为执行对象。
- 不把 Grok App journal 当 Canonical Fact Plane；完整 Transcript 仍属于 Native Session Plane。
- 不把 Grok App 的 Project ID、Session ID 当作跨项目控制句柄。
- 不把 `--always-approve`、YOLO 或 Agent 端静默放行当成 GOGO 的权限证明。
- 不让 Grok App 管 Codex 或未来 Harness；统一调度策略必须留在 GOGO Controller。
- 不把本机已安装 EXE 打包进大众产品，也不要求用户先安装 Grok App 才能使用 Grok Adapter。
- 不通过任何网络、本地 IPC、文件或进程接口连接已安装 Grok App。

## 5. 推荐的两层落地路线

### A. 官方 CLI 直连（规范主路径）

GOGO Runner 直接启动公众版 `grok agent stdio`，Grok Adapter 持有 ACP 会话。用户只需安装公众版 Grok CLI；这条路径接受统一 Conformance Suite。

### B. Grok App Derived Runtime（代码复用路径）

从固定上游 commit 选择性移植 Grok 专属 Rust 模块，形成 GOGO 内部 `grok-runtime` crate。每个派生文件记录上游路径、commit、修改说明，仓库携带 MIT LICENSE/NOTICE。通用状态、权限和工作流语义通过 Adapter Contract 暴露，Grok 专属类型不能泄漏到 Controller。

## 6. 实现 Gate

在复制或修改上游源码前必须完成：

1. 对 `Grok CLI version × GOGO Adapter version` 生成 capability digest；Grok App 版本不得成为产品兼容维度。
2. 用公众 Grok CLI 当前版重跑 ACP initialize/new/load/prompt/cancel/permission/ask-user golden tests。
3. 验证 Windows 子进程树停止、迟到事件、断流恢复和跨 Project journal 污染的负向场景。
4. 对派生源码完成 license provenance 清单。
5. 静态检查发布物和运行时代码不包含 Grok App 进程发现、Mirror RPC、IPC 或会话目录接入路径。
6. 任何未验证能力在 UI 中显示 `UNKNOWN` 或 `Spike verified`，不得显示 `Conformance PASS`。

## 7. 设计结论

采用“官方 CLI 直连 + 源码选择性复用”的组合，明确拒绝任何 Grok App 运行时接入。这样可以借用成熟工程经验，同时让 GOGO PARTY 的发行、品牌、兼容承诺和控制权只建立在官方公众版 Grok CLI 与自有代码之上。
