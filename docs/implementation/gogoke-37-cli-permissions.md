# Gogoke 37 固定 Codex CLI 权限

Owner 授权固定 Codex CLI 的 `account/read` 和 H 模型会话取得 Windows `lpacIdentityServices` capability。它允许 AppContainer 调用 Windows 身份及 SSPI 安全包服务，以初始化 Schannel；它本身不是网络许可，也不能据此推断未知 Windows 服务的完整可达范围只等于 TLS。

Owner 于 2026-10-01 裁决：仅由 Owner 发起的固定 CLI 登录阶段使用与宿主同一用户的普通进程，不在 LPAC 内。原位普通 OAuth 的 localhost 回调曾在现有 LPAC 组合中连接超时；采用 gogo-party 已有的登录方式，不修改 CLI 或系统网络隔离设置。登录进程只运行固定登录命令，不接收或执行模型输出；宿主仍保管进程、会话、取消、原始错误和结果，并自己打开授权网址。凭据由 CLI 写入原注册实例：`HOME`、`USERPROFILE` 和 `CODEX_HOME` 指向该实例自己的 home；保留原 `LOCALAPPDATA`、`APPDATA`，使浏览器使用原有用户环境。宿主不读取、写入或复制凭据。

登录阶段拥有普通当前用户的文件访问权限；实例 home 环境与 Job 保管不构成 LPAC 文件隔离，没有提权或跨用户授权。此例外只适用于已准入的固定 CLI 登录，不能用于模型会话、模型产生的命令或其他程序。登录后的自动 `account/read` 及所有模型会话继续在原 LPAC 内，`lpacIdentityServices` 的边界不变。

| 路径与档位 | `lpacIdentityServices` | `internetClient` | 文件目录范围 |
| --- | --- | --- | --- |
| Owner 固定 CLI 登录（普通用户进程） | 不适用，不在 LPAC 内 | 不适用，使用当前用户网络权限 | 普通当前用户文件权限；凭据 home 固定到原注册实例 |
| 自动 `account/read`（LPAC） | 是 | 是，沿用原有授权 | 仅该实例已绑定 home、运行目录与固定 CLI 程序 |
| H `READ_ONLY` | 是 | 否 | 实例及会话 home 可写；绑定工作树只读 |
| H `NO_NETWORK` | 是 | 否 | 实例及会话 home 可写；绑定工作树只读 |
| H `ISOLATED_WRITE` | 是 | 否 | 实例及会话 home、绑定工作树可写 |
| H `NETWORKED_WRITE` | 是 | 是，沿用原有授权 | 实例及会话 home、绑定工作树可写 |

LPAC 路径的公网访问仍受已有 `internetClient` 与 permission tier 限制。此次没有扩大 LPAC 文件目录授予范围、改变 Owner 默认 profile，或合并其他实例的 AppContainer SID。`registryRead` 保持原有行为。未标记 CLI 模式的独立宿主启动请求保持原能力；LPAC CLI 后代继承相同隔离 token 和 Job 约束，包括这项身份服务许可。LPAC 启动时从 host-owned 布尔模式重建固定 capability 列表，并在 suspended child 上核对 AppContainer SID、capability SID、数量和有效启用状态后才允许 admission。普通用户登录例外不使用这套 capability 列表，仍保留 Job 与原实例身份绑定。

固定 CLI 0.149.0 的非登录 Doctor HTTPS 检查在原 profile 中报告 TLS handshake/cert validation failure；同一 CLI、SID、home 和显式环境仅添加 `lpacIdentityServices` 后，指定网络检查取得实际 HTTP 405。同一 SID 环境的 system curl 也从 `SEC_E_SECPKG_NOT_FOUND` 变为 TLS 成功与 HTTP 400。这些对照确认该 HTTPS 路径需要这项身份服务许可；Doctor 的其他诊断并未整体通过，且外部测量未加载产品的路径兼容模块，不能替代正式产品流程。修复仍须经过云端原生隔离测试和实际签名候选装机回读，之后由 Owner 完成真实登录；跳过的检查不算通过。

目录 ACL 的授予及进程激活前核验保留严格的根和后代检查。进程激活后的操作核对当前 Owner、席位、实例、会话、版本和操作绑定，并核对同一物理根的身份与继承 ACE、固定程序和 Git 元数据绑定；CLI 在获准目录中正常创建、删除或改名的运行文件可以变化。根级证据只证明该根，动态后代没有整树实时合格声明。
