# Artifact、Evidence 与保留策略

- 状态：v0.4-p31-d1-product-contract-draft
- 目标：保留足够证据完成审计和换窗，同时控制敏感数据、体积、成本与删除风险
- P31：ContextEntry/Proposal/Snapshot/Handoff 的 retention/HOLD/GC owner 与 backup 密钥语义的 D1 草案；执行由 P36/P28 后置

## 1. 对象区别

- Artifact：任务产生的文件、补丁、构建物、报告、截图、日志包等内容对象。
- Evidence：对 Result、Gate 或 Decision 的可验证引用，可以指向 Artifact、Git object、命令记录或外部系统。
- Transcript：Harness 私有会话历史，默认既不是 Artifact 也不是 Evidence。

把 transcript 片段提升为 Evidence 必须显式选择、脱敏、记录来源和适用范围。

## 2. ArtifactDescriptor

```yaml
artifact_id: artifact-...
project_id: project-...
assignment_id: assignment-...
attempt_id: attempt-...
producer_id: agent-or-runner-id
media_type: application/zip
size_bytes: 12345
content_digest: sha256:...
storage_uri: gogo-artifact://project/.../artifact-...
classification: PROJECT | CONFIDENTIAL | RESTRICTED
state: STAGED
retention_policy_id: retention-...
created_at: RFC3339
expires_at: RFC3339|null
```

`storage_uri` 是受控 locator，不把本机绝对 secret 路径暴露给 Agent/UI。Evidence 引用同时固定 descriptor revision 和 content digest。

## 3. 内容状态机

```text
STAGED -> VERIFIED -> RETAINED -> EXPIRED -> DELETION_PENDING -> DELETED
             └────> QUARANTINED
任意可保留状态 -> HOLD
```

- `STAGED`：上传/生成中，不可作为 PASS 证据。
- `VERIFIED`：大小、Hash、producer/project scope 已核对。
- `RETAINED`：按策略可读取并可用于 Gate。
- `QUARANTINED`：恶意、泄密、格式或来源可疑，只允许受限检查。
- `HOLD`：被活跃 Gate、争议、审计或人工决定冻结，暂停自动删除。
- `DELETED`：内容已删除但保留不含敏感值的 tombstone、Hash、删除 Decision 和时间。

## 4. 上传与提交

Runner 先 `artifact.prepare`，Controller 返回 scoped upload grant；写入临时对象后校验长度/Hash，再 atomic finalize。Finalize 与 ResultCapsule 接受使用独立状态：Result 不得引用尚未 VERIFIED 的必需 Artifact。

上传结果不确定时按 artifact ID/Hash 查询，不生成新 ID 重传同一逻辑对象。内容去重只能在同 Project 和相同 classification/policy 内进行，禁止全局跨项目内容侧信道。

## 5. 存储层

MVP 支持本地 content-addressed store；小型协议 JSON 和文本摘要可以进入 Git，大型内容留在 Artifact Store。后续可增加 S3-compatible/remote backend，但 Descriptor 语义不变。

Artifact Store 必须实现 Project scope、路径 canonicalization、配额、完整性校验、备份策略和访问审计。数据库只保存 metadata/locator，不把大二进制塞入 Coordination transaction。

## 6. 分类与敏感数据

- `PROJECT`：普通项目产物，仍不得跨 Project 默认共享。
- `CONFIDENTIAL`：包含非公开源码、详细日志或用户资料；限制下载与保留期。
- `RESTRICTED`：可能含认证痕迹、安全事件或高敏感内容；加密、最小访问和人工删除 Gate。
- Secret（API key、cookie、私钥、可复用 token）禁止作为 Artifact；检测到后立即隔离和 Attention，不以“Evidence 需要”为理由保留明文。

日志进入 Artifact 前执行结构化 redaction；原始未脱敏日志尽可能留在 Host-local 受限 spool，并设置短保留期。

## 7. 保留与引用

Retention Policy 按 Project、分类和对象类型声明最短/最长时间、配额、是否需要备份及删除审批。以下引用默认创建 HOLD 或延长保留：

- 未完成 Task/Assignment/Gate
- Accepted Fact/Result 的必需 Evidence
- 活跃事故、争议或人工审计
- 未完成 Session handoff/reconciliation

