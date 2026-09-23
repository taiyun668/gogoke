# 交给 Codex 审计：gogoke 整合设计

- 出稿：Claude（Opus 5），2026-09-17
- 分支：`claude/gogoke-site-cleanup`，**71 个提交，未合并进 main**（远端 main 在 `85a02d3`）
- **这批文档只在这个分支上，main 上没有——不要拿 main 的内容对照**

---

## 一、先读哪几份

| 顺序 | 文件 | 行数 | 是什么 |
|---|---|---|---|
| 1 | [`GOGO-要做的和不做的.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-要做的和不做的.md) | 286 | **入口。** gogoke 已有什么、**已经砍掉什么（带日期）**、后端要做什么、组件库在哪 |
| 2 | [`GOGO-整合设计.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-整合设计.md) | 602 | **被审主体**，十七节 |
| 3 | [`GOGO-整合设计-取舍与依据.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-整合设计-取舍与依据.md) | 260 | 每一条从哪来、为什么用、为什么不用。**含对 V0.12 二十四节的逐节去向** |
| 4 | [`GOGO-V012审计.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-V012审计.md) | 148 | 对 `gogoke_design_v0_12.md` 的审计：收十九节、降级两节、重做三节、不收七节 |
| 5 | [`GOGO-本体互审.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-本体互审.md) | 97 | 设计 × gogoke 本体（约 28,000 行 Rust + 72,000 行前端）的互审 |
| 6 | [`GOGO-研究重读-缺口.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-研究重读-缺口.md) | 179 | 五份研究重读后的缺口，与已被 Owner 裁决覆盖的条目 |

**支撑材料**

