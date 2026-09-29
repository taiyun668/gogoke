# 自家历史：返工、停工与根因（第二批 A）

- 日期：2026-09-25
- 性质：事实底稿，不评判、不给建议。
- 来源：本地五个仓库的 git 历史与文档，以及本仓库 S1-R4 的检查点与会话记录。
  - gogo-party：私有，已归档。
  - NaveHQ：私有，已停。
  - Sandglass：公开。
  - Grok Worker Provider：私有。
  - gogoke：公开。
- 数字均由 Claude 于 2026-09-25 从 git 直接统计。
- 提交分类按提交信息关键词粗分，只作量级参考。

---

## 1. 五个项目的量级对照

| 项目 | 时间跨度 | 提交数 | 结果 | 提交信息关键词分布 |
|---|---|---:|---|---|
| NaveHQ | 2026-06-25 → 08-21（约 8 周） | 1,116 | **停止，无可运行产品** | repair/fix 168、audit/review 156、gate/fuse/guard 104、rebind/rebaseline/vN 76、feat 52 |
| gogo-party | 2026-08-11 → 09-22（约 6 周） | 1,957 | 归档，源码迁入 gogoke | 08-11~20：治理类 410、实现类 80；08-21~31：治理 139、实现 118；09 月实现类上升 |
| Sandglass | 2026-08-28 → 09-11（**2 周**） | 540 | **发布 0.1.0 → 0.1.11，11 个版本** | repair/fix 141、feat 50、audit 26、gate 21、rebind 2 |
| Grok Worker Provider | 2026-07-21 → 09-21 | 146 | 在用，每天跑 | repair/fix 58、release/candidate 18、audit 16、feat 6；本地留有 17 份 r8 候选或发布副本 |
| gogoke（S1-R4 起） | 2026-09-19 → 进行中 | 执行分支 170+ | R2-01 到 R2-05 已完成，R2-06a 进行中 | 见 §5 |

---

## 2. NaveHQ：只治理、不交付

- **现状文件原话**（`docs/navehq_current_status.md`）：
  - "Stage B changes governance documents only. It does not implement Plan/Delivery runtime, mode-level schema/fixture/validator, automatic scheduling…"
  - "Accepted architecture is not implemented runtime."
- **文档数量**：`docs/` 下 271 份，大量带版本号，如 `_v0.1`、`_v0.2`、`_v3.2`。
- **序号化施工**：每一步都是一个编号序列，已知至少到 seq 75。本地留有几十个序列工作目录，名字里反复出现：
  - repair：direct_main_repair 两次、direct_main_invalid_repair、image_anchor_repair、qualification_wiring_repair、sandbox_mount_repair；
  - rebind / 版本迭代：mhg v2.1 → v3 rebind → v4 → v5；
  - 检测机制：hard gate、destructive change gate、audit no progress fuse。
- **每一步都要 Owner 出手**：Owner accepted 回执 + 单独的 merge authorization 回执 + Controller gate 回执 + Runner 回执，逐项记在现状文件里。
- **提交信息里的版本号**：v5 出现 24 次、v6 14 次、v4 10 次；序列 16 走到过 v13（"close sequence 16 post-v13 audit blockers"）。
- **最后的提交**（2026-08-21）："bad"、"noop"、"TEMP"、"x"，以及"restore staged Card B exact bytes after controller incident"。

---

## 3. gogo-party：验证器比产品多

来自 `START_HERE_CONTROLLER.md` §7 自身记录的"历史实测值"（2026-08-22）：

- 当日 47 个提交：governance 29、docs 8、plan 7、audit 1，**实现类 0**。
- `gogo-core` 的 `TransitionEngine` 有 5,098 行，全部公开能力只是一个 `decide()` 加七个 `validate_*()`；另有 5,108 行测试，测的是这些检查器。
- 25,166 行 Rust，本机没有 cargo / rustc，**从未编译过一次**。
- 可执行产品行数：0。
- 216 篇设计文档描述的能力，大多已由 Grok Worker Provider 实现并在日常运行，重读只会重新推导已经存在的东西（同文件 §8）。
- 同文件废除了三条治理规则，理由是"目前没有保护对象"：
  - "Owner ACCEPTED 前不得作为实现授权"
  - "独立 fresh audit PASS"
  - "contract-manifest digest 绑定"
- 治理绑定的版本号曾到 v30（2026-08-25："bind P32 registry digest to v30"）。
- 设计模型有 v3 重写（2026-08-23）；解耦计划有 v1、v2、v3（2026-09-18）。

