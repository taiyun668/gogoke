# G3：已安装正式版 0.1.2 的卸载登记视图

**结论（2026-09-27）：**正式版并非“两个视图都没有卸载登记”。Codex 派生进程的 HKCU 视图有 `Uninstall\gogoke`，普通交互进程的同名键缺席；正式安装文件和两条快捷方式在普通视图可见。旧安装器没有登记的假设与原始快照不符。登记分离符合打包应用的 HKCU 写入虚拟化，但当年安装命令及其进程身份未留原始回执，所以“当年由哪个具体进程安装”仍是推断。

## 直接证据

- 打包视图原始 `formal-before.json` 的字段叫 `registry`，含 `DisplayVersion=0.1.2`、安装根 `%LOCALAPPDATA%\Programs\gogoke`、`UninstallString="...\uninstall.exe"`。外部报告查询 `formalRegistry` 因字段名不符而误判为空。当前 Codex 派生进程重读该 HKCU 键也存在。
- 普通视图 `ordinary-before.json`、`ordinary-after-uninstall.json` 的 `formalRegistry` 均为 null；两次的正式安装树、正式数据、快捷方式逐项相同，原快捷方式可启动原正式 shell。用户 SID 相同，文件和注册表视图不同。原始回执保存在 Owner 机器的 `%LOCALAPPDATA%\Packages\<CodexPackageFamily>\LocalCache\Local\gogoke-agent-tmp\r206a-win11-b025`，对应 MC-140 至 MC-145。
- 同一 Codex 派生 PowerShell 中 `GetCurrentPackageFullName` 返回 15700，但它能读到包视图候选与正式登记；自动删除的 LocalAppData 文件句柄实际路径落在包的 `LocalCache`。这说明仅查当前子进程包身份不能证明未被重定向。
- 仓库的构建治理记载 0.1.2 由 Codex 从云端构建装到 `%LOCALAPPDATA%\Programs\gogoke`；当年启动方式没有进程级回执。既有正式键的 `uninstall.exe` 指向旧 Tauri/NSIS 卸载入口，与本轮 R2-06a 的 `gogoke.exe --uninstall` 不同。

## R2-05 之前的路径决定边界

R2-05 的任何正式安装、卸载、登记核验必须在**同一普通交互式用户视图**完成。本机已验证的启动方式为当前用户计划任务、`Interactive` 登录、`Limited` 运行级别，并应记录实际 SID、会话、组件哈希、Code Integrity 事件及前后正式快照。旧 0.1.2 在这个视图没有卸载登记，不能把 Codex 视图中的旧键当成它的卸载入口，也不能在本阶段跨视图调用旧 `uninstall.exe`。现有正式安装、正式数据、正式快捷方式和两种视图的正式登记均未修改。如何把旧正式安装迁入普通视图或替换为新的正式包，须在进入 R2-05 前由 Owner 选择并单独做精确身份、备份与恢复方案；R2-06a 的候选实测不执行该迁移。
