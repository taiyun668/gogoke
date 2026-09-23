# Fake Kernel 全库存参考执行总审

- 日期：2026-08-11
- 状态：确定性内存参考内核 `54/54` 执行通过；完整 SQLite-backed Fake 纵向闭环尚未完成
- 标签：`FAKE_CONFORMANCE_ONLY`
- 边界：不接真实 CLI、进程、网络、Git、凭据、Harness 或持久产品数据库

## 1. 本轮落地

`run_fake_conformance.py` 已把场景、fixture、声明式 fault、Transition trace 与五类 Oracle 接成第一套可执行参考模型。执行报告通过独立 JSON Schema 校验，并记录每个 fault 的命中、触发前后状态、authority graph、side-effect ledger 与业务 Gate 结果。

当前执行覆盖为：

| 类别 | 已执行 | 总库存 | 已执行场景 |
|---|---:|---:|---|
| E2E | 5 | 5 | `E2E-001..005` |
| ADP | 10 | 10 | `ADP-001..010` |
| RUN | 10 | 10 | `RUN-001..010` |
| REC | 7 | 7 | `REC-001..007` |
| ART | 6 | 6 | `ART-001..006` |
| CTX | 10 | 10 | `CTX-001..010` |
| ISO | 6 | 6 | `ISO-001..006` |
| **合计** | **54** | **54** | **0 条未执行** |

54 条场景全部得到预期测试结果；配置的 42 个 fault 全部恰好命中，没有配置但未命中的伪覆盖。

此外，9 个 disposable-database bridge case 已把五条 Runner 场景、三条 Receipt 场景以及 `ART-001`、`ISO-001` 的高风险断言送入 SQLite 参考事务：并发/幂等 claim、异 digest 冲突、lease expiry、旧 fence heartbeat/artifact/receipt、COMMIT 响应丢失、竞争终态、Artifact Gate 和跨 Project 外键均按预期收敛或拒绝。它们是交叉验证，不能把 54 条内存 trace 宣称为全部 SQLite-backed。

## 2. 三种运行模式的同源性

三条 E2E 使用不同 Intent 入口：

- manual：`intent.submit.manual`
- one-click：`intent.submit.one-click`
- run-to-human-gate：`intent.submit.run-to-human-gate`

manual 与 one-click 得到相同的权威因果图：

```text
context.ready
  -> assignment.claimed
  -> attempt.started
  -> receipt.committed
  -> result.materialized
  -> auditor.review.accepted
  -> human.decision.accepted
```

run-to-human-gate 是同一图在 Human Gate 之前的精确前缀。它没有另一套状态机，也没有绕过 Context、Receipt、Auditor 或 Gate 权威。

## 3. Oracle 裁决

- `StateOracle` 检查 trace 与最终状态一致，不允许已拒绝事件推进权威状态。
- `AuthorityOracle` 检查 Project、Assignment、Attempt、epoch、fence、幂等与 Receipt 权威链。
- `SideEffectOracle` 要求同一幂等副作用键至多执行一次。
- `EvidenceOracle` 约束 Gate：G0 PASS 必须有 compiled context；G2 PASS 必须有 official protocol/hook terminal、verified artifact 和 Receipt；G3 PASS 必须有 auditor visibility proof 和 auditor Receipt；G4 PASS 必须绑定授权 human decision。
- `IsolationOracle` 检查跨 Project 引用、workspace escape 和 Reveal Gate 前的 forbidden material。

测试结果与业务结果严格分轴。`RUN-006`、`REC-007`、`ISO-005` 的测试均 PASS，但相应业务 Gate 是 `INCONCLUSIVE`；这证明系统按预期停在 Attention/reconciliation，而不是把未知状态伪装为成功。

## 4. 尚未证明

本轮不能声明完整 Fake Conformance：

1. 当前完整 trace/Oracle 仍在内存中运行；只有 9 个高风险 bridge case 驱动 `0001_coordination.sql`，尚未把整条场景 trace 映射到数据库事务。
2. Fault action 是参考语义，没有执行真实 process restart、output spool、Git commit 或 OS crash。
3. 尚未验证真实 SQLite crash/WAL、ProcessSupervisor、Git materialization、Codex/Grok Adapter 或 Windows containment。
4. ADR-0009 仍为 `Proposed`，本轮 PASS 不构成实现主干或依赖冻结授权。

## 5. 下一阶段 Gate

下一步应把 Fake Kernel 的 claim、lease、command、artifact、receipt 和 reconciliation 操作接到共享 SQLite 事务接口，而不是继续扩充一套平行的内存权威。按风险顺序迁移：

1. `RUN/REC` 权威写入与 reconciliation；
2. `ART/ISO` scope、Artifact 与 materialization；
3. `CTX/E2E` Gate、rotation 与 correction/pause 收尾；
4. 完成后再进入 ProcessSupervisor Windows bake-off。

54 条确定执行结果与 fault hit proof 已具备；但文档 14 所描述的 Controller + SQLite + durable spool + isolated Git fixture 运行拓扑尚未实现，因此当前结论仍是 `DRAFT_REFERENCE_EXECUTED / FAKE_CONFORMANCE_ONLY`，不是完整 Fake Conformance PASS。
