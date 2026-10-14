# 逐案例临时 HOME 收尾：同 PR94 的合同与共用执行流程

状态：授权提案。Owner 要求与直接事实版本准入放在同一 PR94 由 Claude 复核合并；审定、实际 main 回执读回和受影响云检完成前，不执行新的 HOME 关闭或放宽 Busy。这里修订测试流程，产品接线及共用调用器按本次精确写域施工后，才允许新的创建型案例运行。

## 一个独立 F 动作

沿现有认证 USER 管道发送 `gogoke.37.operations.v1`，family 为 `K-INSTANCE`，operation 为 `temporary-home-close`。`targetId` 是原 instance，`domainId` 是原 domain；新的唯一 `requestId` 只属于本次 F 关闭，`expectedRevision` 为被选 HOME 的 F revision。payload 严格含原 `sessionId`，以及二选一的 `originalCreateRequestId` 或 `historicalHomeId`；不得同时给两个 selector 或加入额外字段。

案例收尾只使用自己在 dispatch 前记录的原创建 requestId。历史清单模式只用于 Owner 本次授权的逐项历史目标；homeId 只是查找键，不能当证明。native 在同一 ProductDatabase/RootLock/OwnerIssuer 从原 F create operation/raw 指纹反查唯一 HOME，复核原 domain/session/generation/instance、创建 request/raw、原 H/C 来源、当前使用及合法后继代、profile SID、marker、Root/DB/HOME FileID、原 namespace 与 row revision。历史模式也必须能反查唯一原创建 journal；缺失或歧义拒绝。

只有两类证明：原 H STOPPED/StopFact 与原 C STOPPED/StopProof 精确同源；或 H 直接取得并构造的、完整原执行集合中每个原 PID/creation 身份均已消失的 typed 事实。可信内核重启截断须满足 PLAN 中同样严格的原执行集合及来源资格。USER 不提供 PID、StopFact、路径、SID 或 `gone=true`；超时、宽限期、外层关窗、release、alias 文本或 D 清单均不构成证明。PID 可靠重用只能证明旧精确对象已消失，不得操作新进程；查询拒绝、身份不清、未知后代或原集合缺失均拒绝。

本动作只将确切合格的 F `ACTIVE(1)` HOME 写为 `CLOSED(2)`，原目录及 marker 留在原位。新 close request/raw、F target、证明种类/来源/精确对象、前后 revision、效果与独立 F receipt 在同一事务提交。原停止证明分支保留既有 v1 close receipt/readback；直接消失分支用严格且无歧义的版本化 F receipt，不冒充原 StopFact。COMMIT 未知时保留原 close request/raw，只核原 journal/receipt；不换 ID 重做。APPLIED/REPLAYED 均重新验证原来源、身份和当前使用。

原 H/C UNKNOWN、claim、episode、stop/release、业务结果和错误不改。没有物理 `CLEANED`、目录/alias/worktree 删除、权限修补、凭据读取或长期注册/认证 HOME 改动。关闭一个 HOME 不会结算其它 holder/intent，也不自动放行版本守卫。没有原执行身份的“从未启动后 release”不是空集合 gone；缺原 StopProof 时继续拒绝，本提案不引入第三证明族。

## 案例创建和 finally

1. 共用调用器在每次原 native 创建尝试前持久记录 case、精确 domain/session/instance、原 requestId/raw/hash 与生产者关联。涵盖 admission-reserve、每代 resume 及该例触发的其它 native 创建；随后失败或 UNKNOWN 也不能从清单抹掉。不能用路径前缀、时间区间或当前 claim 推定旧 HOME 所有权；没有可核原生产者关联的创建路径不得启动新案例。
2. 每例使用 `try/finally`。finally 先沿原合法 H 路径取得原 stop receipt 或读取已提交 stop 的原结果；原 stop UNKNOWN 不再发新 ID 或重放业务输入。只有在未发送 stop 且原合法停止仍适用时才发一次原停止动作；不强杀、不造 StopFact。然后逐个以自己记录的 `originalCreateRequestId` 发独立 F close，原 frame 在 dispatch 前持久化。
3. 记录每个原 close request/raw 与独立 receipt。中断或 UNKNOWN 不自动重发、换 ID 或把失败重命名为成功；交 Controller 核原 F journal。原业务错误是首因，HOME 收尾错误单列，不覆盖它。一次关闭失败仍记录其它互不依赖目标的结果，但不把不完整案例写为完成。
4. 在宿主仍可调用时完成已具证明的 HOME 关闭，再调用现有共用正常 caption close；正常退出、READY、StopFact、十三项保护和闭库事实都调用已有共用模块，不各例写 SQL、计时或 ready 判据。正常 close 0/0 本身不代表 HOME CLOSED。
5. 共用闭库读回按原创建 anchors 核本例确切 HOME 集合及原 F close receipt，要求全部原生 CLOSED；未发起创建的例子才可原文记录无创建。发起过创建但原 F intent/物理结果不确定，不能靠缺回复声称没有 HOME。核其它案例的 HOME、长期认证 HOME 和原十三项保护不变。

只有“原业务结果如实记录且所有自有 HOME 收尾完整”的案例才可进入完成/复核状态；缺源、缺证明、未迁移调用路径或拒绝关闭，一律 `CASE_TEARDOWN_INCOMPLETE`，不能以 skip、业务预期拒绝、正常关窗或保护读回等同通过。原对象及 UNKNOWN 保留，Busy 不变。这个失败状态不是所有 HOME 已关闭的承诺。

现有固定共用模块负责观测；新增 `installed-temporary-home-lifecycle.mjs` 只负责原请求所有权清单与 finally 编排，`product-cdp.mjs` 只接原 USER ingress，provider/visible 案例接统一收尾。未改造的私有历史脚本保持原证据，不再作为新创建型案例调用器。新版调用器以实际加载文件/hash 和当前安装身份绑定，不能拿旧成功脚本作为替身。

## 当前逐项快照与验证

[HISTORICAL_TEMP_HOME_PROOFS.json](HISTORICAL_TEMP_HOME_PROOFS.json) 列 297 个完整 HOME ID：OpenCode 9、Claude 24、Codex 264。183 项只是现有投影的核心停止 tuple 匹配候选，19 项投影没有完整停止 tuple，95 项须追溯未投影的旧 episode；缺投影不等于没有原证明。全部仍 `REFUSED_PENDING_DIRECT_PROOF_AND_SOURCE_LINKS`，精确身份消失资格均未运行。清单不作 native proof、不授权关闭；每项还须当前原生原链、物理身份和修订检查。

原 StopProof 正例与 qualified exact-gone 正例须经真实产品接点；错 home/domain/generation/原 raw、部分创建、活或不能查询的原身份、PID 重用、未知后代、错误 StopFact、原 namespace、Root/SID/marker 漂移、新使用、换 request、commit-unknown、跨例关闭及物理删除均应保持拒绝或原结果。用实际生产 mutation 验证移除 proof/接点会失败。受影响 native 检查在云端，JS/TS 使用签名 Node；fixture、源码复核、跳过及 NOT_RUN 都不是已安装真机通过。
