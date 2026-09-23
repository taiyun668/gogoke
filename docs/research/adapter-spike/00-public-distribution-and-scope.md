# 公众发行物、源码和试验边界

## 1. 供应链固定

| Harness | 公众发行入口 | 实测版本 | Windows x64 SHA-256 | 签名 |
|---|---|---:|---|---|
| Grok Build | `https://x.ai/cli/install.ps1` / `https://x.ai/cli/stable` | `1.0.0 (3cd0d0cbce)` | `B238FE6B380074849C3A718B5989176559E698E4A8F5D27F11BFF936E92585D1` | Valid，X.AI LLC |
| Codex | npm `@openai/codex@0.147.0` | `codex-cli 0.147.0` | `935A1911ED2556E4FFCEC995F4886AC2AC425863BA26FED264DF62E30272AD9D` | Valid，OpenAI OpCo, LLC |

试验没有执行 Grok 的安装脚本，因为脚本会修改用户级目录、配置和 PATH；而是直接下载安装脚本所指向的官方 Windows 二进制到临时目录。Codex 使用 npm 公共 registry 在临时目录安装当时的 `latest=0.147.0`。

## 2. 源码固定

- Codex：`openai/codex`；release tag `rust-v0.147.0` 指向 commit `be6e8eac029b183056b7e4402879f15d2c85f61b`。
- Grok Build：`xai-org/grok-build` 公开镜像；审计 checkout 为 `b13fa526f5112c0b20dad5f1f2300d3d3b127895`，仓库 `SOURCE_REV` 为 `a51a1dc62fe20029ac39a665985bba78edbb870f`。

Grok 公开镜像没有对应 `1.0.0` 的 release tag，二进制报告的短 revision 也不同于镜像 `SOURCE_REV`。因此 Grok 的“源码机制证据”和“发行二进制实测证据”必须分别记录，不能声称已证明 byte-for-byte source provenance。

## 3. 执行边界

试验分四层：

1. `SOURCE_VERIFIED`：读取公开源码和用户指南。
2. `BINARY_PROBED`：版本、签名、帮助、doctor 和 schema 生成。
3. `PROTOCOL_HANDSHAKE`：隔离 Home/Workspace，不登录、不提交模型请求。
4. `REAL_REQUEST`：使用用户已经存在的公众版登录态，凭证只由目标进程读取；不复制、不打印、不写入仓库。

真实请求都在临时工作区、只读权限、单 Turn、无工具副作用的条件下执行。Grok 以 `GROK_AUTH_PATH` 将现有公众登录凭证与隔离 `GROK_HOME` 解耦；Codex 使用标准用户 Profile 加载认证，并创建 `ephemeral` thread。任何浏览器登录、MFA 或设备码流程均未启动。

## 4. 隐私处理要求

原生协议会产生不应进入项目事实仓库的用户级信息：

- Grok 认证响应可能包含账号、团队和订阅元数据。
- Codex 可能发送账号限额、插件、MCP 和用户配置告警。
- 两端的流式事件都可能包含 reasoning/thought、完整用户输入和 Harness 私有会话内容。

Adapter 必须在 host-local Privacy Firewall 中先分类和脱敏，再生成 NormalizedObservation。默认不得把认证响应、账号限额、完整 transcript 或 reasoning 写入 Git、共享日志或跨项目事件流。
