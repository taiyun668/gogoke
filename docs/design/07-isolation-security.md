# 隔离与安全

- 状态：v0.5-p31-d3-p17-namespace-handoff
- 原则：Project 是强隔离域；Agent 在 Project 内按职责、Assignment 和阶段执行最小权限
- P31：施工前 typed security requirements；P17 是唯一 Project/worktree namespace owner；专项后置整合由 P34 消费，不得与 P26 并行写同一文件

## 1. 必须隔离的命名空间

所有以下资源都必须绑定并验证 `project_id`：

- Coordination Store 主键、唯一键和事务查询
- Event stream topic、consumer offset 和 cache key
- Runner Command、lease、fencing token 和 artifact upload
- workspace/worktree、临时目录、日志与 spool（P17 unique owner）
- native Session ID 与 Adapter cache
- credential handle、网络策略和预算账户
- HarnessInstallation、ModelProfile、ProviderReadiness 与 ExecutionTarget 候选/选择记录
- ContextSnapshot、Prompt Manifest、Evidence 和 Result 路径

仅靠目录前缀或 UI 过滤不构成隔离。任何缺失 Project scope 的持久对象都应被 Schema 拒绝。P17 是唯一 Project/worktree namespace authority；P16/P33/P18/P22 只消费，不得另建 root、binding 或 source-write 路径。

## 2. 隔离层与证明

| 层 | 机制 | 必需证据 |
|---|---|---|
| 逻辑数据 | project-scoped query、FK/unique constraint、API authorization | 负向测试与审计日志 |
| 文件系统 | 独立 workspace/worktree、路径 canonicalization、allowlist | resolved path、root ID、越界测试 |
| 进程 | 独立 process tree、Runner ownership、fenced command | process identity、start token、termination receipt |
| 凭据 | host-local handle、role/assignment scoped resolution | readiness 与 scope，不含 secret |
| 网络 | profile allow/deny、代理/沙箱能力声明 | enforcement strength 与 probe evidence |
| 上下文 | Snapshot filtering、Prompt digest、Reveal Gate | manifest 与 visibility proof |
| 成果 | per-agent/attempt namespace、immutable receipt | content hash 与 producer scope |

CapabilityEvidence 必须区分 `PREVENTIVE`、`APPROVAL`、`POST_HOC` 和 `NONE`。没有预防能力时，UI 不得用“沙箱”措辞误导。

## 3. Workspace 模型（P17 unique owner）

P17 强制执行 immutable `Project -> SourceRepository/repository_binding_id`。Attempt 一旦绑定，source write 不得更换 Source Repository 或 `repository_binding_id`。

Canonical namespace 只由已注册 runner root 下的 `project_id/assignment_id/attempt_id` 组成：

```text
<registered-runner-root>/<project_id>/<assignment_id>/<attempt_id>/
```

D1 推荐路径中的 `agent-id` 段不是权威组成。显示名、任务标题、branch 名或 Agent 输出不得成为路径段。实际路径必须 canonicalization，并验证仍在该已注册 runner root 内。

Worktree ownership/lease 绑定当前 Controller epoch、Assignment/Attempt fence 与 Runner/process identity。Runner 只持有 P17 签发的当前 lease，不得自行准备平行 worktree。

每次 source write 前必须完成全部 binding/fence 校验。下列任一失败均为 zero-write reject：

- cross-Project（含 name collision：用显示名/目录名撞上另一 `project_id`）
- cross-Attempt
- stale Attempt/epoch/fence/lease 或 Runner/process identity 不匹配
- path-escape（`..`、绝对路径、junction/symlink 逃出 registered runner root）

Dirty workspace 进入 Attention 与 human Gate。不得 auto-clean、auto-reuse 或 fallback。写 Source Repository 的 Assignment 使用该独立 worktree/clone。只读 Auditor 使用只读快照或受控副本；同一 Project 的 Agent 也不共享可写工作目录。旧 Attempt 在重派后不得继续写新 Attempt 命名空间。P16 只做 control-repo materialization，不得把 task worktree 当作 namespace authority。

## 4. 凭据

- Coordination/Control Repository 只保存 `credential_handle` 与非敏感 readiness。
- Secret 由 Host-local provider 解析，按 Project + Role + Assignment scope 注入。
- Runner 不回传 secret 值、cookie、token 文件内容或可复用认证 material。
- 环境变量、日志、stdout、crash dump 和 artifact 上传必须经过 secret redaction。
- 权限提升返回 `ASK` 并形成人类 Decision；不得由 Adapter 自动重试为更高权限。

## 5. Agent 间可见性与盲审

默认可见：本 Agent 的 Assignment、Snapshot、允许的 Facts/Evidence、自己的 Result 和公共 Project Policy。

默认不可见：其他 Agent 的未接受 Proposal、独立 Auditor 的结论、无关任务 transcript、其他 credential handles 和跨项目资料。

