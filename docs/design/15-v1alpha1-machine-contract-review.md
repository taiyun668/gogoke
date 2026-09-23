# v1alpha1 机器契约总审记录

- 日期：2026-08-11
- 状态：设计候选已机器化；ADR-0009 仍为 `Proposed`
- 范围：Schema、Wire、正反例、摘要向量、Fake 场景清单；不含生产实现

## 1. 本轮裁决

1. per-kind Schema 是不可变语义对象；跨进程统一进入 `WireMessage.payload`，不允许每个 Harness 发明自己的外层格式。
2. `DISCOVER/PROBE` 是 host-scoped；创建、恢复、发送、观察、收集等状态操作必须携带当前 Assignment scope 与 fence。
3. Adapter 只提交 native evidence。`COMPLETED/FAILED/CANCELLED` 的 `NativeTerminalEvidence` 只能以 official protocol 或 fresh official hook 为终态依据；process exit、PTY、transcript、idle 和 heuristic 不能形成权威成功。
4. `NativeTerminalEvidence -> TerminalReceipt -> ReceiptRecord -> Gate` 是四层对象；Schema 不允许 Adapter 直接签发 Receipt，也不把 Gate 结论塞进 Receipt。
5. ContextPack 保留 `included/omitted/conflict/required` 账目；mandatory 缺失或冲突时不得标为 `COMPILED`。Projection 写入和 Harness 已加载是两个证明层。
6. 顶层对象默认封闭未知字段；vendor raw 只允许以受限 Artifact reference 进入证据链。

## 2. 已生成工件

- `spec/draft/v1alpha1/schemas/`：21 份 Draft 2020-12 Schema（含 defs、Wire 与 bundle）。
- `spec/draft/v1alpha1/schema-tests/`：24 个预期结果案例，覆盖 12 种顶层对象；golden 还必须通过统一 bundle。
- `spec/draft/v1alpha1/schema-tests/digest-vectors.json`：Python/Node 共享的安全子集摘要向量。
- `spec/draft/v1alpha1/fake/suite-manifest.json`：54 个 `FAKE_CONFORMANCE_ONLY` 场景清单，仍为 `DRAFT_NOT_EXECUTED`。
- `spec/draft/v1alpha1/validate.cmd`：不改系统 PowerShell policy 的只读一键校验入口。

## 3. 当前证据

本轮本地校验通过：Schema meta-check、所有本地 `$ref`、严格 format、golden/negative 预期、bundle 唯一匹配、外层/内层 Project 检查、scope/fence 检查、重复 JSON member 拒绝、Python/Node 摘要向量、Fake 测试数量与标签清单。

这个阶段的 PASS 只表示 Schema 草案内部一致；当时没有运行 SQLite。后续临时数据库参考验证见 `16-sqlite-machine-contract-review.md`。Codex、Grok、ACP、app-server、Git 副作用、Windows Job、凭据和网络仍未运行，因此不构成真实 Harness conformance 或生产就绪。

## 4. 下一阶段 Gate

1. 把 `13-sqlite-coordination-spec.md` 落为 draft DDL、事务参考模型与并发/commit-unknown 测试；仍先用 Fake Runner。
2. 实现 Fake trace/fault interpreter，使 54 个场景从“清单存在”升级为“可执行 Oracle”，并保持业务 `INCONCLUSIVE` 与测试 PASS 分离。
3. 补完整 RFC 8785 Unicode/IEEE-754/非法输入向量，并在 Rust 与 TypeScript 实现间交叉验证；当前 Python/Node 只覆盖明确标注的安全子集。
4. 进行唯一 ProcessSupervisor 的 Windows descendant containment、spawn/close race、PID reuse、grace-to-kill 和 host-crash bake-off。
5. 最后才接公众版 Codex app-server 与公众版 Grok ACP；任何真实能力都以 doctor/conformance evidence 判定，不能按产品名称推断。
