# S1：Owner 已授权的 Codex 连续施工任务

本轮唯一主任务：执行已由 GPT 制定、Owner 明确批准的完整 S1，不重新研究架构。授权原文、范围与禁止项见同目录 OWNER_AUTHORIZATION.json，ID=S1-OWNER-AUTH-001；该记录不是数字签名，也不声称任务已经运行。

## 固定输入与新状态

- 仓库 taiyun668/gogo-party；固定计划 `34b7f891e4e17715b08491570740516d6cf7f49f`，目录 docs/design/gogoke-s1-plan-v1/。
- 生产源码 `bc665a852833952b76d9508401193bedd2198436`；当前授权分支 `codex/gogoke-s1-execution-r1` 的初始提交只增加授权/交接，没有实施产品功能。
- P00=ACCEPTED_FOR_CODE_ENTRY（仅施工前）；gate=OPEN_FOR_AUTHORIZED_S1_ONLY。
- 已授权 WP10a、WP01、WP02、WP03、WP09a、WP04，以及计划限定测试、Controller commit/push。PR 合并和真实业务副作用没有授权。
- 固定计划中 PENDING/NOT_DISPATCHED 是制定时历史状态，由此较新的 Owner 授权覆盖“是否有施工许可”一项；不改固定计划的语义、测试或历史验证结果。

## 开始执行，不再请求同一份范围授权

先读 OWNER_AUTHORIZATION.json，再完整读固定 PLAN.md、CONTRACTS.md、TASKS.json、TEST_MATRIX.json、SOURCE_MANIFEST.json 和 CODEX_HANDOFF.md。核验仓库 AGENTS/model-routing 与实际角色/工具，不把模型名称或文件存在当作运行证明。

使用本 S1 集成分支；若工具必须使用独立工作分支，限定为一个主 S1 任务的隔离工作分支，记录实际 HEAD/关系，最终回到同一集成线；不得为了绕过容量创建额外独立 Codex 会话。主分支、PR #6、原 A/R2/R3/D/计划分支不动。

先检查 dirty tree、生产基线差异与有效测试环境。不得 reset/clean/force-push 用户工作。仅文档新增不构成生产漂移；存在额外产品变更时隔离并报告准确冲突，不假报基线一致。

接包先写 artifacts/s1/ACCEPTANCE.json 或批准外置目录：实际任务标识（工具可给时）、工作区、分支/HEAD、PLAN_COMMIT、AUTHORIZATION_REF、已读范围、角色和可用槽位、平台、首批任务及未满足能力。不要预造这些事实。随后直接开始波次 A，不以接包报告代替施工。

## 连续路线与团队

A：WP10a + WP01；B：WP02 → WP03；C：WP09a → WP04。六包均已授权，但各 P 产物必须满足原计划前置和当期门禁才可被下游采用。保留 19 张任务卡、33 项 S1 截止、全体 68 T / 157 检查义务；不能停在波次 A 再要求批准 B/C。

Sol/medium Controller 派发与唯一集成；Luna/high 明确接线；construction Luna/max 批量实现和测试；Sol/high 已定界核心；fresh Sol/high 复核和集中修复后的聚焦检查；fresh Astra/xhigh 按原要求首次/最终全轴审查。实际可用角色以运行环境为准，缺能力不冒名；少槽按 DAG 串行，不删独立审查。worker 不 commit/push，Controller 在授权范围内提交和推送。

普通编译、类型、lint、接线、测试失败在 Codex 消化；同一失败两次先交 Controller。只有改合同、安全/权限、root/writer、采用/迁移边界或真正方案阻断时把受影响路径移交 GPT；仍可安全推进无依赖的施工任务。不要切成一小段施工、一小段研究。

## 测试与硬边界

允许只在 S1 专属临时根运行合成数据、受控假 CLI/测试宿主、准确身份的子孙终止测试、文件锁/落盘/恢复以及私有本地 named pipe/UDS。不能碰真实账号/凭据/费用、真实用户 Home/数据、现有生产 daemon/Tailscale、外网 listener、麦克风/模型资产或产品安装发布。

执行构建前审查 lifecycle scripts 和固定依赖，不盲跑 room:start、npx 下载或含安装/发布效果的命令。未列依赖/升级不得自行采用。新增 scoped 测试可做，不能删除/削弱原测试、改预期或制造语法失败当有效 mutation。角色与沙箱限制不因 Owner 业务范围授权被关闭。

缺 Windows 或所需 OS/进程能力时，完成可安全独立实施/验证的工作并明确记录平台阻断；不把 Linux/mock/编译成功替代真实 Windows L/H。31/49 项 P00 假件也不是新实现验收。

每个安全集成点及时提交，失败和运行中断必须保留 checkpoint/日志/下一任务，不报整个 S1 完成。全部六个 P、当期检查、独立审查和受控纵向演示满足后才可声明 S1 完成。此时仅阶段回执，不自动合并、发布或启动 S2。

## 阶段结束回 GPT

固定 actual HEAD、六个产物、每 check 的真实命令/退出码/fail/skip、独立报告、失败修复、残留 custody、回滚、未执行 N1/I/M/平台矩阵和待决事项。若 S2 仍需研究/方案，NEXT_PLATFORM=GPT；不是 Codex 自行开下一轮设计。后续施工须已有方案和新的明确范围授权。
