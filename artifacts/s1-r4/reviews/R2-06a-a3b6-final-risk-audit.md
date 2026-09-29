# R2-06a / a3b6f85e fresh 最终全轴风险审计回执

**状态：审计已执行；未发现新增、已证实的 P1/P2；全轴 PASS 未成立；候选未接受。** 审计对象是 `taiyun668/gogoke@a3b6f85e38ca0971fdd05096734c2e99aeee7ec4` 的生产源码及其精确云端产物。此文由 Construction Controller 依据 fresh `risk_auditor`（Astra/xhigh）的只读报告持久记录，结论不代替 Controller/Owner 验收。审计结束时施工分支 HEAD `43ebdb446fe98b951bcf88c4befb8244cd3a393f` 相对生产源码只增加 MC-122/MC-123 检查点，生产及 workflow 字节未变。

## 审计对象和来源

- 精确源码 desktop run [`36291782858`](https://github.com/taiyun668/gogoke/actions/runs/36291782858)，hygiene run [`36291794814`](https://github.com/taiyun668/gogoke/actions/runs/36291794814)，受信 `main@d0e739be` 签名 run [`36293506694`](https://github.com/taiyun668/gogoke/actions/runs/36293506694)，已装候选云端烟测 run [`36293571078`](https://github.com/taiyun668/gogoke/actions/runs/36293571078)。
- 审计席位 GitHub API 直接访问受沙箱阻断；Controller 将这四次 run/job metadata、原始 logs ZIP、七个 artifact 原始 ZIP 及 metadata 保存于 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-final-audit`。审计席位独立重算七份 ZIP SHA-256，与 metadata 一致，逐字节核 frozen/repro 和 signed/frozen 共同资产。此为 Controller 取得远端原件、审计席位独立核字节的来源链，不能写成审计席位直接重新访问 GitHub。
- 机器回执 artifact `10923302922`：ZIP SHA-256 `b8d2b58269daba721b644e2ec68d04a031eb6960e71061a281b9f6fabe2b3156`，JSON SHA-256 `47962e4378b076b0088506b857cd8047a15fe6e4eaca0bfe25d92d019472df99`。artifact `10923377342`：ZIP `da1377cfad0fe328d50b30af491ea7a76ddb1ee172a85d66125d56edad137fdd`，JSON `cb74c05fd784552d43879d2be3a2fcd00f5757d6cce6e1264f2b5e5d02d86163`。production setup `13dfcd8cb95394348434f83d0cc610a03cc8c1f88e7b708c41f10692844b53c2` 与 CI-only negative setup `ffc5244ae43f72cbe19b692bd1c53c89bcb3d6580ebb5895fb0e229820884a4d` 分离。

## 全轴结果

R3/R4 保留 MC-121 的审查编号；其他 R1–R8 是本次报告的显式轴映射，不冒充历史计划 ID。

| 轴 | 核查结果与证据层级 |
| --- | --- |
| R1 可执行字节与身份 | frozen/repro 六类实际资产逐字节相同，签名包与 frozen 共同文件相同；当前 shell 仅资源/版本变化的跨提交对照仍 NOT_RUN。MC-092 的对照属于旧 shell。 |
| R2 签名、资源、加载 | 候选签名有效；去用途前缀、换 Owner 公钥、篡改一字节均拒绝；生产 pack verifier exit 0，索引有 1,234 资源文件与 12,979 安装静态文件；云端实装 poison 返回 78，未执行 sentinel。 |
| R3 安装与同域登记 | leaf 插入后 installer 退出 2、原字节不变、无登记；同域跨父目录 A/B 中 B 在 A barrier 期间退出 2、未创建目标，A 完成并登记自身。 |
| R4 进程 custody | 两个 Windows lane 各执行 outer hard-owner-exit 1 passed/0 failed/0 ignored，取消后 custody/held-file 各 1/0/0；测试使用生产 launch，取得精确 Node 句柄再终止 owner。 |
| R5 卸载与数据 | 候选实装卸载、AppData sentinel 保留有云端证据；正式域真实快捷方式 writer、AppUserModelId 和正式安装/卸载 NOT_RUN。现有 finalizer 测试使用合成 `.lnk` 字节。 |
| R6 CI 与签名边界 | 受信签名 run 来自 `main@d0e739be`；`candidate-resource-signing` environment 的唯一允许分支为 `main`。production 与 CI-only setup 的来源和 hash 均分开。 |
| R7 产品入口 | 云端实际 WebView → Tauri IPC → service → native-host 有非零行为证据，结果仍为 `TEST_FIXTURE_NOT_ADOPTED`；真实 GitHub adoption 未被证明。 |
| R8 Owner 平台与异构 | Owner Win11/SAC、正式清单后的同字节安装和 exact-source 外部 Claude 均 NOT_RUN。 |

审计席位独立做 PowerShell 5 文件解析、Node 2 文件语法、Python 5 文件 AST，无语法错误；离线候选签名正负 4/4、可执行身份 4/4。云端原始日志显示 typecheck/npm test 146 文件、1,044 tests passed；两 Windows lane 的 Python pack/frozen 各 30 tests OK，Rust 筛选组各 `9+8+1+1+1+1` passed、0 failed/ignored；finalizer 3 tests OK；源码扫描器 11 tests、24,538 committed files、0 leaks。repro lane 四个按条件跳过的打包/上传步骤没有计为通过。Controller 另外执行提交内 focused Python 30 tests，exit 0、日志 SHA-256 `05a64ac9fcf97fd809eec45d4ab1d5c516a08370c4ade4899bfcf20eabf3ea01`；此项不是审计席位独立执行。审计席位未本机编译/测试原生代码，也未运行 production mutation；旧“三条正式命令”未由本席位原样重跑，改核当前云端 workflow 真实命令及日志。

## 保留风险和结论边界

MC-107 的 WCS-26/F1 仍为 `NEEDS_REVIEW`：低信任 ACL/同对象 A/B 动态前提未验证；已检查当前 RuntimeLease pin Node/fixture，不能将 F1 改成 PASS。WCS-26/F4 malformed UTF-16 与 WCS-27/F2 跨主体 mutex 可用性保持既定范围外状态。R2-06b 完整/资源更新、回滚和崩溃恢复没有进入本轮验收。

**Controller 技术判断**：本次没有需要立即改动 a3b6f85e 的具体新缺陷；可以继续准备隔离候选 Win11 实测，但不能把审计报告写为全轴 PASS。当前 shell 的资源/版本变化云端对照、正式域安装/快捷方式行为、Owner 实机和 exact-source Claude 仍需按原边界补证。任何后续生产字节改变都要重跑受影响云端检查并重新绑定复核。PR #27 维持 Draft，PR #34 仍由 Owner 持有；无采用、签名正式清单、代码签名或发布。
