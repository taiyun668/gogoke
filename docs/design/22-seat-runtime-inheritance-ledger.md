# 席位运行时：继承账本与自建清单

- 状态：`v1.0-evidence-backed`
- 日期：2026-08-22
- 性质：**技术设计，不是治理契约。** 不需要 Owner `ACCEPTED`，不需要 fresh audit，不绑定 contract-manifest digest。
- 与既有文档的关系：本文件是 README「方法变更」第三节施工顺序第 1、2 步的技术底稿。

## 0. 这份文档为什么和前面 216 篇不一样

前面所有设计文档的每一条都是推导出来的。**本文件的每一条都有一个正在生产里运行的实现出处，带文件和行号。**

判据很简单：如果一条主张找不到「哪里已经这么跑着」，它就不该出现在这份文件里。

## 1. 产品形态（一句话）

一个项目 = 一个群聊房间。人和多个 worker 都是房间里的席位，任何席位可以 `@` 任何席位。材料走 GitHub，不走聊天。房间是项目上下文；点开一个 worker 进入只属于它的持久上下文。

推论：**没有流程。** 没有 pipeline、没有 stage、没有 workflow DAG、没有 fan-in/fan-out、没有 Transition Engine。`@` 就是调度器。通道对称，策略按角色。

## 2. 三个来源

| 来源 | 提供什么 | 形式 |
|---|---|---|
| `<local>\Grok Worker Provider` | 隔离、契约、密钥卫生、围栏、审计 | **范式继承**，不改造它 |
| agentchattr（MIT） | `@` 路由、频道读写、控制台注入、循环保护 | **概念接入**，GOGO 自己实现 |
| Claudexor（MIT） | 配额池选择、异构双 provider family 审计门 | **思路参考** |

三者都不是 GOGO 要成为的东西。GOGO 是自己的产品。

## 3. 继承清单

每条都标注出处。除非另有说明，出处均为 `<local>\Grok Worker Provider\lib\provider.js`。

### 3.1 隔离原语（第 442 行 `isolatedEnv`）

```js
const env = { ...process.env };
delete env.XAI_API_KEY; delete env.GROK_FOLDER_TRUST; delete env.GROK_SANDBOX;
env.GROK_HOME    = profile.grokHome;     // 每账号持久
env.HOME         = invocationHome;        // 每次调用临时
env.USERPROFILE  = invocationHome;
env.LOCALAPPDATA = path.join(invocationHome, "AppData", "Local");
env.GROK_CLAUDE_HOOKS_ENABLED = "false";
env.GROK_CURSOR_HOOKS_ENABLED = "false";
```

**为什么必须有**：`CAO_REPORT.md` §6 判定「每 worker 的 HOME/配置隔离仍是必需，除非编排器自己注入 per-worker env」，并有实证——OpenCode 曾污染全局 `~/.local/share/opencode` 与 `~/.config/opencode`（`docs/navehq_opencode_global_footprint_boundary_v0.1.md` §2）。agentchattr 的 `command = "claude"` 没有这一层。

### 3.2 两级家目录（第 698 行）

```
profile.grokHome    持久，每账号一个，装认证与 models_cache
invocationHome      临时，每次调用一个，装 HOME / USERPROFILE / LOCALAPPDATA
```

**这是两层上下文模型的落点。** worker 自己的持久存储层已经存在，缺的只是会话生命周期，不是存储。

### 3.3 硬拒绝默认家目录（第 424 行）

```js
assert(normalizeCase(profile.grokHome) !== normalizeCase(DEFAULT_GROK_HOME),
       "INV2_DEFAULT_HOME", "Default user .grok is forbidden.");
```

配合第 327 行 `PATH_DEFAULT_GROK_HOME`：连默认路径的父/子目录都拒绝。全局足迹污染在此被结构性排除。

### 3.4 启动契约是数据，spawn 前校验（第 612、614 行）

```js
invariants: { noPlan, noMemory, streamingJson, terminalDenied,
              subagentsDenied, folderTrustEnabled, trustFlagAbsent,
              compatHooksDisabled }
```

`verifyPlanContract` 在第 617 行逐条断言必需参数存在。

**GOGO 照抄这个形状，但翻转两个不变量**：`noMemory → memory`，`streamDeleted → streamRetained`。见 §4。

### 3.5 密钥卫生（三处）

- 提示词走 `--prompt-file`，**不走 argv**（第 608 行）。直接避开 CAO 记录在案的 `cao launch --env KEY=VALUE` argv 泄密（`docs/navehq_opencode_secret_safe_env_transport_preflight_v0.1.md`）。
- `spawnCapture`（第 433 行）只保留形如 `^[A-Z0-9_]{2,32}$` 的 OS 错误码，绝不保留原始错误文本。源码注释：回执可能被留存审计，不得变成意外的凭据日志。
- `hasSecretKeys()`（第 265 行）与 `redactText()`（第 281 行）全程扫描。

**开源承诺**：产品自身永不经手明文凭据。凭据留在厂商自己的 store 或作用域文件库，registry 只存引用。Claudexor 同此设计。

### 3.6 崩溃安全与所有权围栏

- `atomicWriteJson` + `fsyncDir`（第 288、291 行）、`appendJsonLineAtomic`（第 299 行）
- `captureRunOwner` / `inspectRunOwner`（第 173、186 行）记录 `pid + processStartTicks`，可区分「WAL 属于活进程」与「属于已死进程」
- `taskRunTransaction`（第 208 行）

多席位并发必需。

### 3.7 路径安全（第 308、320 行）

