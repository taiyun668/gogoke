# ADR-0007：Windows Control Plane 与 Runner 执行能力分层

- 状态：Proposed（v0.2 设计候选，待产品所有者评审）
- 日期：2026-08-11

## Context

产品首先服务 Windows 用户，但部分 Harness 的 PTY/tmux、sandbox 或后台执行能力只在 Linux 更成熟。承诺“Windows 完整支持所有 Harness”会把控制面可用性与执行能力混为一谈。

## Decision

Windows 是首个 Native Control Plane 验收平台。Runner 分成 Windows Native SDK/headless CLI、WSL 和 Remote Linux execution profiles；每个 Adapter/Host 对每个 profile 独立出具 CapabilityEvidence。

Windows Job Object 只被描述为进程治理，不被描述为完整 filesystem/network/credential sandbox。需要 Linux PTY/tmux 的能力使用 WSL/Remote profile，并验证路径转换和隔离边界。

## Consequences

- 本地面板、协调数据库和 Git 协议可原生 Windows 运行。
- 某 Harness 缺少 Windows 能力不会拖垮其他 Harness。
- UI 必须暴露执行位置与退化项。
- WSL 路径错配、Windows/WSL 凭据与文件权限成为必测边界。
