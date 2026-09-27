# R2-06a / a3b6f85e Owner Win11 隔离候选实测准备

**状态：SUPERSEDED，NOT_RUN，候选未接受。** 本单原对应 `taiyun668/gogoke` 源码 `a3b6f85e38ca0971fdd05096734c2e99aeee7ec4`。Parallel Inbox `b867dcaf` 的同源 Claude 报告提出 F1/F2，控制席位已在 R2-06a 范围内开始修复；下列旧云端运行、安装包及组件哈希仅是历史证据，不能用于 Win11 安装。须待最终源码重新构建、签候选、通过安装烟测并重新绑定本单后才可执行。原全轴审计的边界见 `artifacts/s1-r4/reviews/R2-06a-a3b6-final-risk-audit.md`，新报告见 `artifacts/s1-r4/control/reviews/claude-r2-06a-a3b6f85e.md`（收件箱分支）。

## 已备好的精确对象

- 双 Windows 构建和原生测试：run `36291782858`；源码卫生：run `36291794814`；受信 `main@d0e739be` 候选签名：run `36293506694`；托管 Windows 安装烟测：run `36293571078`。云端机器回执 artifact `10923302922` 与 CI 专用负测回执 `10923377342` 的 JSON SHA-256 分别为 `47962e4378b076b0088506b857cd8047a15fe6e4eaca0bfe25d92d019472df99`、`cb74c05fd784552d43879d2be3a2fcd00f5757d6cce6e1264f2b5e5d02d86163`；负测安装程序与正式候选不是同一字节。
- 签名候选 artifact `10922689154` 的 GitHub archive digest 为 `sha256:3149291e68d13312c0e434f6a44187c33b0258230e48f79fafe81a70da25d91a`。已在任务临时根下载，并对原始 ZIP 复核该摘要。安装包 SHA-256 为 `13dfcd8cb95394348434f83d0cc610a03cc8c1f88e7b708c41f10692844b53c2`；实际安装后预期 `gogoke.exe` 为 `ea99af8933bbdeb8a8da87db0d1eb7bfdd73503262c3a16855bc0d9051b705cc`，`gogoke-native-host.exe` 为 `842221cb902906e19b439123a638c50f80432edebed0788b75182e7a42c9d267`，`node.exe` 为 `e3be0545990c90995d7bf3a7af5d64af1f2e0fc1bbd9b79c27f7abc1e9676e50`。本机离线复核候选清单用途/来源、索引与资源包 hash、P-256 签名均通过，内存单字节变异被拒绝；尚未执行安装包。
- 准备时本机为 Windows 11 `10.0.26200`，Smart App Control 注册表状态 `1`（强制）。正式版 `0.1.2` 已安装，`gogoke.exe` SHA-256 为 `21549a4f18b3781c68db0d33d41ac83c4be1868fee08c9bcc1180db0229c8dc8`；正式数据有 6 个文件、正式快捷方式有 2 个、无运行中的 `gogoke.exe`。候选卸载登记与候选数据目录均不存在。此为准备时快照，正式测试前必须重取，不能把它当测试后的观察。

## 唯一可选的 Owner 实测触点

| Owner 可选步骤 | 为什么只能由 Owner 做 | 控制席位承担 |
| --- | --- | --- |
| 全部机器检查结束后，可选择看一眼 Home、受控结果和错误是否符合产品目标。 | 只有主观产品判断属于 Owner；按钮执行、机器证据和 Git 回读都可由控制席位完成。 | 运行固定产品测试，记录实际 UI/IPC、子进程、哈希和事件；测试账本结果保持 `TEST_ONLY`，不作采用。 |

测试前由控制席位检查 `gogoke.exe` 与 `gogoke-native-host.exe` 的运行状态；均未运行即视为没有未保存的 Gogoke 工作。若有运行，读取窗口标题和其他可观察状态；只有仍无法判断是否有未保存内容时才向 Owner 提出附建议选项的问题。候选下载、验签、隔离安装、固定测试、取回执和候选卸载均由控制席位执行。Owner 将来保有最终验收权，但验收决定不作为本次实测的前置触点。正式发行 `SHA256SUMS.windows` 的离线签署另属 Owner 独有私钥边界；本次不执行。

