# 审计：Codex 去专属化与公共能力内化施工计划 V1.0

- 被审：`taiyun668/gogo-party@2fafb91`，分支 `codex/gogoke-shell`，`docs/design/gogoke-codex-decoupling-plan-v1.md`（582 行）
- 审计：Claude（Opus 5），2026-09-18
- 方法：两边扫。计划逐节读完；本体按 `apps/desktop/src`（25 feature）、`apps/desktop/src-tauri/src`、`packages/` 逐模块核对。计划的 S01–S20 依据抽验 6 条。

---

## 结论

**方法对，基线真，能签的部分可以签。** 五种处理、"不许用弱实现冒充等价替换"、§9 的七个故障注入点、§12 的六问删除准则 —— 这些不是套话，是能挡住事故的条款。S11／S12／S10／S14／S15／S16 六条依据我逐条回源核对，**全部属实**。

**但有五条必须改，其中第一条会决定整个计划的工作量。**

---

## 一、致命：计划没有看见同一分支上已经写好的 5,249 行归一层

计划全文 **0 次**提到 `packages/seat-runtime`、`packages/room`、`seat`、`ACP`。

而 `packages/seat-runtime` **就在被审的那个分支上**（`codex/gogoke-shell`，不在 main 上）：

```
git ls-tree --name-only 2fafb91 packages/
→ packages/protocol  packages/room  packages/seat-runtime
```

它是 **5,249 行**，零运行时依赖，不 import 房间：

| 文件 | 行 | 是什么 |
|---|---|---|
| `seat-runtime.ts` | 1413 | 进程、隔离环境、生命周期 |
| `grok-acp-seat.ts` | 1000 | Grok，ACP over stdio |
| `claude-seat.ts` | 676 | Claude Code，`--input-format stream-json` |
| `seat.ts` | 288 | **`Seat` 接口 + `SeatEvent` 八种事件** |
| `close.ts` | 275 | `pid + processStartTicks` 进程身份 |
| `seat-runtime.test.ts` / `isolation.test.ts` / `close.test.ts` | 898 | 测试 |
| `test-fixtures/*.mjs` | 262 | **五个假 CLI，三种协议形状**（codex app-server、claude stream-json、grok acp、两种 native-input） |

对照计划要新建的东西：

| 计划要造的 | 已经存在的 | 位置 |
|---|---|---|
| WP01 `NativeBinding` / `PublicEvent` / `CapabilityDescriptor` | `Seat` 接口 + 八种 `SeatEvent` | `seat.ts:216`、`:251` |
| WP03「可独立启用/停用的 CodexDriver」 | 三个 Seat 实现，本来就是可插拔的 | `claude-seat.ts` / `grok-acp-seat.ts` / codex |
| WP11「两个不同协议形状的测试驱动」 | 已有五个假 CLI，三种形状 | `test-fixtures/` |
| T21 权限不等价 / T23 凭据卫生 | **已实现且已有断言** | `isolation.test.ts` |
| WP03 家目录隔离 | `buildIsolatedEnv`，四维改写 + 密钥/端点剥离 | `seat-runtime.ts` |

`isolation.test.ts` 开头那段注释，正好是计划 §11 反复警告的那件事，已经被踩过并写成了判据：

> 原来的验法是「空目录必须报未登录」，而它对 Grok 无效 —— Grok 只认 `GROK_HOME`，那一维恰好被设对了，于是 HOME 是真的、密钥没滤、沙箱能从环境放开，测试照样全绿。**判据挑中了唯一被满足的维度。**

**为什么会漏**：计划 §0 把施工范围划成「主要施工范围为 `apps/desktop/`」（第 11 行）。这一刀把 `packages/` 划在外面了。但同一段又声明"全量"覆盖整个产品能力面。**范围声明和全量声明互相矛盾，代价是最贵的那块资产没进视野。**

**要改成**：WP00 必须把 `packages/seat-runtime` 与 `packages/room/src` 下十三个零 `server.ts` 依赖的模块（约 4,700 行，含 `instances.ts` 的 `InstanceProvider = "claude"|"codex"|"grok"`，正是 WP09 的实例模型）纳入基线扫描，逐条判"复用／改写／不用"。**不用可以，但要写出理由**（例如 TypeScript 在 `packages/` 而宿主执行要落在 Rust daemon —— 那是一条真实的理由，但它需要被说出来，而不是默认不存在）。

---

## 二、首版是五家，计划只写了三家

