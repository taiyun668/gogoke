# ADR-0006：Adapter 能力必须以证据和契约测试声明

- 状态：Proposed（v0.2 设计候选，待产品所有者评审）
- 日期：2026-08-11

## Context

CLI Harness 更新很快，session、structured output、steering、permission、sandbox 和 Windows 支持并不一致。仅凭 adapter 名称或静态布尔字段会让调度器承诺不存在的能力。

## Decision

Adapter 使用统一内核负责 transport 和归一化，不包含 Workflow policy。能力按 support level、integration mode、evidence quality、enforcement strength、已验证 Harness/Adapter/OS 版本和有效期声明。

Adapter 只有通过与声明能力对应的 Conformance Suite 后才可调度。版本变化或证据过期会撤销已验证状态。Generic PTY 没有机器协议/receipt contract 时不得宣称结构化完成、可靠 resume 或强权限能力。

## Consequences

- UI 会显示真实支持、退化、未知与未支持状态。
- Scheduler 按能力证据匹配而不是按品牌匹配。
- 每增加 Harness 都需要维护 Adapter + Projector + Conformance Evidence。
- 统一接口不会抹平 Harness 的原生差异。
