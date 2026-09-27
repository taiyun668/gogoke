# R2-06a / b025ba44 Win11 隔离候选实测准备

**状态：READY_FOR_WIN11_TEST；未运行 Owner Win11 实测，候选未接受。** 本单只对应源码 `b025ba44659da52a2747c7c0a281c5bbd7d4b732` 的云端冻结字节。最终全轴风险审计见 `artifacts/s1-r4/reviews/R2-06a-b025-final-risk-audit.md`；若源码、受信产物或所列哈希变化，本单失效并重新绑定。旧 `R2-06a-owner-win11-isolated-prep-a3b6.md` 已标 `SUPERSEDED`。

## 精确对象与已完成证据

| 证据 | 精确身份与结果 |
| --- | --- |
| 桌面构建与复现 | run `36302148604` attempt 1，五个 job 成功；冻结 artifact `10925629008`，六类字节比较 `PASS`，复现 JSON SHA-256 `904063cd027a6d4d3b24ee466d8ace8d5356ed50fb5384575fe664d52607e947`。另造的 CI 专用负测安装器 artifact `10926557839`。 |
| 公开源码卫生 | run `36302148207` 成功，扫描 24,550 个已提交文件，0 leaks。 |
| 候选资源签名 | 受信 `main@d0e739be45c95ca84d092e9cdb2dc46462cbaf9e` 的 run `36304375541` attempt 1 成功，artifact `10926871618` 原始 ZIP SHA-256 `79ed1d86857f04ae44a27cc5842164e857d2380a6cc7dabcaf2809995dd8f77f`。清单绑定候选用途、源码、构建 run、冻结 artifact、索引及资源包。本机离线 P-256 验签通过，内存单字节变异拒绝。 |
| 托管 Windows 装机 | run `36304535209` attempt 1 成功。正向回执 artifact `10926474589`，JSON SHA-256 `a5dbe25aaf15a870e1a4023001ea456b2c49f15c7bc913b52c507d39dc5bd3da`：实际安装产品、受控 probe、native Controller 准入、资源 readiness、poison 拒绝、卸载与数据保留 `PASS`。负测 artifact `10926549186`，JSON SHA-256 `770ba7517a3381151f4f424d93e7f5384e7db304e84f73f9f25ac1e9597c0746`：leaf insertion、同域双父目录安装、卸载最终核对与登记删除窗口的安装交错三轴均 `PASS`。平台明确为云端 Windows，不能代替本机 Win11/SAC。 |

生产候选安装包 `gogoke-0.1.3-windows-x64-unsigned-setup.exe` SHA-256 为 `2419d848017694e6819eb4e8dcd1004116e0a83e4ad4d2cde003b9d8448eb70d`；安装后预期 `gogoke.exe` 为 `325d7bccae248147a5eeb7d812af1b1c4dba910ac686379d04f5b8f3d878c929`、`gogoke-native-host.exe` 为 `842221cb902906e19b439123a638c50f80432edebed0788b75182e7a42c9d267`、内置 `node.exe` 为 `e3be0545990c90995d7bf3a7af5d64af1f2e0fc1bbd9b79c27f7abc1e9676e50`、`resource-index.json` 为 `cd8ee1462ed49f6647b1a8010ee9cfc7569190bdb83e52f8c5e9903fc0a67451`、`gogoke-resources.windows.zip` 为 `00e5c932959e2adc5e13b8437f2819ca53ef0d8f9329991c0f760c4bb8b87ffa`。CI 专用屏障安装器 SHA-256 为 `c36c096e14ff264214d3f75e74729b82447eda49cda76b85552ee0fb84fd7fdf`，**不安装到 Owner Win11**。

六类复现比较不包括 NSIS `setup.exe`：双车道 setup 字节不同。受信签名 artifact 使用冻结车道的 setup，已与该冻结原件逐字节核对；本机只运行这份精确 setup。

## 控制席位执行；Owner 唯一可选触点