**同文件记录的实测坑（2026-08-22）：**
1. 改了 `src` 忘了重新构建，而运行时加载的是 `dist`。"这个坑吃掉过三轮。"
2. 沙箱参数写错时**静默回落到只读**，不报错。
3. 只看数字不看机制：配额走势 5 → 4 → 5 被当成"回满"，实际是有界 +1 恰好撞上上限。
4. 两轮异构审计各抓到一处设计者自己看不见的问题。

---

## 4. Sandglass：两周十一个版本

- 540 个提交，发布 0.1.0 → 0.1.11。
- gate / rebind 类提交很少（21 / 2）。
- 安装、更新、卸载整条链在 09-07 到 09-11 做完：NSIS 脚本 24 次提交，更新实测 11 次。
- 做法：
  - 不签名，接受智能应用控制可能拦截，并在 README 里如实写明；
  - 主程序自己负责卸载；
  - 由 Owner 签名的清单保证更新的完整性。
- 形态：单人自用工具，Python 源码直接运行，边界清楚。

---

## 5. gogoke S1-R4（本仓库，2026-09-19 起）

事件均记录在执行分支的检查点（MC-*）与本仓库文档中。

| 事件 | 现象 | 当时查明的根因 |
|---|---|---|
| S1-R4 v1 → v2 | v1 冻结约 3 天后停工，重写为 v2 | 范围过大；改为最短纵向闭环 |
| 迁到公开仓库时漏东西 | 模型路由、`.codex` 配置、设计 00–36、研究和治理文件没有随源码迁移 | 迁移范围定成了"施工需要什么"；迁移前复核只查一致性和泄露，没查完整性 |
| Codex 用旧模型 | 施工一直调用 gpt-5.6 | 公开仓库缺少路由配置，回退到了旧仓库的默认 |
| 已做的决定丢失或被违反 | 账本分工、构建约束、签名的约束被重新加回来 | 没有一处可以对照 Owner 已做过的决定 |
| R2-04 排错四轮 | 安装后服务退出码为 1，连续四轮云端构建都没找到原因 | 通过有意隐藏 stderr 的产品通道分类错误；对照实验在失败之后才运行，没有拿到失败现场。改为一轮内直接取失败现场的原始输出后，约 1 小时定位（Windows `\\?\` 路径前缀导致 Node 无法加载） |
| 智能应用控制拦截 | 041a4966 的安装程序和便携版 `gogoke.exe` 都被拦，985e17d1 的同一流程则放行 | 每个新版本都产生新的 exe 字节，智能应用控制逐文件判定；由此催生 R2-06 |
| R2-06 耗时 | 约 15 小时后仍未出候选 | 安装实测和签名放在 main 的同一个 workflow 里，每次排查都要合进 main（#38–#43）；签名 workflow 前后做了 9 次全轴审计；主控开在最高档，占了全部输出的 57% |
| 审计循环 | 每一次"最终全轴审计"都找出 1–3 个新 P2，修完又从头审 | 没有收敛条件 |
| 夸大的估时 | "半天"变成超过一天 | 低估了各自独立的失败点数量，每个都要一轮 30–60 分钟的云端往返 |

---

## 6. 跨项目反复出现的现象（事实归纳，不含建议）

- **治理与验证先于产品，并且自我增殖。** NaveHQ 的 fuse、gate、hard gate 系列；gogo-party 的验证器比产品多、Rust 从未编译；两者都走到过 v5–v30 级别的版本号。gogo-party 的漂移检测表正是从这段历史里总结出来的（"开始建造用于检测问题的机制""校验器行数超过产品行数"）。
- **Owner 触点密集。** NaveHQ 每个序列都要 Owner 给 accepted 和 merge 两次授权；gogo-party 早期同样如此，后来废除了三条。
- **能交付的项目，形态更简单、边界更清楚。** Sandglass 与 Grok Worker Provider 都在用。Sandglass 两周发了十一个版本，gate 类提交最少。
- **迁移和交接会丢失东西。** gogoke 迁库漏掉路由与治理文件；NaveHQ 的"唯一事实源"现状文件长期累积，接力靠重读。
- **排错会绕着症状加层。** R2-04 通过隐藏信息的通道分类错误；gogo-party 忘了重新构建，吃掉三轮。两次都是先拿到一手证据才找到根因。
- **版本与候选的副本堆积。** 本地盘上 NaveHQ 有几十个序列或版本工作目录，Grok Worker Provider 有 17 份 r8 候选或发布副本。
