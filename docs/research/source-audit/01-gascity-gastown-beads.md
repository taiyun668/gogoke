# Gas City、Gas Town、Beads 源码审计

## Gas City

### 可直接借鉴

1. **Session 不是 Work。** `internal/session/REQUIREMENTS.md` 明确把 Session 定义为可替换的运行身份，工作持久化在 Beads；这与 GOGO PARTY 的 Agent/Session 分离完全一致。
2. **运行时失败必须是 UNKNOWN。** `internal/runtime/runtime.go:51-67` 定义 `ErrRuntimeUnavailable`，并明确它不能被破坏性 reconciler 当作“没有运行中的 session”。Provider 接口在 `:137-229` 统一 Start/Stop/Interrupt、存活、活动时间、列表和能力。
3. **统一状态不是二值。** `internal/worker/handle.go:95-113` 的 phase 包含 unknown、starting、ready、busy、blocked、stopping、stopped、failed；接口还把 lifecycle、messaging、transcript、interaction 和 live observation 分开。
4. **状态驱动轮换。** Session requirement ledger 覆盖 progress stall、idle timeout、max session age、待交互、assigned work、rate limit、rapid crash、context churn 和 quarantine。轮换决策明显不只是 token 阈值。
5. **协调循环有失败语义。** `internal/dispatch/control.go:108-220` 将 closed attempt 分类为 pass、hard-fail、continue/retry、exhaust；结构损坏会 quarantine，而不是让整个 serve loop crash-loop；下一 attempt 使用幂等键。
6. **事件与事实分层。** `internal/events/events.go:331-375` 的 Event/Provider 提供 append/query/watch，但项目说明事件通知不是持久任务真相本身。

### 需要修改后借鉴

- Gas City 当前 `main` 是集成 Gas City、T3 Code 和 DoltLite-backed Beads 的 fork，不能把 fork 特有耦合误当成通用 SDK 边界。
- 默认/回退运行时仍深度依赖 tmux。官方 Windows 路径是 WSL，不是原生 Windows 控制平面；GOGO PARTY 不能以 tmux 为核心抽象。
- Gas City 倾向“判断留在配置/Prompt，不进 Go”。GOGO PARTY 的权限、隔离、状态提升和自动接力必须保留确定性 Policy/Transition Engine，不能交给 Prompt 判断。
- Beads Store 的某些批量元数据写对外部存储是顺序应用，存在部分失败窗口；GOGO 需要把权威状态提升设计成明确事务或可恢复 saga。

### 测试源码证据

存在 runtime provider conformance、worker phase/structured transcript conformance、event conformance、API idempotency、session lifecycle/chaos、store conformance 和 acceptance tests。`internal/worker/workertest/structured_conformance_test.go` 还用 Claude、Codex、Gemini 真实格式的 fixture 归一化结构化工具结果。状态：`TEST-SOURCE / NOT-RUN`。

## Gas Town

### 可借鉴的产品概念

- `DefaultAgent`、town/rig/role/worker 多层覆盖体现“通版角色 + 项目/成员覆盖”；解析优先级写在 `internal/config/types.go:52-75`。
- `internal/beads/handoff.go:25-210` 用固定 handoff bead 与自发 mail 支持换会话接力，并清理旧 hooked mail。
- `internal/cmd/crew_lifecycle.go:214-239` 的默认接力消息要求新 Session 回到 mail 和 Beads 取当前状态，契合“成果和事实优先于复述对话”。
- `internal/polecat/manager.go:1344-1399` 的 reclaim 在 Agent、Session、assigned work、worktree、MR 证据不清时 fail closed，值得保留。

### 不应照搬

- Web 状态在 `internal/web/api.go:1866-2012` 依赖 tmux 活动时间、pane 当前命令、最后十行是否出现 `?` 等词，以及 2/10 分钟阈值。这只能是低置信度 hint，不能成为“完成/等待/可回收”的权威证据。
- `tmux not running` 直接投影为 ready，会把“观察失败”和“没有 Session”混为一谈；GOGO 必须保留 UNKNOWN。
- 角色和目录结构较固定，适合参考 Party 模板，不适合作为用户可自由组织角色的领域模型。
- 根 `AGENTS.md` 指向不存在的 `CLAUDE.md`，说明仓库级指令交付也可能漂移；GOGO 的 Context Compiler 必须验证生成物存在、版本和 hash。

## Beads

### 它真正是什么

`engdocs/PROJECT_CHARTER.md` 明确：Beads 只拥有 issue lifecycle、依赖、readiness、metadata、同步、备份和恢复；agent routing、模型选择、重试、调度和跨系统协作属于上层 orchestrator。它是很好的任务账本参考，不是现成指挥中心。

### 可直接借鉴

- `internal/storage/storage.go:100-320` 把 lifecycle、reader、atomic claimer、ready claimer、dependency editor、metadata CAS 等拆成角色接口。
- `internal/storage/bulk_issues.go:16-27` 定义 claim、claim-ready、heartbeat 和 expired lease reclaim。
- `internal/storage/embeddeddolt/ready_claimer.go:30-65` 在同一事务中完成 ready selection、compare-and-set 和 hydration，避免双派发。
- `internal/storage/storage.go:55-59` 的 `ErrCommitIndeterminate` 很关键：提交结果不确定时不得自动重放，否则可能重复执行。
- Provenance 是 append-only、确定性 ID 幂等；适合 GOGO 的 Result/Evidence/Revision 引用。
- clone-local metadata 与会同步的项目事实分开，适合区分本机 Runner 状态和项目权威状态。

### 对“仓库消息总线”的校正

当前 Beads 的 source of truth 是 Dolt。数据经 `refs/dolt/data` 同步；`.beads/issues.jsonl` 只是 viewer/interchange 的被动导出，文档明确禁止把 JSONL 当同步协议或完整备份。因此 GOGO PARTY 可以让 Git 仓库承载成果、事实和任务投影，但并发认领、租约、幂等消费和运行协调必须有结构化协调存储，不能只靠 Agent 写文件。

### 结论

Beads 的语义比它的具体 Dolt 实现更值得复用。MVP 应先实现较小的 `Task + Dependency + Claim/Lease + CAS + Provenance` 内核；是否采用 Beads/Dolt 本身应另做依赖、Windows 和运维评估。