删除前做反向引用检查；UI 必须说明删除将使哪些 Gate/Fact 变成不可复验。必要证据丢失后，相关事实的当前可验证性应显示 `EVIDENCE_UNAVAILABLE`，不能继续显示无条件 PASS。

Context 对象沿用同一 Artifact/HOLD 合同，不另造删除权威：

| 对象 | GC owner | HOLD 触发 | 非权威 UI |
|---|---|---|---|
| ContextEntry / CollaborationProposal 附件 | P36 消费 P15 CAS | 活跃 Task/Gate/Fact/Snapshot/Handoff/incident/restore | Feed 卡片与 Inspector |
| Snapshot / Handoff / LoadProof digest | Controller 元数据 + P36 附件 | 未完成 Session 轮换或 reconciliation | Handoff 卡片 |
| Instruction / Fact 必需 Evidence | Canonical Fact retention | 未 SUPERSEDED 的接受事实 | Inspector Facts |

transcript 默认既不是 Artifact 也不是 Evidence，不得因“需要审计”自动入库。`ReactionSignal` 无领域 retention。MVP 不提供批量递归清理。

## 8. 删除协议

删除是独立授权动作：

1. 创建 deletion proposal，列出精确 artifact IDs、scope、引用和 backend。
2. Policy/Gate 检查 HOLD、活跃任务、备份和跨 backend 副本。
3. 进入 `DELETION_PENDING`，停止新引用。
4. 各 backend 返回删除回执；无法确认则 `DELETE_OUTCOME_UNKNOWN`。
5. 全部确认后保留 tombstone，不保留内容或可恢复 secret locator。

MVP 不提供批量递归清理按钮。Session/工作区回收也不得连带删除已经登记的 Evidence。

## 9. Evidence 可验证性

Gate 读取 Evidence 时验证 Project scope、descriptor revision、content hash、access result、source revision 和 freshness。外部 URL 只能作为补充；关键验收证据应有稳定 Hash 或本地副本策略。

截图和自然语言报告证明“观察到什么”，不自动证明底层命令成功。Exit code、测试报告、diff、commit 和环境信息应分别记录，避免把一份摘要当作全部事实。

## 10. Observability 与成本

记录每 Project/Task/Agent/Attempt 的 artifact bytes、日志 bytes、保留量、上传失败、redaction 命中和删除积压。用量估计与供应商账单不是同一证据等级；UI 显示来源和新鲜度。配额到达时先阻止新大产物/创建 Attention，不丢弃 terminal receipt 或必要 Evidence。

## 11. 备份、恢复与可移植性

备份是版本化 Artifact/Store 操作，不是简单复制运行目录。一个可恢复 bundle 必须固定 Coordination Store snapshot、Control Repository revision、Artifact manifest/content policy、Project/Role/Workflow 配置、schema/migration 版本和完整性摘要。

portable backup bundle 默认加密且使用 authenticated manifest/content digest。包密钥来自用户明示管理的 passphrase/recovery key，不写入同一 bundle；DPAPI 可保护本机密钥缓存，但 DPAPI-only 包必须标为 Host-bound，不能宣称可移植。

portable bundle 默认排除 secret material、CredentialHandle 解析值、token/cookie、Harness 登录态、native Session cache、活进程身份和未明示选择的 `RESTRICTED` Artifact。support bundle 是独立的脱敏导出，必须在用户可见 manifest 预览后生成，默认不包含 Context/prompt/transcript/repository content、账号标识和 secret。

CredentialHandle 的 Host-local 解析、OS principal binding、加密密钥、Harness 登录态、native Session locator 和活进程身份默认不可移植。恢复到新 Host 后必须显示 `REAUTH_REQUIRED | REBIND_REQUIRED | UNAVAILABLE`，不得因为 metadata 存在就显示 READY。

恢复先在隔离位置完成摘要、版本、Project ID 冲突、Artifact 引用和 migration compatibility 检查，再原子切换；失败不得覆盖当前可用数据。read projection/索引可以重建，但必须显示重建水位。卸载默认保留用户 Project/Artifact 数据；删除是独立授权流程并遵守 HOLD/tombstone 规则。Project 删除提案与卸载是不同动作，不得由安装器或 UI 关闭触发。