## 控制席位实测顺序与停止条件

1. 复核最终源码的审计结论、候选 archive/setup/hash/签名、`main` 授权 blob 与 PR 状态。先查 `gogoke.exe` 与 `gogoke-native-host.exe` 是否运行；两者均无则直接继续，有则读窗口标题等可观察状态，只在确实无法判断未保存内容时问 Owner。重取 SAC 强制状态、Code Integrity 日志起始时间/记录号、正式版 exe/登记/数据文件及快捷方式 hash、候选登记/数据根缺席。保留已有正式安装 `%LOCALAPPDATA%\Programs\gogoke` 和正式数据 `%APPDATA%\app.gogoke.desktop`。
2. 只用已验签候选安装包，在 Owner 当前账户下 NSIS 安装到新的 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-install-a3b6`。安装包同目录仅放候选 sidecar，不能混入正式 `SHA256SUMS.windows`；安装前确保目标不存在、路径及祖先无 reparse。候选 HKCU 登记必须是 `...\Uninstall\gogoke-candidate`，数据根必须是 `%APPDATA%\app.gogoke.desktop.candidate`，实例 ID 与正式版分离；不得触碰正式版根或正式快捷方式。
3. 记录 installer 的进程身份、退出码、实际安装后 shell/native-host/Node 哈希和候选资源代；逐一观察每个组件是否由已安装产品拉起。以真实 `gogoke.exe` UI/IPC 执行受控 fixture，保留真实服务与 native-host 身份、结果与测试账本回读；不要把云端负测工具或便携 exe 当作实装产品。关闭并从同一安装根重开一次，观察已验证资源与结果回读，不把草稿当 adoption。
4. 从记录的起始事件号读取 `Microsoft-Windows-CodeIntegrity/Operational` 的 3033/3077，结合实际 Win32 4551（仅在发生时记录）写明时间、目标组件、安装路径的占位表示与 SHA-256。任一拦截、签名/哈希/登记/实例不符、正式版字节或数据/快捷方式变化、子进程去向不明时立即停止该尝试并保全现场；不重试变通、不关闭或改动 SAC、不本机编译原生代码。
5. 安全完成后由候选安装的 `gogoke.exe --uninstall --quiet` 卸载候选，核其最终回执、候选登记消失、候选数据逐字节保留，正式版 exe/登记/数据/快捷方式仍等于测试前快照。**完成正式版快照比对之后**，再从原正式快捷方式启动正式版一次，观察能否正常启动；启动可能改写正式数据，故不把启动后的数据拿来做卸载前后逐字节比较。候选失败或收尾身份无法证明时不强删，记录 custody 和下一步。MC-122 的两文件证据副本始终保留，等 Owner 明确指示清理。

## 预期残留与清理条件

| 对象 | 预期状态 | 清理条件 |
| --- | --- | --- |
| `%LOCALAPPDATA%\gogoke-registration-CI_CANDIDATE_RESOURCE.lock` | 安装和卸载的同域锁文件可能留存；它不是正式版字节或数据变化。 | 仅在无同域安装/卸载进程、确认无打开句柄及确认文件身份后，另行按本机卫生授权清理；实测不删除。 |
| `%APPDATA%\app.gogoke.desktop.candidate` | 卸载按产品语义保留候选用户数据。 | 先核对用途和逐字节清单，再由 Owner 决定是否放弃；实测不自动删除。 |
| `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-install-a3b6` 内的未知文件 | 卸载只删经验证的产品自有文件；未知文件可能留在安装根。 | 逐项记录路径、哈希、来源与归属，明确授权后再清理；身份或收尾不明时保全现场。 |

此单不执行 R2-06b 的完整/资源更新、回滚或崩溃恢复，不签正式清单，不合并 PR #34，不公开发布。云端 Windows Server PASS 与本单准备状态都不能替代 Owner Win11 强制模式下的实际观察。
