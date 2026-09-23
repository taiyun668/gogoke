# GOGO PARTY 设计稿：施工操作系统与界面

| | |
|---|---|
| 日期 | 2026-09-11 |
| 地位 | 产品 + UI 的一份可读设计。实现细节与 PR 表仍以 `35-construction-os.md` 为准；界面细则以 `36-construction-os-ui.md` 为准 |
| 实现根 | `<gogo-party>`（`packages/room` + `packages/seat-runtime`） |
| 蓝图根 | `<local>\NaveHQ`（冻结，只读） |
| 原型 | `pnpm room:start` → `http://127.0.0.1:8330`（仍是旧群聊 UI，本设计尚未进代码） |
| 许可 | AGPL-3.0-or-later |

---

## 1. 产品

GOGO PARTY 是跑在本机上的**施工操作系统**。

一段目标一个 `conversationId`。你默认对**主控**说要做成什么；主控用 `@` 把活派给各家**官方 CLI** 席位。你也可以打开**审计**、**秘书**会话（问缺口、要摘要），共用这一段的锚点和对账。独立审计只读核对仓库。对账三栏齐了才能上报「还差什么」，卡不能变绿。仓库若有检查命令，**房间自己跑** `verify`。完成只有你在这段 CONTROL 上点验收。

第一份货物是内容电商。第一个产品是这台 OS：先在 GOGO 自己的仓库跑通闭环，再把工作区指到电商仓库。

**不是：** 对等群聊；Cindy 一个伙伴底下换引擎；Ekko 工作流画布；Grok Bot 云端电脑；NaveHQ 只能判断的指挥中枢。

---

## 2. 怎么来的（三套并排，选边）

| | 是什么 | 命运 |
|---|---|---|
| NaveHQ 蓝图 | Owner 只在两端；Plan / Delivery / Assurance；对账不能由派活者打分；闭环回流 | **冻代码**。留不变量 |
| GOGO 章程 00–08 | 指挥中心、Implementer + 独立 Auditor、证据才能完成、Transition Engine | **历史**。对象模型和 DAG 不进 runtime |
| 8 月 22 日房间 | 席位、`@`、git 信封、M7、halt、官方 CLI、隔离 HOME。M1–M8 闭合未 push | **唯一实现面**。群聊身份被本设计替换 |

调度仍然只有 `@`。不复活校验器、任务卡、Matching Layer。

---

## 3. 角色

角色是花名册上的策略，不是新对象。工人是 PATH 上已登录的 `codex` / `claude` / `grok`，隔离 HOME，凭据不进 GOGO。

| | 主控 | 施工 | 审计 | 秘书 |
|---|---|---|---|---|
| 权限 | 可写 | 可写 | 强制只读 | 强制只读 |
| 你怎么用 | 默认谈目标、看派出 | 派出卡 / Run dock / 插话 | 打开会话质询 | 摘要、提醒 |
| 无 `@` 新目标默认发给 | 是 | 否 | 否 | 否 |
| 写对账第三栏 | 否 | 否 | 是，且必须异厂商 | 否 |
| 算施工主体 | 否 | 本段最后一次实质净 diff | 否 | 否 |
| `@` 人类 | 否 | 否 | 否 | 否 |

一房间一个主控；秘书可无。第二份主控或秘书 → 409。主控不能当 M7 施工主体。跟审计说「过了」不会验收。

建议座位（用时再绑，不自动选厂商）：主控 GPT 20x Codex；施工 Plus Codex / SuperGrok；审计与施工不同家（常为 Claude 5x 或 Grok）；Gemini 不进房间。

---

## 4. 一段目标怎么走完

```
你 → 主控（说明目标）
        ↓ @
     施工改文件 → 房间署名 commit
        ↓ [对账] 或满 3 次实质 commit
     异厂商审计只读写缺口
     房间跑 verify（有命令才跑）
        ↓
     CONTROL：可上报距离，不涂绿
        ↓ 你点验收
     acceptedAt → 这段施工停
```

三个机器事实，都不靠模型自称：

1. **`reportable`**：锚点已锁 + 房间算的 git digest + 异厂商审计正文 + M7 SATISFIED。只表示「可以报还差多少」。
2. **`verify`**：房间执行仓库声明的检查。无声明则 `skipped`。失败则验收 409。
3. **`accepted`**：你点验收。之后 continue / 改向 / 再验收都 409。

自动对账仍是 3 次实质 commit / 30 文件 / 4 小时。一口收口走 `[对账]`，不必等满 3 次。

主控 `@` 施工之后你仍可对主控说话、可插施工，界面不得转圈锁死输入。

---

## 5. 从市面收进来的

对照过 Cindy、CodeFleet、Conductor、Claw Autoloop、orchflows、HAR、Claudexor、Claude Agent Teams、AgentsRoom、Ekko、Grok Bot 等。收机制，不收壳。

| 收 | 来自 | 放哪 |
|---|---|---|
| 派出 chip、「此刻」dock、本段派出清单、Files/Git 窄轨、托盘活动中心 | CodeFleet | 界面；托盘在 Control Host |
| 角色会话并排、引用另一席、颜色只表示在跑、主控不空等 | Cindy | 界面 |
| Diff 当一等审查面 | Conductor | 图标轨 |
| 房间自己跑检查，过不了不能验收 | Claw / orchflows / HAR | `verify` |
| 同厂商额度 next-up，对话粘实例，禁止跨成另一家 | Claudexor | 实例池 |
| 只为决定回来 | Grok Bot / 已有 halt | CONTROL 闩锁 |
| 席位互相 `@` | Claude Agent Teams 邮箱 | 已有 `@` |

