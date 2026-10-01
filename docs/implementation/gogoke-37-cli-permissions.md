# Gogoke 37 固定 Codex CLI 权限

本轮 Owner 授权固定 Codex CLI 实例会话（包括 device login、account/read 和 H 会话）取得 Windows `lpacIdentityServices` capability。它允许 AppContainer 调用 Windows 身份及 SSPI 安全包服务，以初始化 Schannel；它本身不是网络许可，也不能据此推断未知 Windows 服务的完整可达范围只等于 TLS。

| 路径与档位 | `lpacIdentityServices` | `internetClient` | 文件目录范围 |
| --- | --- | --- | --- |
| Owner device login、account/read | 是 | 是，沿用原有授权 | 仅该实例已绑定 home、运行目录与固定 CLI 程序 |
| H `READ_ONLY` | 是 | 否 | 实例及会话 home 可写；绑定工作树只读 |
| H `NO_NETWORK` | 是 | 否 | 实例及会话 home 可写；绑定工作树只读 |
| H `ISOLATED_WRITE` | 是 | 否 | 实例及会话 home、绑定工作树可写 |
| H `NETWORKED_WRITE` | 是 | 是，沿用原有授权 | 实例及会话 home、绑定工作树可写 |

公网访问仍受已有 `internetClient` 与 permission tier 限制。此次没有修改文件或目录 ACL、扩大文件目录授予范围、改变 Owner 默认 profile，或合并其他实例的 AppContainer SID。`registryRead` 保持原有行为。未标记 CLI 模式的独立宿主启动请求保持原能力；CLI 后代继承相同隔离 token 和 Job 约束，包括这项身份服务许可。启动时从 host-owned 布尔模式重建固定 capability 列表，并在 suspended child 上核对 AppContainer SID、capability SID、数量和有效启用状态后才允许 admission。

固定 CLI 0.149.0 的非登录 Doctor HTTPS 检查在原 profile 中报告 TLS handshake/cert validation failure；同一 CLI、SID、home 和显式环境仅添加 `lpacIdentityServices` 后，指定网络检查取得实际 HTTP 405。同一 SID 环境的 system curl 也从 `SEC_E_SECPKG_NOT_FOUND` 变为 TLS 成功与 HTTP 400。这些对照确认该 HTTPS 路径需要这项身份服务许可；Doctor 的其他诊断并未整体通过，且外部测量未加载产品的路径兼容模块，不能替代正式产品流程。修复仍须经过云端原生隔离测试和实际签名候选装机回读，之后由 Owner 完成真实登录；跳过的检查不算通过。
