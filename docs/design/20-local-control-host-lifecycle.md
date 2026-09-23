# Windows Local Control Host 与桌面产品生命周期

- 状态：`v0.2-p31-d1-product-contract-draft`
- 性质：P31 D1 产品契约草案；运行时基础由 P38 负责，UI 由 P21、安装升级由 P28 负责
- 原则：桌面窗口是 Control Host 的客户端，不是 Controller、Scheduler 或 Runner 的生命周期所有者

## 1. 产品进程模型

P31 D1 产品拓扑：本地单用户、Windows 原生、当前 OS principal 所有的 per-user 后台进程；**不使用 Windows Service**。该选择在 Owner `ACCEPTED` 前仍是草案，但下游不得改选 Service 或跨用户会话：

```text
Desktop UI process
        │ authenticated local API + event cursor
        ▼
Local Control Host（单实例）
  ├─ logical Controller / Scheduler / Reconciliation
  ├─ Coordination Store owner
  ├─ Repository/Artifact services
  ├─ Runner connection manager
  └─ notification/deep-link broker
        │
        ▼
Host Runner / supervised Harness processes
```

关闭或崩溃 Desktop UI 不得终止 Control Host、Runner 或已在途 Attempt。Control Host 退出也不能直接把未知的原生进程标为终止；重启后先 reconciliation。

## 2. 用户可理解的生命周期

产品必须区分：

- `Close window`：关闭当前 UI，后台任务继续；若仍有活跃/Attention 项，给出明确提示。
- `Quit GOGO PARTY`：请求停止 UI 和 Control Host；有在途副作用时进入有界 settle/cancel/reconcile，不承诺立即退出。
- `Pause automation`：停止认领新 Assignment，不等于取消已运行 Attempt。
- `Stop current attempt`：发送 fenced cancel，等待真实终止证据。
- `Force terminate`：高风险动作，展示影响范围和未知结果，形成 Decision/Receipt。
- `Windows sign-out/shutdown`：尽力持久化 cursor/spool，不伪造正常完成；下次启动执行恢复。

系统托盘、后台启动和“登录后自动运行”必须是显式用户设置。默认由 UI/deep link 按需启动 Host；UI 关闭后 Host 可继续当前任务，但用户注销/关机时只做有界持久化和安全终止，不伪装成 service 跨用户会话继续运行。未显式启用登录自启时，Host 不得静默随系统启动并认领新工作。

Host 状态目录默认位于当前用户的 LocalAppData 下，ACL 只授予该 OS principal、SYSTEM 和显式的管理恢复边界；不使用公共可写目录。更新必须取得独立 update lease，让旧 Host 停止新 claim、有界 settle 并提交 handoff，新版本完成 migration/reconciliation 前不启动第二 Controller。

## 3. 首次启动与项目进入

首次启动向导必须形成可恢复、可重进的步骤：

1. 签发 GOGO `operator_id` 并建立 Control API authentication。OS principal 是必要归属证明，不等于已授权用户；origin/CSRF/IPC identity 与 Host owner token 缺一则拒绝。
2. 选择或创建 Project，绑定并验证**恰好一个** Source Repository。
3. 探测公众版 Harness 安装和版本。MVP 缺失时只提供 **detection + manual official guide**。Controller-authorized one-click official install plan 不进入 MVP；若未来超出 P11/P35/P23/P24 边界，必须新开工作包，不能塞进 Adapter 或 P28。
4. 创建 HarnessProfile，显示 shared/dedicated 的隔离差异；登录/MFA 始终由用户在可见官方流程完成。检测回执、安装回执和 READY 互不等价，不得互相伪装。
5. 探测 CapabilityEvidence、ModelProfile 与 ProviderReadiness。
6. 选择 Party/Role 模板；MVP 允许复制后受控编辑，修改形成新版本而不是覆盖通用模板。
7. 预览 ExecutionTarget、权限、预算、自动化上限和将生成的项目文档。
8. 完成 Context projection/LoadProof 后才显示 Agent READY。

任一步失败都保留已完成状态和安全的重试入口；不得通过扫描整个用户 Home、复制凭据文件或隐式登录“自动修复”。

## 4. 单实例、启动和恢复

Local Control Host 必须使用精确 process identity 和单实例协调，不以进程名或端口占用猜 owner。第二 UI 实例连接既有 Host；若 owner 身份或 Store lease 不可证明，进入 `RECOVERY_REQUIRED`，不能启动第二 Controller 写入。

