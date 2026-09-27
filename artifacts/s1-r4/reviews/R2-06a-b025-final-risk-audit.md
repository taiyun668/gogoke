# R2-06a / b025ba44 最终全轴风险审计

审计席位：`risk_auditor`，只读独立复核；固定生产源码 `b025ba44659da52a2747c7c0a281c5bbd7d4b732`，覆盖 R1–R8 与 Claude `b867dcaf` 的 F1/F2。结论：**未发现新的已证实 P1/P2；可继续已授权的隔离 Win11 测试。候选未接受，全轴 PASS 不成立。** 审计席位没有修改文件、安装、签名、清理或作 Owner acceptance。

## 实际核对的原始证据

| 对象 | 精确身份与核查 |
| --- | --- |
| [桌面构建](https://github.com/taiyun668/gogoke/actions/runs/36302148604) | `b025ba44`，attempt 1，五个 job 成功；冻结／复现双车道六类资产逐字节一致。受信签名使用冻结车道 setup；两车道 setup 本身不同，不在六类复现断言内。 |
| [源码卫生](https://github.com/taiyun668/gogoke/actions/runs/36302148207) | `b025ba44`，11 tests，24,550 文件，0 leaks。 |
| [受信候选签名](https://github.com/taiyun668/gogoke/actions/runs/36304375541) | `main@d0e739be`，`workflow_run`，成功；environment 当前 custom branch policy 唯一允许 `main`，无逐次 reviewer。签名候选 ZIP SHA-256 `79ed1d86857f04ae44a27cc5842164e857d2380a6cc7dabcaf2809995dd8f77f`；setup SHA-256 `2419d848017694e6819eb4e8dcd1004116e0a83e4ad4d2cde003b9d8448eb70d`。 |
| [实装烟测](https://github.com/taiyun668/gogoke/actions/runs/36304535209) | workflow revision `6bbf7593`，实际执行代码与 `b025ba44` 一致。正向 JSON SHA-256 `a5dbe25aaf15a870e1a4023001ea456b2c49f15c7bc913b52c507d39dc5bd3da`；负测 JSON SHA-256 `770ba7517a3381151f4f424d93e7f5384e7db304e84f73f9f25ac1e9597c0746`；全部实际执行成功。 |

审计席位独立复算 frozen、repro、comparison、CI-only installer、signed、正负 smoke、双车道 custody 共九份原始 ZIP 的大小和 SHA-256，均与 Controller 取得的 GitHub API metadata 一致。原始 API run/jobs/artifacts JSON、logs ZIP、九份 artifact ZIP 及当前 environment/branch-policy 快照保存在 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-b025-audit-evidence` 等同任务证据目录；审计席位独立回读本地原件，未声称自己直接重新访问 GitHub。生产 finalizer 的完整字节恰好一份嵌在候选 exe 中，**不含** CI 删除暂停。资源索引、生产 pack verifier、用途绑定 P-256 验签和三种拒绝控制均核过。

## 风险轴结果与实际边界

| 轴 | 本轮观察 |
| --- | --- |
| R1 字节／来源 | 六类资产两车道相同；signed 与 frozen 七个共同资产逐字节相同。`3bd831e6→b025ba44` 的资源包有 25 个 frontend 条目变化，portable／installed shell、native host、Node 字节未变。专门的版本变化实验未执行。 |
| R2 资源信任／加载 | 固定公钥、候选用途与正式域隔离、实例核对、同一打开对象验 hash 后供给、RuntimeLease 和 Node 模块路径限制已核；实装 poison 被服务以 78 拒绝，无执行标记及 readiness。 |
| R3 安装／卸载登记并发 | leaf 插入被拒且 sentinel 保留；同域不同父目录的 B 在 A 持锁时退出 2、未进入安装屏障。F1 测试在生产 finalizer **内存副本**的最后 `Assert-Root` 与唯一 `DeleteSubKey` 间暂停，B 同样退出 2；A `DELETED`、登记锁释放后，B 安装成功并正常卸载。它证明这一具体删除窗口，不冒充两个真实已安装产品的完整端到端竞态实验。 |
| R4 进程／句柄 custody | 两条 Windows 车道执行 owner hard exit 与取消后的实际 held-file 测试；机器回执证明 root/child 在精确 Job 中、child 持有任务文件、owner 返回后文件／目录一次性删除成功。child 句柄采样 258 仅按既定 C07 窄标准解读。 |
| R5 卸载／保留 | 真实候选 `gogoke.exe --uninstall`、继承锁见证、实例和打开对象核对、数据 sentinel 保留通过；本轮 finalizer 用时 83 秒。正式快捷方式启动与正式发行域实装尚未执行。 |
| R6 CI／签名 | 受信签名 workflow 在 `main`；唯一 environment branch policy 为 `main`。冻结产物上传后才单独重打包 CI-only 安装器；两个 setup 哈希不同，仪器与 run／source／shell 身份相符。 |
| R7 产品入口 | 云端实装 WebView → Tauri IPC → service → native-host、准入与资源 readiness 通过；结果为 `TEST_FIXTURE_NOT_ADOPTED`。本轮未证明 Owner 本机测试账本写入或任何真实 adoption。 |
| R8 平台／裁决 | **Owner Win11/SAC NOT_RUN**；Owner 正式签名后的正式发行域实装、最新源码外部 Claude 复核均未完成。准备单 `artifacts/s1-r4/R2-06a-owner-win11-isolated-prep-b025.md` 已重绑精确候选字节。 |

F2 的 NSIS `?e` 捕获后只将 Windows 错误 32 报为同域竞争；云端负测要求仅该分支产生的 `lock-busy` 标记，故错误 32 路径已实际观察。**非 32 分支未单独动态触发。** 原 `36300743274` 的 90 秒屏障失败根因仍未知；本轮三个屏障分别在 62.7、61.4、59.0 秒到达，不据此倒推旧失败。

MC-107 的 WCS-26/F1 低信任 namespace／ACL 前提及同对象 A/B 动态实验仍为 `NEEDS_REVIEW`；当前 RuntimeLease 证据不证明所有 native 入口安全。malformed UTF-16 身份与跨主体 mutex 可用性仍按既定范围处理。同账户候选隔离不是 OS 沙箱，不证明候选 JS 无法访问同账户文件、网络或凭据。

下一步：控制席位刷新本机基线，按新准备单只对最终字节执行一次 Win11 隔离安装／产品测试；异常时停止、保全现场。外部 Claude 精确范围为 `a3b6f85e..b025ba44`，由 Owner 转交，结果按并行收件箱纪律分类。PR #27 保持 Draft，PR #34 保持 Owner-held；不开始 R2-06b，不作正式签署、公开发布或系统安全设置改动。