盲审需要的不只是不同目录：

1. 为每个 Auditor 生成独立 ContextSnapshot 和结果命名空间。
2. Event subscription 与 UI/API 查询在 Reveal Gate 前过滤其他 Auditor 结果。
3. Prompt Manifest 不含共识、排名或其他 verdict。
4. Gate 保存 visibility manifest/hash，证明审计时可见集合。
5. 结果提交并封存后才能 fan-in/reveal。

无法证明独立性时 Gate 为 `INCONCLUSIVE`。

## 6. 跨项目发布

跨项目复制必须形成 PublicationCapsule，声明发布者、源/目标 Project、允许字段、内容 Hash、有效期和撤销策略，并经双方 Policy 检查。共享 cache、全局向量库、剪贴板监听或通用“最近上下文”不得绕过此边界。

## 7. Runner 信任与故障边界

本地 Runner 通常与当前 OS 用户同信任域；这不等于对恶意本机管理员提供安全隔离。Remote Runner 需要机器身份、传输加密、注册审批、epoch/fencing 检查和最小 artifact scope。

Runner 失联时：停止发新命令、标记观察 `UNKNOWN`、保留 lease/attempt 证据并 reconciliation。不得因心跳超时立刻在另一 Host 启动同一可写 Assignment。

## 8. Windows Execution Profiles

### 8.1 Windows Native Control Plane

必须支持 UI/API、Coordination Store、Repository Projector/Reactor、Git、Context Compiler 和本地无云运行。

### 8.2 Windows Native SDK/headless CLI Runner

仅调度已验证支持非交互/结构化协议的 Harness。Job Object 可约束进程树和终止传播，但不等于 filesystem/network/credential sandbox。

### 8.3 WSL Runner

用于 Linux-only CLI、tmux/PTY 或更成熟的 Linux 工具链。Windows 路径必须转换为受验证的 `/mnt/...` 或 WSL 原生 workspace；不得把未经转换的 `<local>\...` 传入 Bash。

### 8.4 Remote Linux Runner

用于需要长期后台执行、强 Linux 隔离或 Host 能力不同的任务。MVP 不要求，但协议从一开始不得假设 Runner 与 Control Plane 同机。

## 9. 高风险动作

默认需要显式 Policy/人工 Gate。每次危险动作必须同时携带 exact scope、actor、expected revision、幂等键、风险预览与显式确认 Decision：

- merge、force push、tag、release、deploy
- 删除/覆盖仓库、工作区、artifact 或 Project
- 写系统设置、安装软件、修改网络/防火墙
- 访问新 credential profile、更换账号或跨项目数据
- 向外部服务发送消息、创建费用或公开发布
- Force terminate、越权批准、清理/GC 绕过 HOLD

Task `COMPLETED` 不授予上述权限。自然语言授权、reaction、通知点击和托盘菜单都不能满足确认要求。

## 9.1 Operator identity 与本地认证

- `operator_id` 在首次启动签发，是 GOGO 持久操作者身份。
- 当前 OS principal 是必要的本机归属证明，**不等于**被授权用户。
- Control API 必须验证本地认证会话：origin、CSRF、IPC identity 与 Host owner token。
- 端口占用、同一 OS 用户、窗口标题、PID 或“本机进程正在运行”都不足以证明 authority。
- 必须防护：local process spoofing、origin/CSRF、跨 Project replay、缓存串域。缺一项则 fail-closed。

## 10. 最低安全测试

必须包含：路径逃逸、Project ID 混淆、缓存键碰撞、旧 fencing token、重复 Command、恶意 Result path、secret 日志泄露、Blind Review 泄露、Runner 失联双跑、WSL 路径错配和跨项目 Publication 拒绝测试。P17 负测还必须覆盖两个 active Project 的 workspace 互写拒绝、name collision、path traversal、stale Attempt 与 dirty workspace 的 fail-closed/no-write；不得 auto-clean、auto-reuse 或 fallback。

## 11. CredentialHandle、HarnessProfile 与 Workspace 分离

三者是独立安全对象：

- `CredentialHandle` 只允许目标 Harness 进程在 host 本地使用，不暴露 token 内容。
- `HarnessProfile` 决定用户级 config、hooks、plugins、MCP、native session store 和缓存的继承范围。
- `Workspace` 由 P17 按 `project_id/assignment_id/attempt_id` 在已注册 runner root 下创建，承担文件系统和 repository revision 隔离；不得按 Agent 名、Session 名或显示名建根。

首版显式支持 `SHARED_USER_PROFILE` 与 `DEDICATED_PROFILE`。共享 Profile 必须显示继承源、执行配置 preflight、过滤账号级事件；独立 Profile 必须通过官方 CLI 自行登录，GOGO 不复制 credential 文件来克隆身份。

