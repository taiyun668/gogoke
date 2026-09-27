# Claude 异构复核：R2-06a 精确源码 b025ba44

- 复核方：外部 Claude（只读），应 MC-145 `external_heterogeneous_review_to_owner` 的请求，由 Owner 转交。
- 对象：`taiyun668/gogoke@b025ba44659da52a2747c7c0a281c5bbd7d4b732`，重点是 `a3b6f85e..b025ba44`：F1/F2 的修复、云端交错测试、打包视图卸载失败、普通视图 Win11 实测回执、正式版保护。施工分支 HEAD `cf0504a4`。
- 独立核对方式：
  - 直接读 b025 的 `gogoke.nsi`、`update/gogoke-uninstall-finalizer.ps1`；
  - 直接读 Owner 机器上的实测证据目录（Codex 包的 LocalCache 下 `gogoke-agent-tmp/r206a-win11-b025/`），自己比对快照。
- **结论：F1、F2 已正确修复；普通视图的实测结论有原始数据支持。新发现一条 MEDIUM：安装和卸载对"被重定向的目录"判断不一致，在打包环境里会产生卸不掉的实例。对本项目来说这很实际，因为 agent 就是从 Codex（MSIX 打包应用）里操作 Owner 机器的。**本复核不是批准。

## 发现

### G1 [MEDIUM] 安装和卸载的路径信任判断不一致：打包环境里装得上、卸不掉

- **机制**：
  - Codex 桌面版是 MSIX 打包应用。从它里面启动的进程，对 `%LOCALAPPDATA%` 和 HKCU 的写入会被重定向到包的私有空间（`Packages\OpenAI.Codex_…\LocalCache`，以及虚拟化的注册表）。
  - 安装程序的 `GogokePinDirectory` 在这种环境下接受了 `$LOCALAPPDATA`，安装顺利完成，登记写进了包内的虚拟注册表。
  - b025 的 finalizer 新增的 `Acquire-RegistrationLock`，会先对 `LocalApplicationData` 执行 `Pin-Ancestors`。MC-145 的无损复现，定位到它在被重定向的祖先目录上报 `OPENED_OBJECT_UNSAFE`，于是卸载拒绝继续。
  - 结果就是一个装得上、卸不掉的实例，也就是 MC-145 保留的那个打包视图候选。
- **为什么是缺陷**：物理路径检查本身是对的，不能放松（MC-145 也这么写）。问题在于两边口径不一致：安装在一个卸载必然拒绝的环境里放行了。应该在任何改动发生之前，就明确拒绝这种环境。
- **对本项目的实际影响**：
  - agent 从 Codex 里直接运行安装程序，就会落进这种状态；
  - Owner 的正式版登记也可能跟这种环境有关，见 G3。
- **建议**：
  1. 安装程序和卸载预检都在最开始检测是否运行在打包环境里：`GetCurrentPackageFullName` 返回的不是 `APPMODEL_ERROR_NO_PACKAGE`（15700），就是在包里。检测到就在任何改动之前停止，并给出明确的提示："不支持从打包应用内运行，请直接运行安装程序或卸载程序。"
  2. 物理路径检查保持不变。
  3. 云端补一个负向测试。如果云端很难造出 MSIX 环境，就把"在打包环境里拒绝"这一分支单独做成可测的函数，用参数注入来测，并如实标出没有覆盖的部分。
  4. 治理和准备单写明：凡是在 Owner 机器上的安装和卸载，一律在普通视图里执行，也就是本次使用的"计划任务、当前用户、交互式登录、受限运行级别"那种方式。R2-05 的 Owner 验收同样适用。

### G2 [LOW] 卸载一侧拿锁失败时，也不区分原因

- **位置**：`Acquire-RegistrationLock` 在 `CreateFileW` 失败时，一律报 `GOGOKE_UNINSTALL_REGISTRATION_LOCK_UNAVAILABLE`。
- **建议**：带上 `Marshal.GetLastWin32Error()`（P/Invoke 声明需要 `SetLastError=true`）；32 表示"同一登记域有安装或卸载正在进行"，其他错误如实报错误码。这与安装一侧 F2 的修法一致。

### G3 [NEEDS_REVIEW] 正式版 0.1.2 在两个视图里都没有卸载登记

- **独立核对**：`ordinary-before.json` 和 `ordinary-after-uninstall.json` 的 `formalRegistry` 都是 null；打包视图的 `formal-before.json` 里 `formalRegistry` 也不存在。
- **含义**：
  - 正式版不在 Windows"应用"列表里，Owner 没法从系统设置里卸载它；
  - 本次"正式版登记不受影响"只能证明"本来就没有登记，现在仍然没有"（MC-145 已如实写明）；
  - 原因待查：旧版安装程序本来就不写登记，还是当初从打包环境里安装、登记落进了某个虚拟注册表。
- **建议**：R2-05 之前查明原因，并定下正式版的安装和卸载路径。不要在查明之前去动正式版。

### G4 [HOUSEKEEPING] Owner 机器上的遗留物需要一份清理计划

目前保留在 Owner 机器上的：
- 打包视图里的候选实例：文件、包内虚拟注册表中的登记、两个数据文件；
- 普通视图里：登记域锁文件、候选数据的两个文件、安装根下 1,087 个空目录；
- MC-122 的两文件证据副本；
- Codex 包 LocalCache 下的实测证据目录（60 个文件）。

**建议**：由控制席位出一份逐项清单，写明位置、大小、是否仍被证据引用、在哪个视图里清理、怎么清理。凡是要永久删除的，由 Owner 明确同意后执行；打包视图里的那份候选，要等 G1 的修复可用以后，按保持身份一致的方式处理。

## 已核对，没有发现问题

- **F1**：finalizer 在第一次 `Assert-Root`（它会调用 `Assert-Instance`）之前拿到登记域锁，一直持有到 `DeleteSubKey` 之后，在 `finally` 里释放。安装一侧从预检之前到登记回读都持有同一把锁，所以同一登记域的装与卸是互斥的。两边拿锁都不等待，拿不到就失败退出，不会死锁。
- **F2**：安装一侧用 `System::Call ... ?e` 取 `GetLastError`，每次都弹栈，栈是平衡的；只有 32 报"正在进行"，其他错误报出错误码。
- **阶段标记**：`GogokeCIStage` 只在定义了 `GOGOKE_NSIS_TEST_BARRIER` 时展开，不进生产安装包。
- **普通视图的正式版保护**：由我独立比对。`ordinary-before.json` 对 `ordinary-after-uninstall.json`：
  - `formal`、`formalData`、`formalRegistry` 展开后共 54,133 项，除时间戳字段外完全一致；
  - 两个快捷方式一致；
  - 智能应用控制状态都是 1，用户 SID 相同；
  - 候选登记在卸载前后都不存在（前者是安装之前的快照）。

## 没有做的

- 没有逐行审 `tools/ci/gogoke-r206a-installer-negative.ps1` 新增的约 186 行，也没有审 `test_gogoke_uninstall_finalizer_cloud.py`；
- 没有核对 `product-corrected.json` 的产品结果，也没有看截图；
- 没有下载云端产物重算哈希；
- 没有运行任何构建、测试或安装。
