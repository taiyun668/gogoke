# M2：0.1.64 换装与 OpenCode 初始化续测

## 已完成

0.1.64 的受影响云端检查、原冻结资源签名及验签通过。普通 Win11 视图正常卸载旧候选并安装同一冻结字节，安装后十三项正式保护相等，全部旧 H/C/映射记录连续。没有公开发布、代码签名或自行验收。

旧 OpenCode 初始化失败的 H 仍为 COMMITTED，原进程操作及 StopFact 为空；其席位仍为 BUSY。原记录和错误保留，持有者消失不记作 STOPPED。本次使用同一原实例、同一固定 CLI 与原 Grok 来源，在生产 USER 通道创建新的测试席位、工作树，再验证新的 H；不新增登录或修改旧数据。

## 测量修正

闭库直接字段证明指定秘书长席位为 BUSY。前置仪器错误要求它为 IDLE；现保留该原值，启动后必须核对同一当前 H/thread 并取得真实 stop 与读回，之后才允许 USER 修改，关闭前再次核对。OpenCode 页面聚合 ERROR 原样保留；自管来源资格来自当前闭库的原 MANAGED/READY、登录、home、物理身份和文件字节，不伪造 UI 状态。独立聚焦复核无阻塞，原仪器失败日志保留。

## 当前结果与下一步

新 USER E/F 闭库读回为 IDLE、REGISTERED，原自管来源 READY、登录及 home 身份一致。随后新的生产 H open 返回实际 session ID；stop 与 admission-release 均为 APPLIED，原 H/C 的 StopFact 一致，映射记录 REMOVED，原名称查询不存在。shell/native 正常退出均为 0，十三项保护相等，旧失败 OpenCode H/映射保持原样，原 C 的 UNKNOWN 恢复变化单独记录。该初始化及收尾用例通过实测，未发送模型问题。

秘书长同一 H 的真实冷恢复会推进代次并复用原 history，原 read/probe/stop 回执与新的 H/C StopFact 逐项关联；只有这个已核对的恢复变化从旧记录不变断言中分出，其他原 H、映射及元数据仍逐项核对。四项内存负例确认外部 H、错误 C StopFact、外部 alias 和错误 predecessor 被拒绝。原测量失败和原件保留，未重放已完成的 USER 或 H 操作。

通过独立聚焦复核的 OpenCode、V08 自管来源测试工具已集成，未改安装中的产品字节。继续原 Grok 来源的一次真实提问，以及 V08 双席审批和剩余 Host/升级用例。模型提问、完整 M2/M3、稳定点完整门槛与 SAC 开启后的实测尚未通过。云端等待使用 `gh run watch --exit-status`，结束后在本轮继续。

## 参照

沿用此前真实 USER E/F 的 ActualProduct 入口、当前原数据库只读物理身份绑定、秘书长真实 H stop/read、原冻结字节正常换装和十三项正式保护比对。测量前提由闭库直接字段证伪后修正；未新增恢复标记、凭据读取或权限许可。

## Original OpenCode question readback correction

- One actual Grok-source OpenCode question received H APPLIED/end_turn, real stop/release, normal shell/native exit zero and unchanged thirteen protection fields. The first immutable reader failed because it queried candidate-generation step names in the original H path. The original FAIL journal and once claim remain unchanged; no model request is replayed.
- Reference: production original H in `store/product_database/v37_runtime.rs` writes `setting-model` and `setting-effort`; `store/session_transport/rpc_journal.rs` adds an operation prefix only in the candidate-generation path. An ordinary immutable scoped read found one original ACK for each setting and zero prefixed rows. Both exact option values and response identities match, with database bytes preserved.
- Fix: query the original step names, retaining full custody tuple, unique ACK, exact option and order assertions. The existing supplementary reader can bind this single OpenCode readback failure using the unchanged original journal and executed reader digests. Independent focused review found no blocker. Full answer/golden supplementary readback remains pending; M2 remains incomplete.
- Parallel measurement: replacing per-file Get-FileHash with .NET streaming preserved all five formal projections but measured 91.6 seconds versus 56.6 seconds for the original. The proposed optimization is rejected; original observer remains active. No product bytes changed.

## Refusal and next ordinary question

The scoped original ACP read confirms a real assistant refusal: "I can't reply with that exact marker." It ended the same typed prompt normally; the model and effort original ACKs match. This is not an omitted answer or an inferred success. The original marker case and supplementary failure remain FAIL, with no golden success credit and no replay.

Reference: reuse the existing Claude natural arithmetic question and independent-number answer check in `m2-provider-win11.mjs` and its immutable reader. New OpenCode questions use that same exact ordinary question; original marker evidence remains supported. The new case retains original prompt/end-turn correlation, no-tools, actual managed executable, exact model/effort ACK, stopped custody, normal close and protection gates. The returned vendor session field now comes from the already verified original session/new ID. Fresh focused review of the two tools found no remaining blocker. This is a test detail within the authorized tools scope; no product/CLI bytes or permission changed. A new H/request is required and has not yet run.

V08 preparation continues in parallel. Its current ordinary closed read confirms the actual Codex managed copy, original login/home, policy template, Owner/host capacity and absent new test identities; this remains preparation, not V08 product credit. It must be rebound if another product launch changes the closed database.

## Actual natural question complete