`checkNoReparse` 拒绝符号链接/junction，防止 worktree 越狱；`validateWindowsPath` 约束到允许根内。

### 3.8 项目权威预检（第 517、527 行）

`listAuthorityFiles` / `preflightProject` 拒绝把 worker 派进带可授权配置（如 `.claude/settings.local.json`）的目录。本文件撰写当日实测触发过一次 `PROJECT_AUTHORITY`，`realRequests: 0`，未消耗额度。

### 3.9 运行时策略钩子（`lib/hook-boundary.js`，35 行）

向 profile home 写入 `PreToolUse` 钩子，匹配 `Edit|Write|Bash|run_terminal_cmd`，在 worker 内部执行边界判定。

### 3.10 异构独立审计门（Claudexor）

```
clean verified review/apply gate 要求：至少两个不同的 provider family
--reviewer-panel "claude=...,cursor=..."
```

对应 NaveHQ 已记录的 `REVIEW_INDEPENDENCE_INVARIANT`。**这是多账号多模型的第二个理由**——第一个是配额，第二个是审计独立性无法由同一模型自证。

## 4. 自建清单

| 维度 | grok-worker 现状（出处） | GOGO 必须做的 |
|---|---|---|
| 进程模型 | `spawnSync`，默认 30s 超时（第 433 行） | `spawn` 常驻，会话保活 |
| 记忆 | `--no-memory` 是硬不变量（第 608、617 行） | 反转：席位有自己的记忆 |
| 过程记录 | `rawStreamDeleted: true`（第 855 行区域） | 留存 transcript |
| provider | 全篇写死 Grok（`grok-`、`GROK_HOME`、`models_cache.json`） | 多 provider 抽象 |
| 上层 | 无 | 房间、`@` 路由、两层上下文、可视化建 worker |

### 4.1 分岔点

**`lib/provider.js:433` 的 `spawnSync` 是 GOGO 与 grok-worker 分岔的那一行。**

隔离、契约、密钥卫生、围栏、审计全部可继承；只有进程生命周期这一处必须换掉。换掉它会连带翻转 `--no-memory` 与 `rawStreamDeleted` 两个不变量——而这两个翻转正是「点开 worker 看到它自己的上下文」得以成立的前提。

### 4.2 一次性席位与持久席位

两种都要支持，但能力不同，**必须在界面上如实标注**：

| | 一次性席位 | 持久席位 |
|---|---|---|
| 实现 | 现有 `spawnSync` 路径 | 新的常驻路径 |
| 中途插话纠偏 | **不可能** | 可以 |
| 上下文 | 每次 `@` 都是新人 | 累积 |
| 成本 | 低 | 占用账号 |

「不用停就能纠正方向」这个能力**只存在于持久席位**。

### 4.3 从 agentchattr 接入的四个概念

均由 GOGO 自己实现，因为必须挂在 §3.1 的隔离环境里：

1. `@` 路由经 trigger 队列文件，不经内存
2. worker 通过 MCP 风格的 `chat_read` / `chat_send` 主动读写频道，而非被动接收推送
3. Windows 上用 Win32 `WriteConsoleInput` 向常驻控制台注入
4. `max_agent_hops` 循环保护——席位互相 `@` 到第 N 跳自动暂停，人类消息始终放行

第 4 条不是可选项。任何允许 agent 互相 `@` 的系统都会遇到失控链。

## 5. 明确不做

在施工顺序第 4 步之前，以下一律不做：

- workflow DAG / Transition Engine / fan-in / fan-out
- Gate 编排（第一道 gate 在第 4 步，此时才首次出现真实的不可逆动作）
- Context Pack 编译器（下一个席位读仓库在某个 commit 的状态即可）
- 「Owner `ACCEPTED` 前不得作为实现授权」「独立 fresh audit `PASS`」「contract-manifest digest 绑定」

## 6. 验收

**唯一规则：每一轮结束时，能跑起来的东西必须多了一项能力。以「某个契约被接受」结束的一轮，记为零。**

施工顺序（每步以能跑给人看为准）：

1. 拉起一个真的 harness，拿到输出
2. 一个任务从发起到结束，结果落进仓库
3. 两个席位同时跑，互相 `@`，不互相踩
4. 到此才装第一道 gate

## 附录：本文件依据的实测证据（2026-08-22）

| 事实 | 来源 |
|---|---|
| grok-worker 活跃：doctor 10/10，4 个 profile 全 active，累计 7,240 万 token / 51 次调用，当日四个账号全部使用过 | `grok-worker doctor` / `pool status` |
| grok-worker 共 4,954 行、42 commit、2 个工作日（07-21、08-20） | `git log` |
| Result Capsule 携带 `commitEvidence` `diffEvidence` `changedFiles` `baseCommit` `taskId` `grokSessionId` `invocationId` `requestId` `selectionEvidence` | 实测样本 |
| 报告正文被删、只留 `finalTextSha256` | 实测：`redaction.rawStreamDeleted = true` |
| 改为写文件后报告完整留存 40 KB / 331 行，`changedFilesFinalState` 仅含目标文件 | 实测第二次派发 |
| CAO 结论为 superseded 而非失败：双 worker 并发已被证明可行，停的是 WSL/tmux 产品化路线 | `CAO_REPORT.md` §4 |
| agentchattr 在 Windows 上是直接子进程，不需要 WSL / tmux / Docker / 管理员权限 | 官方 README + 本机 `config.toml` 实测 |
| 本机 Python 3.11.15 存在 | 实测，`CAO_REPORT.md` §6 预测的该项阻塞已解除 |
| agentchattr MIT，Copyright (c) 2026 Ben Curtis | `LICENSE` |