- [`GOGO-要什么.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-要什么.md) —— Owner 口述的产品陈述
- [`GOGO-拆块清单.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/GOGO-拆块清单.md) —— 19 块
- [`docs/research/gogo-ui-2026-09-13/`](https://github.com/taiyun668/gogo-party/tree/claude/gogoke-site-cleanup/docs/research/gogo-ui-2026-09-13) —— 七份研究，字节未改；**U01–U18、M01–M22 的定义在 `研究覆盖与缺口.md`**
- [`docs/design/demos/`](https://github.com/taiyun668/gogo-party/tree/claude/gogoke-site-cleanup/docs/design/demos) —— 工作树演示（八版，Owner 逐版看过）与组件库展示面

---

## 二、按四条判据审

1. **对 gogoke 本体是否成立。** `apps/desktop/src-tauri/src`（约 28,000 行）+ `apps/desktop/src`（约 72,000 行）。设计说「已有」的是不是真有，说「缺」的是不是真缺。
2. **对 V0.12 是否公允。** 那是你的稿子——**特别请反驳我降级的两条**，见第三节。
3. **内部是否自洽。** 我已自查出并修掉两处打架（切项目停不停、席位常不常驻）。可能还有。
4. **Owner 的裁决有没有被违反。** 都带日期记在入口文档里。

**产出格式**：按「哪一条不成立 / 证据是什么 / 建议改成什么」逐条给，不要整体评价。

---

## 三、请重点反驳的两条（我对你的稿子做了降级）

### B-1　§11.1 Managed-Tools 降级

我把「关闭三家内建工具 + 自建工具网关 + LPAC ToolWorker」降成：**收口清单 + spawn 前逐条断言 + 一个钩子 + 路径围栏**。

依据：[`docs/design/22-seat-runtime-inheritance-ledger.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/22-seat-runtime-inheritance-ledger.md) —— 那份记录的实现本体在**本机 `<local>\Grok Worker Provider`，你（网页版）读不到**。请按该文档里的转述判断，它带文件与行号。

要点：启动契约当数据、spawn 前断言 `terminalDenied` / `subagentsDenied` / `compatHooksDisabled` 等；两级家目录（持久装认证、临时装 HOME）——**这条正好解掉你 §5.2 自己点出的死结：隔离 `CODEX_HOME` 就丢登录态**；硬拒默认家目录；路径围栏；项目权威预检；剩余边界一个 35 行的钩子。

**如果你认为网关不可降级，请指出：上面六样在哪种攻击面下不够。**

### B-2　§6.2／§8.4 步骤隔离取消

我把「一个进程只服务一个 NativeStep」改成：**席位常驻 + 不可复用的进程身份（`pid + processStartTicks`）+ terminal receipt fence**。

依据两条：

- `22-seat-runtime-inheritance-ledger.md` §4.1／§4.2 —— 本项目已为反方向付过钱：一次性进程**中途插话纠偏不可能、上下文不累积**
- [`docs/research/adapter-spike/01-capability-evidence.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/research/adapter-spike/01-capability-evidence.md) —— 你的方案靠官方 resume 接回上下文，而该表记 Codex `thread/resume|fork|archive` 为 **SUPPORTED / SOURCE，「仍需动态隔离测试」**；未 PASS 清单第三条正是「进程崩溃、Controller 重启、协议断线后的 resume/reconcile」

**如果你坚持步骤隔离，请给出 resume 的动态实测，或指出进程身份 + receipt fence 在哪种时序下仍会把旧帧算到新回合。**

---

## 四、我知道自己最没底的四处，请重点看

1. **§8.5 会话管理** —— 新写的，**没有任何人看过**。改造后主控派活不是 Codex 的 sub-agent，是 **CLI 派活给 CLI**（Owner 2026-09-17），两个独立进程两份独立会话；隔离全落在这一节。
2. **归一层我说「已经有」** —— `packages/seat-runtime/src/seat.ts` 的 `Seat` 接口与八种 `SeatEvent`（`turn-started` / `delivery-received` / `assistant-delta` / `tool-activity` / `turn-completed` / `cli-input-requested` / `cli-input-resolved` / `error`）。**首版五家（Claude Code / Codex / OpenCode / Grok Build / Antigravity）够不够用？OpenCode 与 agy 接进来要不要扩？**
3. **Antigravity 的判断可能是错的** —— 我说 `agy` 一次调用一个进程、只能当一次性席位，**依据只是 AionUi 的源码注释；本机没装 agy，没跑过 `agy --help`**。而 [`32-gemini-adaptation.md`](https://github.com/taiyun668/gogo-party/blob/claude/gogoke-site-cleanup/docs/design/32-gemini-adaptation.md) 记过一次同类教训：官方文档漏掉 `--acp`，照文档做会得出「只能一次性调用、做不了持久席位」的错误结论，**`--help` 才是权威**。
4. **§7 事件映射刚整张重写** —— 从 Codex 的 `ConversationItem` 换成 `SeatEvent`，**改完没复核过**。

---

## 五、三条硬边界，不要在审计里重新提出

1. **不要把 `packages/room` 当后端。** gogoke 与房间底层不是一个东西（Owner 2026-09-16），**不存在「接通」**。
   但 `packages/seat-runtime`（4,987 行）与 `packages/room/src` 下**十三个零依赖 `server.ts`** 的模块（约 4,700 行）是**可以直接搬的代码**——搬代码不是接后端。
2. **不要把 Codex 协议的形状当成 gogoke 的产品能力。** `apps/desktop/src-tauri/src/` 里 claude／anthropic／grok／xai／gemini **零引用**；它有的是渲染层，产品能力为零。
3. **砍掉清单上的东西不要再提**（每条带日期与原因，在入口文档第三节）：看板／归栏五列、成果 tab、交付清单页、**验收动作**、闸门、`branchId`、送达七态、独立性标记、**「一席一实例」**、**远程访问与语音输入（暂时关闭，封入口不删码）**。

---

## 六、你（网页版）读不到的东西

- **本机路径一律读不到**：`<local>\Grok Worker Provider`（工具收口六样的生产实现）、`<local>\gogo-ui-research`（研究原件，已搬进仓库）、本机安装的五家 CLI。
- **本机实测的结论**在文档里都标了出处与日期，例如：Codex `0.149.0`、Claude Code `2.1.196`、OpenCode `1.17.18`、Grok `1.0.30`、Antigravity IDE `1.107.0`（其无头 CLI `agy` **未安装**）。
- 涉及这些的结论，**请按文档里的转述判断，或指出哪一条需要补实测**——不要当作已验证。
