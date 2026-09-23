# 账本、工作流、角色与上下文源码审计

## Beads：任务账本语义基线

Beads 仍是 GOGO Task Ledger 的主要语义来源：Task/Dependency、ready query、原子 claim、lease/heartbeat、CAS、indeterminate commit 和 provenance。其 Dolt/JSONL 选型不进入 GOGO Core。

判定：`SEMANTIC-PORT`。GOGO 要重新实现最小 SQLite/Postgres 账本，并用竞争 claim、租约过期、旧 owner fencing、重复 receipt 和未知提交结果的负向测试验收。

## Gas City：长期协调与 Session 治理基线

可移植语义：

- Session 与 Work 分离，UNKNOWN 是显式运行失败而非 idle；
- attempt phase、retry/exhaust/quarantine 和 dispatch 失败分类；
- 根据任务状态、等待交互、卡死、年龄、crash churn 和上下文压力轮换 Session；
- 事件、事实和投影分开。

大体量 Go 实现、tmux/WSL 耦合和 prompt policy 不适合整块复制。判定：`SEMANTIC-PORT`。

## Gas Town：角色与 handoff

可移植语义：角色默认值 + 项目覆盖 + 成员覆盖、handoff/reclaim fail-closed、fleet/party 可视化。固定 Town 角色命名、tmux pane heuristics 和把运行目录当身份不进入 GOGO。

判定：领域和交互为 `SEMANTIC-PORT` / `PRODUCT-PATTERN`，固定组织模型为 `REJECT`。

## agtx：项目隔离、插件工作流与一键操作

### 值得移植

- task 明确携带 `project_id`、agent、session、worktree、branch、plugin 和 cycle；全局 MCP 与项目 MCP 分开，global mode 强制先解析 `project_id`。
- `ProjectConfig` 支持项目级默认 Agent、阶段 Agent 覆盖、worktree、copy files、init/cleanup 和 workflow plugin。
- `WorkflowPlugin` 将阶段命令、prompt、artifact、trigger、copy-back 和 context clear 配成数据，适合作为 GOGO Role/Workflow template 的产品参考。
- transition request 用带条件的单条 `UPDATE ... WHERE claimed_by IS NULL AND processed_at IS NULL` 抢占；notification 用 `DELETE ... RETURNING` 原子消费。
- MCP 暴露 list/get/create/update/move task 和 allowed actions，说明同一 Command 可服务面板、一键按钮与自动 Controller。

### 不直接采用

- claim 没有 lease、claim token 或 fencing；旧 claim 一小时后直接被清理，不能证明工作安全接管。
- tmux 是会话承载和状态观察的核心依赖，不适合 Windows-first 公共内核。
- artifact/prompt trigger 可作为信号，但不得自行提升 authoritative task state。
- orchestrator 收到完成通知后直接前移，不审计成果；不符合 GOGO 的 Gate authority。

判定：Rust 数据结构和个别原子 SQL 为 `SOURCE-CANDIDATE` 参考，但账本整体仍为 `SEMANTIC-PORT`；TUI/一键阶段操作为 `PRODUCT-PATTERN`。

## Squad：轻量消息与角色投影

### 值得移植

- SQLite WAL + busy timeout；Agent、Message、Task 分表；消息带 kind、task ID 和 reply-to。
- create task 与 `task_assigned` message 在同一事务内提交。
- receive messages 在同一事务中读取并标记已读。
- task ACK 用条件更新抢占，requeue 对读取到的完整旧值做 CAS。
- built-in role + project-local role Markdown，以及同一意图投影为 Claude/Codex/Gemini/OpenCode 的不同发现路径。
- session token 能发现同 ID 被另一终端替换，client/protocol version 为兼容性演进提供字段。

### 限制与拒绝

