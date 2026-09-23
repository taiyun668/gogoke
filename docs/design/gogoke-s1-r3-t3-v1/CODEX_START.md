# Owner → Codex 启动词（GitHub 固定方案版）

接管 Gogoke S1-R3 的整段代码施工。

仓库：taiyun668/gogo-party
方案分支：codex/gogoke-s1-r3-plan-v1
施工起始 commit：以 Owner 消息给出的固定 PLAN_COMMIT 为准。
Gogoke 停工输入：88ef8e7dfbf5ba5aef58743dc45fa660f946276e
生产/审计追溯基线：bc665a852833952b76d9508401193bedd2198436
donor：pingdotgg/t3code@d6f291303ddc0c9a14f570266a4d9eff6d431593

这是新的 S1-R3 授权入口。旧 S1 自建 journal 路线和旧 AionCore 首选包均不得恢复为施工指令。

先读取固定 PLAN_COMMIT 中：
START_HERE_GPT_S1_R3_HANDOFF.md
docs/design/gogoke-s1-r3-t3-v1/DECISION.md
docs/design/gogoke-s1-r3-t3-v1/ARCHITECTURE.md
docs/design/gogoke-s1-r3-t3-v1/PLAN.md
docs/design/gogoke-s1-r3-t3-v1/GATES.md
本文件。

再读取固定旧计划 34b7f891e4e17715b08491570740516d6cf7f49f 的 TASKS.json 与 TEST_MATRIX.json，只用于保全旧 19 tasks、68 主测试、157 截止、33 S1 当期检查和证据 ID。实现路线以本 S1-R3 为准。

先核实际 HEAD、dirty/untracked、活跃任务、进程 custody、模型/角色/工具/Windows 能力。保全用户成果，不 reset/clean，不按名称 kill。

从固定 PLAN_COMMIT 创建新的 execution branch（建议 codex/gogoke-s1-r3-t3-execution-r1），先登记 Owner 本条授权与 MACHINE_TASK_MAP，再执行 G0。G0 是实际资格验证，不得引用 GPT 研究结果直接写 PASS。

G0 通过依赖项后，按 G1→G4 和六 WP 连续施工。普通编译、debug、测试失败、已定方案内重构留在 Codex，不逐 WP 回 GPT，不逐 worker 询问 Owner。

架构已定：
- 一个 T3-derived 主服务；
- Gogoke Tauri/React 壳不替换；
- SQLite/event/receipt/orchestration 复用 donor 主体；
- Gogoke 产品 policy/Seat/domain/Owner/Delivery guarantee 在同一服务内实现；
- 薄 Rust native custody layer 负责 OS lock/Job/handle/private IPC；
- 不另建第二 Session/Execution/Delivery 权威；
- 不叠加 AionCore/OpenClaw/Herd/Pi/OMP 为第二主内核；
- 不把 native CLI 静默替换 API；
- 不继承 full-access 为默认产品策略。

所有 native/SDK/helper spawn 都必须纳入 launch/custody 审计；某 SDK 无法直接接统一 spawner 时，必须给出等价的 owning process handle/custody 证明，不能绕开。

施工每 3–5 个小项或约 3–5 分钟可保存工作写 micro checkpoint：HEAD、diff、dirty、source/build/plan/auth、命令/exit、失败/skip、活进程/custody、NEXT_ACTION。checkpoint 不是审批点。

团队按 docs/model-routing.md 实际核实；worker 不 commit/push；Controller 负责共享集成和提交。缺槽位排队，不外建 Codex 会话绕上限。

允许：限定分支源码/测试/文档；固定 donor 下载核验；独立工具目录；临时根；fake/受控子进程；私有本地 IPC；构建测试；Controller 限范围 commit/push；草稿证据 PR。

禁止：merge/auto-merge、force/reset/clean、真实账号/凭据/费用、复制 OAuth、真实 Home/数据迁移、外部监听/live daemon/Tailscale/SSH/云、麦克风/model 下载、全局安装、registry/update/signing/release、S2/S3。

只有证据要求改变主内核、事实权威、公共合同、权限/私域、原生身份、迁移边界或产品能力时，暂停受影响路径并把反证回 GPT。普通失败不触发重新全球选型。

缺 Windows/真实 native 环境时准确记 BLOCKED/NOT_RUN；不能用 mock 或 Linux 结果冒充对应 PASS。

不要只写接包回执就停。接包后立即 G0，并持续推进到 S1-R3 完成或真实架构级阻断。

最终回执必须列：最终 commit、修改文件、真实命令/exit、失败与修复、所有 S1 当期检查、独立最新候选审查、未执行轴、回滚/升级、residual custody。PR 不合并。阶段结束进入 S2 研究时回 GPT。
