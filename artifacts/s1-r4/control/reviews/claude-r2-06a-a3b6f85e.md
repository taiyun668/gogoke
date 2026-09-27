# Claude 异构复核：R2-06a 精确源码 a3b6f85e

- 复核方：外部 Claude（只读），应 MC-123 `external_claude_handoff` 的请求，由 Owner 转交。
- 对象：`taiyun668/gogoke@a3b6f85e38ca0971fdd05096734c2e99aeee7ec4`，重点是 `a8dc77f4..a3b6f85e` 的改动。施工分支 HEAD `e6870984` 相对 a3b6f85e 只增加了检查点和文档。
- 同时读了：`artifacts/s1-r4/reviews/R2-06a-a3b6-final-risk-audit.md`、`artifacts/s1-r4/R2-06a-owner-win11-isolated-prep-a3b6.md`，以及 a3b6f85e 的 `gogoke_uninstall.rs` 和 `update/gogoke-uninstall-finalizer.ps1`。
- **结论：没有阻挡 Owner Win11 隔离实测的问题，也没有架构阻挡。有一条 MEDIUM，接受 R2-06a 之前应当修复，或由 Owner 明确延期。**本复核不是批准。

## 发现

### F1 [MEDIUM] 新增的"按登记域"锁只挡住了安装和安装之间的竞争，没有挡住卸载

- **位置**：
  - `apps/desktop/src-tauri/installer/gogoke.nsi`：`AcquireGogokeLifecycleLock` 新增的 `gogoke-registration-<domain>.lock`；
  - `apps/desktop/src-tauri/update/gogoke-uninstall-finalizer.ps1`：`Assert-Instance`（约第 162–176 行）和 `DeleteSubKey`（约第 431–432 行）；
  - `apps/desktop/src-tauri/src/gogoke_uninstall.rs`：卸载前的预检只打开按父目录的 `gogoke-install-lifecycle.lock`（约第 808–815 行）。
- **机制**：
  - 新锁的注释写明它针对的情形是"不同的目标父目录仍可能写同一个按用户、按登记域的卸载登记"。
  - 卸载的最后一步是：先在 `Assert-Instance` 里核对登记的 `InstallLocation`、`InstallInstanceId`、`InstallDomain`、`UninstallString`，然后再 `DeleteSubKey`。
  - 这整个过程只持有自己父目录的生命周期锁，不持有登记域锁。
- **失败场景**：
  - 同一个用户、同一个登记域，A 装在父目录 P1、正在卸载；B 装到另一个父目录 P2。
  - 如果 B 恰好在 A 的核对与删除之间写入了自己的登记，A 会删掉 B 刚写的登记项。
  - 后果：B 的文件都在，但从"应用"列表里消失；以后再从同一个登记域安装或卸载，都会遇到登记和实际不一致。
  - 时间窗很小，需要同时做装和卸才会碰上。
- **为什么不挡住 Win11 实测**：候选（`gogoke-candidate` / `CI_CANDIDATE_RESOURCE`）和正式版（`gogoke` / `OWNER_RELEASE`）用的是不同的登记项和不同的锁文件；实测单是按顺序单人操作的。
- **建议**：
  - 卸载预检（Rust）或 finalizer 在 `Assert-Instance` 之前，以同样的方式（不共享、不跟随重解析点）打开同一个 `%LOCALAPPDATA%\gogoke-registration-<domain>.lock`，一直持有到 `DeleteSubKey` 之后；
  - 在云端的负向安装测试里加一种情形：A 卸载时，同一登记域、不同父目录的 B 在屏障处等待，断言 B 的登记项不被删除。
- **现有审计**：`R2-06a-a3b6-final-risk-audit.md` 的 R3 测试了同一登记域、不同父目录的两次**安装**，R5 测试了单独的卸载，没有覆盖装与卸交错。所以这一条是新发现。

### F2 [LOW] 登记域锁打开失败时，任何原因都报成"另一个安装正在进行"