启动顺序：

1. 验证数据目录、版本、migration/backup compatibility；
2. 获取 Control Host identity 与 Controller epoch；
3. 恢复 Coordination/Outbox/Spool；
4. 对账 Runner、Harness process、Git 和 Artifact 状态；
5. 恢复 projection cursor；
6. 仅在无双写风险后开放新 claim。

UI 可在 Host 恢复期间只读显示 `RECOVERING/UNKNOWN`，不得用缓存投影显示绿色运行态。

## 5. Attention 与通知

后台运行时，仅以下事件产生系统通知：人工 Gate、权限/登录需求、失败或 UNKNOWN、预算/额度阻塞、危险动作确认和用户订阅的完成事件。通知只包含脱敏摘要和稳定对象 ID；点击后深链到正确 Project/Task/Attention，并重新鉴权与验证当前 revision。

通知本身不能执行批准、重试、切目标或危险动作。Toast 丢失不丢 Attention；权威待办保存在 Coordination Plane。

## 6. 备份、恢复、迁移与卸载

产品必须把内容分为：

- 可移植：Control Repository、Project 配置、结构化事实、SQLite backup、Artifact manifest/content（按策略）、Role/Workflow 模板。
- Host-bound：CredentialHandle 解析、OS principal binding、加密密钥、Harness 登录态、native Session locator、进程身份和部分 Profile 缓存。
- 可重建：read projection、搜索索引、部分 Live Observation。

备份必须固定版本、数据库快照、repository revision、artifact manifest 和校验摘要。恢复先在隔离位置验证兼容性、Project ID 冲突和缺失 Host-bound 项，再原子切换；失败不得覆盖当前可用数据。

portable backup 默认加密，并默认排除 secret material、CredentialHandle 解析值、token/cookie、Harness 登录态、native Session cache、活进程身份和未经用户选择的 `RESTRICTED` Artifact。可移植包使用用户显式管理的 passphrase/recovery key 派生包密钥；DPAPI 可保护本机缓存密钥，但 DPAPI-only 包必须标为 Host-bound，不得宣称 portable。密钥不写入同一 backup bundle。

恢复必须先验证 authenticated manifest/content digest、版本和密钥，再在新 Host 重建 local key wrapping；不可移植对象统一进入 `REAUTH_REQUIRED | REBIND_REQUIRED | UNAVAILABLE`。support bundle 是独立的最小脱敏导出，默认不包含 Context 正文、prompt、transcript、repository content、账号标识和 secret，并在导出前给用户 manifest 预览。

迁移到另一台机器后，Host-bound 对象进入 `REAUTH_REQUIRED | REBIND_REQUIRED | UNAVAILABLE`，不能伪装为已恢复。卸载默认不删除 Project/Artifact 数据；删除数据是独立、可预览的破坏性流程。

## 7. 最低验收

1. UI 关闭/崩溃时在途 Attempt 继续，重开后从持久 cursor 补齐。
2. Control Host 在 claim、command、receipt、Git outcome 不同阶段崩溃，重启不双跑且未知状态进入 Attention。
3. 两个 UI 同时启动只连接一个权威 Host；伪造端口/PID/旧 owner token 不能接管。
4. Pause、Stop、Quit、Force terminate 的语义和回执互不混淆。
5. 首次向导在 Harness 缺失、login-required、Profile warning、LoadProof 失败时可安全恢复。
6. 通知迟到、重复或点击旧 revision 不会重放动作。
7. 备份/恢复成功、密钥错误、摘要篡改、损坏、版本不兼容、目标 Project 冲突、DPAPI-only 跨 Host 拒绝和 Host-bound 缺失均有负测。
8. 卸载不默认删除用户项目；数据删除需要独立确认和 tombstone/receipt。
9. per-user Host 不以 Windows Service 或其他 OS principal 启动；未授权登录自启、旧版 update owner 和公共目录写入都失败关闭。

## 8. 施工归属

- P31：用户语义、首次旅程、关闭/退出和数据移动边界。
- P38：Local Control Host 单实例、启动恢复、后台生命周期、通知 broker 和运行时状态机。
- P18/P19：本地认证 API、持久 cursor 与恢复投影。
- P21：桌面窗口、托盘、向导、Attention 深链和用户交互。
- P28：安装、升级、回滚、备份/恢复和卸载包行为。
- P26/P34：通用与 Project Context 专项威胁/恢复验证。