- 完成任务只校验 `status` 和 `lease_owner`，不校验租约是否过期；没有 fencing token，旧 worker 仍可能完成。
- session token 缺失时为向后兼容而 fail-open，且 token 是本地明文文件；不能直接成为 GOGO 身份安全模型。
- receive loop 依赖 Agent 按 Prompt 反复运行 `--wait`，Prompt 仍承担控制职责。
- 没有项目 ID 列；隔离依靠每个 workspace 一个 `.squad`，不能直接扩展为多项目中央控制面。

判定：小型 Rust schema/角色投影可作为 `SOURCE-CANDIDATE` 研究样本；账本和身份语义必须 `SEMANTIC-PORT` 后补齐 fencing、project scope 和 deterministic runner。

## Open Harness：可移植控制面与安全脚手架

### 值得移植

- `.oh/` 将控制面机器资产与项目源代码分开，并用 manifest include/exclude 定义可分发 payload。
- `oh init` 对所有目标写入做 path-escape guard，默认不覆盖既有文件；可生成 AGENTS/CLAUDE alias 和 provider-specific skills/agents/hooks surface。
- provider symlink repair/check 将 canonical pack 与各 Harness 发现路径分离。
- 每个目标项目新建自己的 memory/tasks/workspace seed，不把上游项目自己的运行历史复制进去。
- scaffold 明确把 secrets 放到 gitignored env，而非项目 YAML。

### 限制与拒绝

- Docker/devcontainer、tmux、单一 sandbox workspace 是该产品主路径，不能成为 GOGO Core 强依赖。
- `oh update` 在 `.oh/` 内可覆盖用户修改且当前无 backup；GOGO Context Projector 必须保留 generated manifest、previous hash 和 rollback。
- symlink 不能作为 Windows 唯一路径，需要 copy/junction/manifest 三种受测策略。
- 大量自动化仍由 Markdown skill/prompt 驱动；GOGO 确定性 claim、Gate 和 receipt 必须由 Controller 执行。

判定：manifest、path guard、provider projection 与 scaffold 幂等性为 `SOURCE-CANDIDATE`/`SEMANTIC-PORT`；整套容器壳为 `REJECT`。

## Clay：YOKE Adapter 与跨供应商上下文

### 值得移植

- Claude/Codex Adapter 把供应商事件 flatten 成统一 YOKE events，支持 resume、abort、usage、approval 和 context-compaction 表达。
- Codex Adapter 直接使用 app-server thread/turn/item；Claude Adapter 使用 SDK/worker，证明同一宿主可以拥有不同 transport。
- `instructions.js` 证明“排除目标 Harness 原生会读的文件，只补充其他供应商指令”有实际价值。
- TUI/native Session 保持原生形态，冷 Session 才根据查看偏好切换表示；避免观察行为劫持共享会话。
- worker abort 给 SDK 保存 Session 的 grace，超时后才强杀，体现 cancel 与 resumability 的冲突。

### 限制与拒绝

- instruction merge 只扫描根目录固定文件表，优先级固定；没有祖先/子目录 specificity、mandatory fragment、冲突拒绝、hash 或 omitted manifest。
- 两个 Adapter 分别超过千行，宿主、SDK、UI 和 worker 管理耦合明显，不适合整块复制。
- Ralph loop 以 Session callback、文本 judge verdict 和 watchdog 推进，不能替代 GOGO 的任务回执与 Gate。
- CLI transcript 解析只能做只读展示或低置信度 metadata，不能还原权威任务状态。

判定：YOKE 事件词汇、Adapter 分离和 instruction exclusion 为 `SEMANTIC-PORT`；workspace/chat/PWA 为 `PRODUCT-PATTERN`。

## GOGO 目标实现

这些来源合并后，GOGO 应实现三件独立的东西：

1. **Task Ledger**：带 project scope、atomic claim、lease、heartbeat、monotonic fencing token、receipt dedup 和 CAS。
2. **Workflow Engine**：把阶段、依赖、Gate、重试、人工/一键/自动 Command 表达为数据，但不让 Prompt 或 artifact 自行取得 authority。
3. **Context Projector**：从 canonical project/role/member/task 片段编译每个 Harness 的项目文档，输出 included/omitted/conflict/hash manifest，并验证实际加载。