计划第 170 行：「**三家**目标接入地位对等」；第 336 行：「Codex/**Claude/Grok** 的目标地位对等……再接通**至少一个**真实非 Codex CLI」。

Owner 2026-09-17 的裁决是**五家**：Claude Code、Codex、OpenCode、Grok Build CLI、Antigravity CLI。

计划里 `OpenCode` 0 次、`Antigravity` 0 次、`agy` 0 次。

这不只是数字差。OpenCode 和 Antigravity 是**形状最不一样的两家** —— Antigravity 的无头 CLI `agy` 很可能一次调用一个进程（本机未装，未跑过 `agy --help`，这条结论我自己也标了不可靠）。如果归一层只按 Codex/Claude/Grok 三家的形状定型，第四第五家接进来时改的就是公共合同本身，而公共合同一动，WP01 之后的全部工作包都要回炉。

**要改成**：WP01 定合同前，先把五家的形状摸齐；WP11 的验收从"至少一个真实非 Codex CLI"提到"五家各自的真实调用记录，未装的明确列为未验证"。

---

## 三、全量扫描漏了一个 1,873 行的能力族：远程访问

计划 `tailscale` **0 次命中**。而本体里：

```
src-tauri/src/tailscale/  1,873 行（mod.rs 685 / core.rs / daemon_commands.rs / rpc_client.rs 291）
src-tauri/src/remote_backend/  515 行
前端：SettingsServerSection、useSettingsServerSection、MobileServerSetupWizard、
      useMobileServerSetup、types.ts、i18n、tauri.ts
```

C01–C40 四十个能力族里没有它。WP10 叫「贯通 daemon、**远程**、移动和恢复」，落点列的是 `daemon rpc/*、remote_backend、events、mobile、tray、daemonctl/退出路径` —— **`tailscale` 不在落点里**。设备配对状态、授权、主机名这些持久数据，§9.1 的迁移盘点表里也没有对应行。

**同时这里有一条和 Owner 裁决正面冲突的条款。** 计划第 328 行：

> 既有已承诺远程能力**不能通过永久关闭来完成"保留"**。

Owner 2026-09-17 裁决：**远程访问暂时先关掉**（封入口不删码）。

这条冲突要 Owner 定，不能由任何一边自己改。我的建议：计划这一句的本意是防"用禁用按钮冒充重构成果"，Owner 的裁决是"首版不做这一摊"。两者可以并存 —— 改成"关闭必须是显式的产品决定并记录在案，代码与数据不删，恢复路径可验证"，冲突就解掉了。

**要改成**：新增 C41 远程访问与设备配对，落点写明 `tailscale/*`；§9.1 加一行配对状态/设备授权的迁移；第 328 行按上面改。

---

## 四、四处施工断链

### 4.1 WP08 ↔ WP11 循环依赖（真断链）

- WP08 依赖 WP03/WP04（第 290 行）
- WP08 通过条件：「**生成能力通过真实非 Codex 调用验证**，手填兜底不冒称生成通过」（第 298 行）
- 真实非 Codex 接入是 **WP11** 的交付物
- WP11 依赖 **WP08**（第 332 行）

**WP08 过不了，除非 WP11 先跑；WP11 跑不了，除非 WP08 先完。** C31（提交信息/运行元数据/Agent 描述生成）卡在同一个环里。

**要改成**：把 WP11 拆成 WP11a（最小真实非 Codex 通路，只要能发一条、收一条）和 WP11b（完整矩阵）。WP08 依赖 WP11a，WP11b 依赖 WP08。

### 4.2 WP04 需要能力探测，而探测归 WP09，两者之间没有边

- WP04 通过条件含「stale turn 或不支持 steer 不自动退成另一动作」（T15/C13）
- 要判断"支不支持 steer"，靠的是 `CapabilityDescriptor`
- WP01 只**定义类型**；**填值**的是 WP09：「能力以目标实例和版本探测为准」
- WP04 依赖 WP02/WP03；WP09 依赖 WP01/WP02/WP03。**两者平行，谁先谁后没定。**

结果是 WP04 的核心验收（队列 vs 插话不能互相冒充）没有数据来源，两个工人各自以为对方会做。

**要改成**：WP04 依赖里补 WP09 的能力探测部分，或把探测切一小块进 WP03（实例起来时就探）。

### 4.3 隐私隔离被劈成两半，而 Owner 裁决的那个方向一个测试都没有

- C38（权限/数据隔离）→ WP03/WP04/WP07
- C35（通知 payload）→ WP05/WP10
- T11 私聊边界同时压在 C08/C35/C38 上
- **WP05 依赖 WP04；WP07 依赖 WP01/WP02/WP04。两者平行，没有边。**

更要紧的是方向。T11（第 444 行）写的是：

> **审计私人侧聊**不自动出现在项目公开流、通知正文或其他会话缓存

这是**从侧聊往外漏**。Owner 2026-09-17 的裁决是另一个方向：

> **项目施工角色或者席位不应该看到我和主控的对话内容**

这是**从主对话往下漏**。计划里没有任何一条测试覆盖这个方向。

改造后主控派活是 **CLI 派活给 CLI**（Owner 2026-09-17），两个独立进程两份独立会话 —— 隔离不再靠渲染层过滤，而是靠派活时到底往下游进程喂了什么。这就是必须有一条实测的理由：**在主对话里只说一句话，然后问施工席位；它答得上来，隔离就是假的。**

**要改成**：新增 T49「派工入口隔离」，覆盖下行方向；指定一个工作包owner（建议 WP04，因为喂什么给下游是投递环节决定的）。

### 4.4 四十八个测试场景族没有归属工作包

§11 的 T01–T48 只映射到 C，不映射到 WP。但 §13 要求每包回执带 `test_ids`。

于是"T02 归谁跑"要走两跳：T02 → C01/C20/C23/C37 → WP05/WP08/WP03/WP09/WP07/WP02/WP12 —— **七个工作包，没有主责**。计划自己的验收门，找不到人签。

**要改成**：T 表加一列 owner WP。

---

## 五、"误删"这一问，用这份文件答不了

Owner 让我重点查"有没有误删应该内化的能力"。我的回答是：**现在判不了，而且这本身是个问题。**

§3 定了五种处理，第五种是**移除**（第 62 行），写了删除条件。但 §5 的四十个能力族里，**用"移除"的是 0 个**：

```
内化 11 / 内化＋X 9 / 保留 3 / 保留＋X 6 / 替换 2 / 替换＋适配保留 2 / …
移除 0
```

所有删除决定都被推到 WP13 和 WP00 的机器账本 —— **而那份账本还不存在**。

这不是坏事（保守是对的），但要说清后果：**这份文件不能被审"误删"，只能被审"删除的闸门够不够严"。** 闸门本身写得是够的（§12 六问 + 六个同时成立条件 + §12 末尾"明确不做的破坏性清理"）。

真正的风险因此转移到了 WP00 的账本质量上。这里有一个**已经埋好的陷阱**，计划没有点出来：

```rust
// src-tauri/src/codex/home.rs:6
pub(crate) fn resolve_workspace_codex_home(
    _entry: &WorkspaceEntry,        // ← 没用
    _parent_entry: Option<&WorkspaceEntry>,  // ← 没用
) -> Option<PathBuf> {
    resolve_default_codex_home()    // ← 直接回默认家目录
}
```

**这个函数长着"按工作区隔离家目录"的签名，行为上一点也不隔离。** WP00 如果照签名登记，会把一个不存在的能力记成"已有"，而 WP09 的"同账号多开、隔离运行配置"正好建在它上面。

§5 的表头写着"缺项不冒称既有"（C07）—— 这就是那类项，只是它伪装得比缺项更好。**建议 WP00 的账本加一列：证据是签名还是运行结果。**

---

## 六、"改名代替解耦"：防住了，但有一处要补

计划在四个地方明文防这个（§3 末、§8 S2/S5 行、§12、§14），防得比我预期严。WP01 还给了机械判据：「新增小型依赖边界检查，识别导入、命令注册和运行入口，**不以 grep 零 codex 为目标**」（第 196 行）—— 这是对的做法。

补两点：

**1. 这个依赖边界检查要从 WP01 起进 CI，计划没说。** 只在 WP01 建、到 WP13 才对账，中间十一个包没有机械约束。

**2. 本体里已经有一个"改名式检查"，别让它冒充成果。** `apps/desktop/scripts/check-product-identity.mjs` 干的就是禁止旧产品名字符串（`CodexMonitor`、旧仓库地址）。计划 §11.1 把 `npm run check:product-identity` 列为可用的既有测试 —— 它可用，但它正好是计划自己在 §8 S8 行点名的那种假完成（"grep 零命中就宣布完成"）。**WP13 的对账不能算上它。**

顺带：本体还有 `npm run lint:ds` 和一套 `codemod:ds`（design-system 的 lint 与迁移脚本），加上 `src/features/design-system/` 的五个 primitive（ModalShell / Panel / Popover / Settings / Toast）。**T12「视觉交互保全」的机械抓手已经在仓库里了，计划没用上。** 研究里"先统一组件语言，才能避免每次加功能就长出一种新的确认框"那条，落点就是这里。

---

## 七、数据迁移：§9 写得好，缺四行

七个故障注入点、切换后回滚要先保全新数据、旧 exe 不许开新库、"读失败则返回空对象"被点名禁止 —— 这一节可以直接签。

盘点表（§9.1）缺四行：

| 缺的数据 | 为什么要 |
|---|---|
| **远程/设备配对状态** | tailscale 的设备授权、主机名。见第三节 |
| **语音本地模型文件** | `dictation` 下载的权重可能上 GB。C36 说保留，迁移/回滚没说落在哪、要不要搬、回滚会不会重下 |
| **gogoke 自更新状态与更新主机白名单** | `gogoke_update.rs` 775 行。C39 说保留信任根，§9.1 没有对应行 |
| **新的 gogoke 数据根到底在哪** | C23/C32/WP07 反复说"转到 gogoke 数据目录"，全文没有一处给出路径、平台差异，也没说桌面与 daemon 是否共用。迁移的目的地未定义，§9.2 的"切换唯一所有者"就没有落点 |

另：`P0` 这个词在第 210 行和第 539 行出现两次，全文没有定义（WP00 从不叫 P0）。两处都在讲职责，建议统一成 WP00。

---

## 八、可以直接签的部分

不全是问题，下面这些我核过，站得住：

- **§2 的十一条耦合事实**：抽验 6 条（S10/S11/S12/S14/S15/S16）全部属实。`files_core.rs:34` 的 `FileScope::Global => resolve_default_codex_home()`、`prompts_core.rs:8` 混用 codex home 与 app_data、`state.rs:35` 的 `Mutex<HashMap<String, Arc<codex::WorkspaceSession>>>`、生成三件套确实落在 `rpc/codex.rs` 与 `codex_aux_core`、六个 npm 脚本确实存在 —— 一条没错。
- **§6.1**：把"RPC 接受停止请求"和"实际停止"分开，响应丢失记"接受结果未知/待核对"而不是自动重发。这是对的，而且和研究 U11 完全一致。
- **§6.2**：快照续聊 ≠ 原生 fork；界面折叠 / 摘要 / 实际 compact 是三件事。对。
- **§6.4**：不许为了对齐而删成最低共同集。这条正是 Owner 立的"尽量先不砍功能"。
- **§12 的六问**和末尾"明确不做的破坏性清理"（不删 `.codex`、不卸官方 CLI、不清工作区）。签。
- **C36** 特意点出"Whisper 不因出自 OpenAI 就误删"、**C40** 点出"保留开发用 AGENTS/.codex" —— 这两条是真的想过，不是凑数。

---

## 九、给 Codex 的六件事

1. 把 `packages/seat-runtime`（5,249 行，就在你的基线分支上）和 `packages/room/src` 十三个零依赖模块纳入 WP00，逐条判复用还是改写。**不用可以，要写理由。**
2. 三家改五家：补 OpenCode 与 Antigravity，WP01 定合同前先摸形状。
3. 补 C41 远程访问（`tailscale/*` 1,873 行）；第 328 行与 Owner 的"远程暂时关掉"冲突，按第三节改法解。
4. WP08↔WP11 拆环；WP04 补能力探测依赖；隐私隔离补下行方向的测试（T49）并指定 owner；T 表加 owner 列。
5. §9.1 补四行（远程配对／语音模型／更新状态／**新数据根的路径本身**）；`P0` 统一成 WP00。
6. WP00 账本加一列"证据来自签名还是运行"，`resolve_workspace_codex_home` 是现成的反例。

---

## 附：核对方式

| 结论 | 怎么核的 |
|---|---|
| seat-runtime 在被审分支上 | `git ls-tree --name-only 2fafb91 packages/` |
| 5,249 行 | 逐文件 `git show \| wc -l` 求和 |
| 计划 0 次提到 | `grep -c` 于 seat-runtime／packages/／ACP／OpenCode／Antigravity／agy／tailscale |
| tailscale 1,873 行 | `wc -l src-tauri/src/tailscale/*.rs` |
| 40 族无一"移除" | 抽 C 表第 3 列 `sort \| uniq -c` |
| 六条依据属实 | 逐条回源 grep，见第八节 |
| 六个 npm 脚本存在 | 读 `apps/desktop/package.json` |
| `resolve_workspace_codex_home` 是空壳 | 读 `src-tauri/src/codex/home.rs:6-11` |