- **位置**：`gogoke.nsi` 新增的 `CreateFileW(... i 0, p 0, i 4, i 0x00200080 ...)` 与紧随其后的 `Abort "Another Gogoke install for this registration domain is active."`。
- **问题**：权限不足、路径问题、安全软件拦截等别的失败，也显示成"另一个安装正在进行"，会误导人去等待或重试。行为本身是出错就停，这一点是安全的。
- **建议**：用 `GetLastError` 区分：只有 `ERROR_SHARING_VIOLATION`（32）报"正在进行"，其他情况如实报错误码。

### F3 [LOW] Win11 实测单应把几处预期残留列出来

实测单第 5 步要求卸载后，正式版的字节、登记、数据、快捷方式都与测试前的快照一致。但下面这些会按设计留在 Owner 的机器上，应在单子里列为**预期残留**，并写明清理条件，免得被误判为"有变化"，也避免它们被遗忘：
- `%LOCALAPPDATA%\gogoke-registration-CI_CANDIDATE_RESOURCE.lock`：新锁文件，安装和卸载都不删除它；
- `%APPDATA%\app.gogoke.desktop.candidate`：卸载时按产品语义保留用户数据；
- `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-install-a3b6` 下卸载后剩下的未知文件（如果有）。

### F4 [NOTE] 实测单可以补一次正式版能否启动的检查

- 正式版的字节和数据快照比对完成**之后**，可以再从正式快捷方式启动一次正式版，确认它还能正常起来。先比对再启动，是因为启动会改写正式版的数据文件。
- 字节不变不等于运行不受影响，比如单实例、AppUserModelId 这类按标识共享的东西。审计 R5 把正式域的 AppUserModelId 列为没跑，这一步正好给出一个低成本的实际观察。

## 已核对，没有发现问题

- **`product_entry.rs`**：Node 服务进程通过 `PROC_THREAD_ATTRIBUTE_JOB_LIST`，在 `CreateProcess` 时直接进入已配置好的 kill-on-close Job，消除了"已创建、还没放进 Job"时持有者死亡、留下孤儿的窗口。显式的 `AssignProcessToJobObject` 核对保留。`#[cfg(test)]` 的暂停钩子不进生产字节。测试取得了精确的 Node 句柄以后才终止持有者，并按 PID 复用的风险做了防护。方法正确。
- **安装程序中新锁本身**：以不共享、`OPEN_ALWAYS`、`FILE_FLAG_OPEN_REPARSE_POINT` 打开，再检查不是目录也不是重解析点；持有到 `ReleaseGogokeLifecycleLock`。更新协调器路径与普通安装路径都会先经过它（它位于两条路径分叉之前）。获取失败会立即中止，不等待，所以没有死锁的风险。
- **云端工作流隔离**：
  - 带屏障的安装包在冻结产物上传之后才单独构建，屏障环境变量只在这一步里生效；
  - 构建时断言产品主程序与冻结版逐字节相同、安装包的哈希与冻结版不同，然后用单独的产物名上传；
  - 负向测试脚本同时绑定生产安装包、源码、运行编号和尝试次数。
- **签名边界**：GitHub 环境 `candidate-resource-signing` 的部署分支策略只允许 `main`（按分支类型的自定义策略，已通过 API 读取）；签名运行 `36293506694` 由 `main@d0e739be` 的 `workflow_run` 触发。
- **实测单的隔离设计**：
  - 候选装在任务临时根下，用独立的登记项、数据根和实例编号；
  - 测试前后都对正式版做快照；
  - 从记录的起始事件号读取代码完整性日志；
  - 任何不符都停止并保全现场，不重试变通，不改动智能应用控制。

## 没有做的

- 没有逐行审 `tools/ci/gogoke-r206a-installer-negative.ps1`（347 行），只核对了它在工作流里的绑定参数。
- 没有下载机器回执，也没有重算产物的哈希；这部分采信审计回执里记录的来源链。
- 没有运行任何构建、测试或安装。