1. 在安装前重新读 `gogoke.exe` 与 `gogoke-native-host.exe` 进程。两者均未运行，机器事实视为没有未保存的 Gogoke 工作；若运行，先读主窗口标题与可观察状态，只有仍无法判断未保存内容才请 Owner 选择，并提供建议。重读 Windows 11、Smart App Control 强制状态、Code Integrity 日志起点、正式版安装根与 HKCU 登记、正式数据和正式快捷方式的逐字节快照；候选登记、候选数据根及新安装根必须缺席。任何不符先停下诊断。此步全由控制席位完成。
2. 从已核原始 artifact 中只取上表正式候选 setup 与同目录 sidecar；再核 setup SHA-256、清单用途／来源、索引／资源包及签名。确认隔离目标位于 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-install-b025` 且不存在，路径祖先均为普通目录。使用当前账户单次静默 NSIS 安装；记录安装器进程、退出码及相关 SAC/Code Integrity 事件。不执行便携 exe，也不混入正式发行清单。此步全由控制席位完成。
3. 机械核实候选 HKCU 登记在 `...\Uninstall\gogoke-candidate`，`InstallDomain=CI_CANDIDATE_RESOURCE`，安装实例 ID、目标根和正式版互相独立。核已安装 shell、native host、Node、资源代的实际 SHA-256；从该安装根启动实际 `gogoke.exe`，观察它实际拉起的服务和 native host，执行固定的 `gogoke_r2_goal_probe` 产品入口并回读受控结果及测试账本身份；关闭后从同一根重启，验证签名资源 readiness 与结果回读。只记 `TEST_ONLY`，不作 adoption。此步全由控制席位完成。
4. 查询从起点之后的 `Microsoft-Windows-CodeIntegrity/Operational` 3033/3077；如实际出现 Win32 4551 也记录。每个事件绑定发生时间、确切组件、SHA-256 和本机路径的 `%LOCALAPPDATA%` 占位表示。任何拦截、签名／哈希／登记／实例偏差、正式版变化或子进程来源不明，停止该尝试、保全现场，不重试变通，不改 SAC。此步全由控制席位完成。
5. 成功后只调用该候选安装根的 `gogoke.exe --uninstall --quiet`，读取最终 finalizer 回执，核候选登记和自有文件删除、候选数据逐字节保留；重取正式安装、登记、数据及快捷方式快照并与安装前比对。**正式版快照比对完成之后**，从原正式快捷方式再启动一次正式版，观察其正常启动；此后正式数据可能变化，不再用启动后的数据替代先前比对。卸载身份或收尾不明时保留现场，不强删。此步全由控制席位完成。

全部机器检查结束后，Owner **可选**看一眼 Home、受控结果与错误是否符合产品目标；只有这一主观产品判断需要 Owner。控制席位提交结论、哈希、事件和证据路径供 Owner 裁决；候选测试本身不构成最终验收。

## 预期残留与清理条件

| 对象 | 预期状态与条件 |
| --- | --- |
| `%LOCALAPPDATA%\gogoke-registration-CI_CANDIDATE_RESOURCE.lock` | 安装／卸载后可能留下普通锁文件。记录身份及哈希；本次不删除。只有确认无同域进程、句柄已释放且获得单独清理授权才清理。 |
| `%APPDATA%\app.gogoke.desktop.candidate` | 产品卸载会保留候选用户数据。记录文件清单及 SHA-256；Owner 明确决定放弃该数据后才能清理。 |
| `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-install-b025` | 卸载只删核实自有的文件，未知文件或空目录可能残留。逐项核来源和归属；另有明确清理授权前保留。 |
| MC-122 的 `%TEMP%\gogoke-r206a-evidence-a3b6-36293571078` 两文件副本 | 原封保留，等 Owner 明确指示；不以其他办法清理。 |

正式安装 `%LOCALAPPDATA%\Programs\gogoke`、正式数据 `%APPDATA%\app.gogoke.desktop` 和正式快捷方式必须保全。不合并 PR #34、不开始 R2-06b、不发布、不做代码签名、不改任何系统安全设置。