On the unchanged installed 0.1.64 candidate, the original managed OpenCode instance with xAI/Grok answered the new arithmetic question with the original agent text `778`. Immutable readback confirms matching original prompt/end-turn, exact model/effort ACKs, executable and home, stopped custody and released H. Shell/native exited normally with zero, all thirteen protection fields are unchanged, database bytes are preserved by measurement, and one protocol golden remains REVIEW_REQUIRED / NOT_ASSESSED. No prior request or failure was replayed or rewritten. This satisfies the one-question technical checks, not M2/M3 acceptance.

The current post-question closed qualifier was read again, with actual Codex managed source/home, policy template, capacity and absent V08 test identities. The original six USER operations completed: each of two seats was created, bound to the instance and given orchestration bounds 4. These operations do not register F. The next ordinary immutable read verified the six original request fingerprints and result snapshots, the same seat incarnations and revision chains, current managed Codex source/home, genuine Secretary H/C stop and both new worktrees still absent. Database bytes and four physical identities were preserved. The unmodified approval core is bound to that new closed read; it will create F itself. Core execution and Host busy-to-idle remain pending.

F.2 lifecycle work proceeds independently in its own branch/worktree. Current trusted catalogs have only one version per driver; there is no honest two-version upgrade/rollback success credit. Version-key selection and guarded switching interfaces must preserve existing home/credential bytes and reject active/UNKNOWN use. Candidate swapping does not supply CLI upgrade credit. Stable full chain and SAC-enforced checks remain pending.

## V08 measurement failure and fresh continuation

The first complete approval flow reached its final cleanup, but Root polled the live atomic JSON journal using an ordinary file reader. The writer then returned `EPERM: operation not permitted, rename` while saving before dispatching the stop request. The original journal remains FAIL; the unsent stop was not replayed. This was a measurement fault caused by Root, not evidence that the product's approval flow passed or failed.

Reference: the existing ActualProduct operation writes the journal before invoking the product. Its monitoring contract already uses append-only stdout and reads the JSON only after task completion. That existing practice is now followed. New original requests obtained capability proofs, actually stopped and released both existing H sessions, and stopped/read the same Secretary H. The product and native host exited normally with zero. A closed read confirmed all thirteen protection fields and genuine H/C stop/release facts. The original failed case and all records remain unchanged; this cleanup does not turn the failed case into PASS.

A new case uses new E/F and request identities with the original six USER payloads. All six applied, both E revision chains and bounds 4 were read back from the original native records, both new F remained absent, and current managed Codex bytes/home and physical database identity matched. Secretary stop/read, both normal exit codes and all thirteen protection fields passed. The complete original core is now running from those fresh closed facts. Its final approval, rejection, policy and closure assertions remain pending. Host busy-to-idle preparation proceeds separately and must bind this new successful core, never the earlier FAIL.

F's newly integrated version-switch API is still unused. Direct registration code review found that changing the pin also requires a durable witness connecting the immutable registration request to every subsequent real version switch. The manual Codex witness cannot supply that proof for managed four-provider changes. The F worker is correcting this inside the existing instance write scope; the unimplemented product bridge remains gated on Owner's scope merge. A second official OpenCode archive has been downloaded and its integrity, members and executable digest checked without execution or installation. Neither preparation nor this unused API counts as a runnable upgrade/rollback capability.

## 当前 V08 核心实测完成

新一轮原 core 已正常退出 0。封存后的原读取确认 13 次真实模型策略调用、对应原生工具回包和政策账本，包括提交、带原因拒绝、重新提交、批准、过期和上限约束、合法阶段流转。两席分别有 9/4 个真实终轮，原 H episode 与 C 的停止哈希一致，原 stop/release 请求链满足 CAS；两次标题栏正常关闭的 shell/native 均为 0/0。十三项保护的原快照逐字段相等，闭库测量未写库、未读凭据，原文件字节保持。任务按原 custody 核对后清理并确认不存在。旧 FAIL 不改，这一轮没有重放旧请求。

原最终读取仍明确为 V08_INCOMPLETE：跨项目、对 Owner 投递、Host 送达及真实 stalled 链未由这份 core 覆盖。只认当前切片，不自行验收。参照现成 Host 忙转闲入口，新输入绑定本轮成功 journal/final、原两 H 真停止释放及当前 MANAGED/READY 来源，接收者均为新 E/F；该普通视图任务已经启动，最终送达及保护读取待完成。

已用 `gh pr comment` 真正发出这段进展，并回读确认：[PR #54 进度](https://github.com/taiyun668/gogoke/pull/54#issuecomment-6131253444)。普通评论不消费待计入下一实际心跳的新能力，也不重置连续无新增计数。

并行 V06 来源仪器保留原 UI ERROR，复用同一 sealed 来源和同一次 H capability-probe，补齐实际 helper 的运行哈希记录。全新聚焦复核、受影响来源安全云检通过；秘书长真实停止接线及新输入仍待完成，尚未跑真机 V06。Root 首次本机检查漏传明确 scratch 参数，被原 guard 拒绝；原失败保留，补传 D 目录后安全断言通过，没有修改 guard 或测试预期。F 的持久链修订完成集中静态复核，新的原生云检仍在跑，不使用旧提交通过或未覆盖 activation 的元数据夹具证明 CLI 升级。
