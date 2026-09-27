# R2-06a / b025ba44 Win11 隔离候选实测准备

**状态：普通用户视图的 Owner Win11/SAC 隔离安装、产品探针、卸载和正式快捷方式复启已执行；候选未接受。** 本单只对应源码 `b025ba44659da52a2747c7c0a281c5bbd7d4b732` 的云端冻结字节。最终全轴风险审计见 `artifacts/s1-r4/reviews/R2-06a-b025-final-risk-audit.md`；若源码、受信产物或所列哈希变化，本单失效并重新绑定。旧 `R2-06a-owner-win11-isolated-prep-a3b6.md` 已标 `SUPERSEDED`。

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

1. 在安装前重新读 `gogoke.exe` 与 `gogoke-native-host.exe` 进程。两者均未运行，机器事实视为没有未保存的 Gogoke 工作；若运行，先读主窗口标题与可观察状态，只有仍无法判断未保存内容才请 Owner 选择，并提供建议。重读 Windows 11、Smart App Control 强制状态、Code Integrity 日志起点、正式版安装根与 HKCU 登记、正式数据和正式快捷方式的逐字节快照；候选登记、候选数据根及新安装根必须缺席。Codex 打包进程会虚拟化 LocalAppData 和 HKCU，因此用同一用户、同一交互会话的普通用户进程核最终安装视图；两个视图分别记账。任何不符先停下诊断。此步全由控制席位完成。
2. 从已核原始 artifact 中只取上表正式候选 setup 与同目录 sidecar；再核 setup SHA-256、清单用途／来源、索引／资源包及签名。确认普通用户视图的隔离目标 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-real-install-b025` 缺席，任务临时父目录为普通目录。使用同一用户的普通进程静默 NSIS 安装；记录安装器进程、退出码及相关 SAC/Code Integrity 事件。不执行便携 exe，也不混入正式发行清单。此步全由控制席位完成。
3. 机械核实候选 HKCU 登记在 `...\Uninstall\gogoke-candidate`，`InstallDomain=CI_CANDIDATE_RESOURCE`，安装实例 ID、目标根和正式版互相独立。核已安装 shell、native host、Node、资源代的实际 SHA-256；从该安装根启动实际 `gogoke.exe`，观察它实际拉起的服务和 native host，执行固定的 `gogoke_r2_goal_probe` 产品入口并回读受控结果及测试账本身份。重启与回读只按实际发生的进程记账，不把另一视图的重启冒充本视图证据。只记 `TEST_ONLY`，不作 adoption。此步全由控制席位完成。
4. 查询从起点之后的 `Microsoft-Windows-CodeIntegrity/Operational` 3033/3077；如实际出现 Win32 4551 也记录。每个事件绑定发生时间、确切组件、SHA-256 和本机路径的 `%LOCALAPPDATA%` 占位表示。任何拦截、签名／哈希／登记／实例偏差、正式版变化或子进程来源不明，停止该尝试、保全现场，不重试变通，不改 SAC。此步全由控制席位完成。
5. 成功后只调用该候选安装根的 `gogoke.exe --uninstall --quiet`，读取最终 finalizer 回执，核候选登记和自有文件删除、候选数据逐字节保留；重取正式安装、登记、数据及快捷方式快照并与安装前比对。**正式版快照比对完成之后**，从原正式快捷方式再启动一次正式版，观察其正常启动；此后正式数据可能变化，不再用启动后的数据替代先前比对。卸载身份或收尾不明时保留现场，不强删。此步全由控制席位完成。

全部机器检查结束后，Owner **可选**看一眼 Home、受控结果与错误是否符合产品目标；只有这一主观产品判断需要 Owner。控制席位提交结论、哈希、事件和证据路径供 Owner 裁决；候选测试本身不构成最终验收。

## 2026-09-27 实际结果与边界

| 轴 | 实际观察 |
| --- | --- |
| Codex 打包视图 | 首次安装和产品探针使用了该视图的虚拟化 LocalAppData/HKCU；相同候选字节的 Home、受控结果及同根再次启动有证据。卸载父进程两次报 `GOGOKE_UNINSTALL_LOCK_HANDOFF_UNCONFIRMED`，候选登记/根/数据仍在。安全诊断副本在 `Pin-Ancestors` 观察到逻辑路径与打开句柄的 Codex 包 `LocalCache` 物理路径不一致，足以复现同型拒绝；真实卸载子进程的内部阶段未采样。此视图的卸载轴 `FAIL`，不能算普通用户候选的卸载，也不能据此弱化物理路径断言。见 MC-138、MC-139。 |
| 普通用户视图安装 | 同一用户 SID、交互会话 1，SAC 状态 1。正式版安装根 12,989 文件、数据 2 文件和两条快捷方式先建独立基线；正式 HKCU 登记在该视图原本缺席。冻结 setup SHA-256 `2419d848017694e6819eb4e8dcd1004116e0a83e4ad4d2cde003b9d8448eb70d` 安装退出 0。安装后 shell、host、Node、索引和资源包 SHA-256 均等于上表；普通候选实例 ID 的 UTF-8 SHA-256 为 `9896485fdfec67c0deaa28592bdb0ac09b2bb6a9c75e3e0f2e7e90634337cc20`。见 MC-140 至 MC-142。 |
| 普通用户视图产品 | 已安装且 Authenticode `Valid` 的 OpenJS Node 驱动一次实际 Home 和固定 `gogoke_r2_goal_probe`：Controller 准入、native host 可达、签名资源 readiness、Git blob `a20115fdd5acf9e7e5025c3b3ca50696001badac` 回读，受控结果 `VALIDATED_TEST_RESULT_NOT_ADOPTED`，acceptance `TEST_FIXTURE_NOT_ADOPTED`。截图 SHA-256 `ca3f84c2fedd8712fc83da2c910eba9912c3e941cc74cb21dd311651c2d71b29`；CDP 截图的黑色背景不能单独证明实际桌面画面有缺陷。普通视图没有第二次产品启动；打包视图的再次启动单列。见 MC-143。 |
| 普通用户视图卸载与正式版 | `gogoke.exe --uninstall --quiet` 父进程退出 0，精确实例 finalizer 回执 `DELETED`；候选登记和全部文件消失，安装根只留 1,087 个空目录；候选数据 2 文件卸载前后树哈希均为 `498070a77bddc1d137cd0d5c10e30fe99cf05168709dcab0045aaf710ba4466d`。**先**比对正式根、数据、快捷方式与安装前逐字节一致，**后**从原开始菜单快捷方式启动正式版，窗口响应 5 秒并正常关闭。正式数据随后只有 `.window-state.json` 变化，不能拿启动后快照替代先前比对。见 MC-144 与下一检查点。 |
| SAC / Code Integrity | 普通安装、产品、卸载、正式启动全程 SAC 状态 1；基线 RecordId 7097 之后没有新事件。此前 7090–7097 是 Chrome Vulkan DLL 的 3033 与对应 3089，不归到 Gogoke。没有观察到 Win32 4551。 |

以上是候选平台证据，**不是**正式 Owner 清单签名、正式发行域实测、R2-05 adoption、Goal Acceptance 或公开发布。精确 `b025ba44` 的外部 Claude 异构复核仍由 Owner 转交；并行收件箱截至 `b867dcaf` 没有该结果。

本机原始回执在 Codex 打包视图的 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-b025`；普通用户进程可读取的同一证据目录物理位置为 `%LOCALAPPDATA%\Packages\<CodexPackageFamily>\LocalCache\Local\gogoke-agent-tmp\r206a-win11-b025`。普通用户实际安装根和 finalizer 回执则在该进程自己的 `%LOCALAPPDATA%\gogoke-agent-tmp`。这些路径视图不能混称同一个对象。