Grok Build 可以通过 `GROK_AUTH_PATH` 将公众登录态与隔离 `GROK_HOME` 分开；Codex 当前公众 OAuth 登录态与 `CODEX_HOME` 的耦合更强，严格配置隔离应采用 dedicated login。任何 Profile 中的 hook/plugin/MCP 解析告警必须进入 `adapter.warning`，并由 Policy 决定是否阻塞，而不是继承整个用户 Home 后假定环境干净。

## 12. ExecutionTarget 与 Provider 隔离

- Target candidate、ProviderReadiness、pricing/quota evidence 和 fallback policy 必须按 Project/Assignment scope；不得使用全局“最近账号/最近模型”作为隐式选择。
- Attempt 绑定精确 HarnessInstallation、ModelProfile、HarnessProfile、CredentialHandle 和 Host。任一项变化都需新的 TargetSelectionRecord。
- ProviderReadiness 只暴露脱敏状态、scope、证据等级和有效期；账号标识、限额原文和登录材料不得进入 Project Event、Context 或 Git。
- 只有明确归因到目标 Profile/账号的 quota evidence 可以冻结该目标；网络、认证未知或 Provider 全局故障不得误伤其他 Project/Profile。
- shared profile 即使由多个 Project 使用，也必须分离 event filter、native Session locator、budget 和 workspace；无法证明过滤时该 Profile 对自动调度为 `UNAVAILABLE`。

## 13. Local Control Host 边界

Local Control Host 与 Desktop UI 使用本地认证、origin/CSRF/IPC identity 和单实例 owner token。端口存在、同一 OS 用户、窗口标题或 PID 相同都不足以证明 authority。第二实例、旧 owner token、恢复中的 Store 和 UI 缓存不得启动第二 Controller 或推进状态。

通知和 deep link 只携带脱敏对象引用；点击后必须重新认证、验证 Project scope 和当前 revision。Toast、托盘菜单和操作系统启动项不能成为旁路审批或自动 claim 通道。

## 14. 跨 Project 共享容量边界

- ResourcePool 可以是 Host/Provider/Profile 全局 scope，但 Project 只能看到自己的 CapacityRequest/Permit 和脱敏等待原因。
- 公平性和防饥饿不得暴露其他 Project 的任务、Profile、账号、模型用量或精确队列位置。
- permit 绑定 project/assignment/attempt/controller epoch/fence；旧 permit、重复 grant、越界 release 和旧 Runner 迟到事件均不得影响当前容量。
- shared Profile 的 capacity/readiness 若无法证明 scope 则为 `UNKNOWN/UNAVAILABLE`，不得通过合并账号状态提高可用额度。
- ResourceBroker 的 policy 更改是可审计 Decision；UI、Adapter 和 Runner 都没有隐式优先级通道。

## 15. P31 施工前安全交接（供 P34 消费）

下列 typed requirements 在 Owner 冻结后由 P34 映射到 threat register、audit vocabulary 与负向 CI。P26 只拥有通用 baseline；P31 不写 P26 文件。

| ID | 要求 | 负测要点 |
|---|---|---|
| `P31-SEC-01` | Project/Agent/Auditor/Context candidate/Artifact 隔离；P17 namespace fail-closed | 跨 Project 查询、缓存键、workspace；两个 active Project 互写；name collision；path traversal；stale Attempt；dirty workspace no-write |
| `P31-SEC-02` | operator/auth、origin/CSRF、local-process spoofing | 仅 OS 用户/端口/PID 不能通过危险动作 |
| `P31-SEC-03` | 危险动作 exact scope + expected revision + 确认 | 无预览或自然语言授权不得执行删除/发布/强制停止 |
| `P31-SEC-04` | detection 回执 ≠ 登录成功；禁止隐式登录/切账号 | Adapter 扫描 Home 或复制凭据文件必须拒绝 |
| `P31-SEC-05` | ExecutionTarget 禁止静默替换；readiness 误归因 | 切模型/Profile/Host 无新 Attempt；全局故障不冻结错误账号 |
| `P31-SEC-06` | Local Control Host 单实例与旧 owner/deep-link | 第二 Controller、旧 token、通知重放 |
| `P31-SEC-07` | backup/restore 不覆盖；Host-bound 不可伪装 READY | 错误密钥、篡改摘要、跨 Host DPAPI-only |
| `P31-SEC-08` | CapacityPermit 双 grant/旧 fence/饥饿/队列隐私 | 等权连续旁路 >2、跨 Project 用量泄漏 |
| `P31-SEC-09` | Context Room mention/reaction/transcript 无权威 | `@Name` 无 agent_id、👍 当 PASS、Reveal Gate 命中数 |
| `P31-SEC-10` | Context retention HOLD/tombstone；transcript 不默认升 Artifact | 活跃 Gate 引用被 GC；未脱敏 transcript 入库 |
