# Codex + Grok Build 公众版 Adapter Spike

- 日期：2026-08-11
- 状态：Spike 已完成；允许进入 Adapter 实现设计，不代表 Conformance Suite 已 PASS
- 边界：只使用公众可下载的官方发行物和公开源码；不使用私有 Provider、账号池或内部 shim

## 结论

首发组合保留为 **Codex + Grok Build**。两者都具有可用于控制面的原生机器协议，也都完成了真实模型请求：

- Codex 主通道：`codex app-server --stdio`；一次性降级通道：`codex exec --json --ephemeral`。
- Grok 主通道：`grok agent stdio`（ACP）；一次性降级通道：`grok -p --output-format streaming-json`。

两者都不应以 PTY 文本解析作为自动完成依据。当前证据足以开始实现 Adapter 和 Conformance Suite，但权限拒绝、取消、恢复、分叉、断线重连和项目文档加载仍需在实现阶段逐项做负向测试。

## 文档

1. [发行物、源码和试验边界](00-public-distribution-and-scope.md)
2. [CapabilityEvidence](01-capability-evidence.md)
3. [首发适配决策与设计影响](02-mvp-adapter-decision.md)
4. [可复现的无登录协议探针](tools/probe_stdio_protocol.py)
5. [Grok App 复用审计](../grok-app-reuse-audit.md)

探针不会登录、复制凭证或提交模型请求，除非调用者显式传入 `--prompt`；其默认用途是验证公开二进制的握手和无认证失败语义。