## 预期残留与清理条件

| 对象 | 预期状态与条件 |
| --- | --- |
| 普通用户视图 `%LOCALAPPDATA%\gogoke-registration-CI_CANDIDATE_RESOURCE.lock` 与 `%LOCALAPPDATA%\gogoke-agent-tmp\gogoke-install-lifecycle.lock` | 都是普通文件，分别 0 与 647 字节，均已释放且可独占打开；见本机 `ordinary-residue.json`。本次保留，清理前仍需确认无同域进程、对象身份和单独清理授权。 |
| 普通用户视图 `%APPDATA%\app.gogoke.desktop.candidate` | 卸载后保留 2 文件、精确树哈希 `498070a77bddc1d137cd0d5c10e30fe99cf05168709dcab0045aaf710ba4466d`；Owner 明确决定放弃该数据后才能清理。 |
| 普通用户视图 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-real-install-b025` | 自有文件和候选登记已清，剩 1,087 个空目录；逐项核来源和归属，另有明确清理授权前保留。 |
| Codex 打包视图的候选根、`gogoke-candidate` 登记、候选数据与登记域锁 | 卸载握手失败后保持原样。其根与普通用户视图不是同一个对象；不得跨视图重试卸载或手动删登记。后续清理须先有经过审计的虚拟化路径处理或单独的精确实例恢复方案，再核身份与正式版不受影响。 |
| MC-122 的 `%TEMP%\gogoke-r206a-evidence-a3b6-36293571078` 两文件副本 | 原封保留，等 Owner 明确指示；不以其他办法清理。 |

正式安装 `%LOCALAPPDATA%\Programs\gogoke`、正式数据 `%APPDATA%\app.gogoke.desktop` 和正式快捷方式必须保全。不合并 PR #34、不开始 R2-06b、不发布、不做代码签名、不改任何系统安全设置。