不收：随包 CLI、工作流画布、NL 分类器、云端电脑工人、人当舰队司令的网格首页、用 LLM 宣称测试通过。

---

## 6. 界面

左轨选**这段目标**，舞台条选**跟谁说话**。中间是当前角色的会话。验收、对账始终属于左轨那项目标。

```
┌──────────────┬─────────────────────────────────────┬──────────┐
│ 项目 / 房间  │ 舞台条：主控 │ 审计 │ 秘书 │ 施工chip │ Files    │
│              │─────────────────────────────────────│ Git      │
│ 本段目标列表 │                                     │ Diff     │
│ （未验收置顶）│  当前角色 × 当前目标 的 thread        │ （图标轨 │
│              │  可撕第二列并排（主控|审计 或 秘书）   │  默认收）│
│ 本机 · 实例  │─────────────────────────────────────│          │
│              │ composer → 舞台条当前席              │          │
│              │ 状态：ready / 在跑（再发=插话）      │          │
│              │        / 闩锁 / 已验收（再发=新目标）│          │
│              │ [对账]  不自动点亮主控 chip          │          │
└──────────────┴─────────────────────────────────────┴──────────┘
施工在跑时：thread 旁 Run dock（此刻、插话）
```

| 点舞台条 | 中间是什么 | 输入 |
|---|---|---|
| 主控 | 目标、派出卡、diff 摘要、对账卡 | 默认；新目标也是它 |
| 审计 | 与审计的问答；对账卡钉在列顶 | 有，只读仓库 |
| 秘书 | 摘要、提醒 | 有，只读仓库 |
| 施工 | Run dock + 插话 | 不当开新目标 |

并排最多两列，一条 composer 属于焦点列。可把审计会话链贴进主控输入（引用）。工人不能 `@` 你。空白态：「要做成什么，跟主控说。」

视觉沿用现有 Codex 灰阶令牌。彩色只表示在跑。对账卡永远 CONTROL 灰。绿只出现在「已验收」条和左轨小点。

登录、装 CLI、换号：整页设置 · 实例，不在会话里。终端 dock 留下当诊断。

刷新后聚焦从服务器 `halt.conversations` 恢复：未验收恰好一段就钉住它。

---

## 7. 和现在页面的差

现在：中间群聊，人 `@` 调度，点席位是设置抽屉，每发一句新开 halt 窗口。

改为：默认发给主控并续同一个 `conversationId`；点角色换会话不换目标；抽屉不再当主控窗；对账/验收在 CONTROL；Diff 不靠终端。

设置整页、实例池、一席一实例、项目树：不动。

---

## 8. 原型与技术栈

| | |
|---|---|
| 仓库 | `<gogo-party>` |
| 启动 | `pnpm room:start` → `127.0.0.1:8330` |
| 房间 | Node 24 ESM，原生 `http`，无 Express；`server.ts` |
| 页面 | 单文件 `packages/room/public/index.html` |
| 席位 | `@gogo/seat-runtime`：Codex app-server、Grok ACP、Claude stream-json |
| Git | 席位不能写 `.git`；房间署名 commit |
| 停机 | `halt-conditions.ts`；`pnpm --filter @gogo/room test:halt` |
| 数据 | 盘符根 `gogo-party-room-data\`（隔离 HOME、配置、transcript） |
| 工作区 | 默认 `.gogo/room-workspace`；或 `GOGO_ROOM_WORKSPACE` |
| 不进房间 | Rust `crates/*`、NaveHQ、Gemini provider |

Windows 优先。GOGO 不保存 CLI 密钥。

---

## 9. 保留 / 不做

**保留：** 官方 CLI、隔离 HOME、`@`、git 信封、M7、halt +1、实例池、多项目、审计只读硬拒绝。

**不做：** NaveHQ 代码、Transition Engine、任务卡、分类器、Cindy 捆绑二进制、Ekko 源码、Grok App runtime、Windows Service、本阶段跨对话写锁、自动绑 Claude 5x、在本设计里开内容电商对象模型。

---

## 10. 已拍板

1. 工人不得 `@` 人。  
2. Claude 5x 用时再绑。  
3. 第一枪 GOGO 自己，第二枪内容电商。  
4. 关窗口 ≠ 退出（Control Host 排在主路径后）。  
5. 可与主控、审计、秘书对话，不只主控。

---

## 11. 怎么落地（摘要）

A 产品句换掉 START_HERE §2 → B 角色含主控/秘书 → C 施工主体 + 对账 + 验收 + 派出清单 → D 默认发给主控 → E 第一席即主控 → F 目标×角色 UI → G 假席位测闭环 → K 房间 verify → L 额度 next-up → 真 CLI 第一枪 → 电商仓库 → J Control Host 托盘 → M 施工 worktree（第一枪后）。

每步合入：`test:halt` A1–A9 全绿。不改 +1 公式。

---

## 12. 分文档

| 文件 | 用途 |
|---|---|
| 本文件 | 给人读的设计稿（产品 + 界面） |
| `35-construction-os.md` | 谓词、API、PR、测试、权威条款 |
| `36-construction-os-ui.md` | 界面块与交互矩阵 |
| `37-product-report.md` | 来源、路径、技术栈总览 |
| `START_HERE_CONTROLLER.md` §3 起 | 正在跑的房间 |
| `<local>\NaveHQ\docs\navehq_target_blueprint_v1.0.md` | 对账意图（只读） |
