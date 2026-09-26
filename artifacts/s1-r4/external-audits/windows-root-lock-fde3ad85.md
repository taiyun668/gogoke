# WCS-27 — Windows root identity / lock boundary 静态审计结果

## 固定对象与结论

| 字段 | 值 |
| --- | --- |
| task_id | WCS-27 |
| role | independent fixed-commit Windows root-identity and lock-boundary auditor |
| repository | taiyun668/gogoke |
| task card | `docs/trials/web-chatgpt-subagent/WCS-27.json` @ `440f81c551c7048c179109fb1d8630b2fffb7f7d` |
| task-card blob | `ca41595eeb8541fabffa577ded2aa6e6ac58ce21` |
| base_sha | `fde3ad85cf6518e521c840ac1ca2e383e778d32e` |
| review_sha | **`fde3ad85cf6518e521c840ac1ca2e383e778d32e`** |
| assigned_branch | `gpt/web-chatgpt-wcs27-root-boundary-audit` |
| sole result path | `docs/trials/web-chatgpt-subagent/WCS-27-RESULT.md` |
| result | **STATIC_AUDIT_COMPLETE_WITH_FINDINGS — 2 findings** |
| execution evidence | **NOT_RUN**：本次未执行本地 CLI、原生编译、测试、CI dispatch 或 Windows 11 实机操作 |

**结论：发现一个本进程 poison 准入检查的并发缺口，以及一个带明确威胁模型前提的全局 mutex 名称预占拒绝服务问题。** F1 的本地 Rust API 控制流证据较强；其生产可达性取决于未审阅的上层失败清理。F2 是可用性问题，不是取得写权限或制造双 writer 的证明。未将尚未验证的路径重定向假设升级为已确认漏洞，也不据此宣称整个边界安全、项目验收通过或产品可发布。

所有下文源码行号均绑定上述 review SHA，不引用移动分支上的源码。任务卡全文已读取；源码读取仅涉及任务卡列出的路径。

## 证据对象

| 文件 | 审阅 blob SHA |
| --- | --- |
| `AGENTS.md` | `9f10c92267383e4a1d961e0ee404af3aad381fd9` |
| `docs/governance/gpt-construction-window-rules.md` | `7ef89e7abab1ad4a6eaff603f5fa906561fd4e79` |
| `apps/desktop/native-host/src/root/mod.rs` | `6effb3ac57b0aee21f9d654cea584827fbd52701` |
| `apps/desktop/native-host/src/root/lock_probe.rs` | `5992ffd1180593cfddd65be10515e9840e1e2f7b` |
| `apps/desktop/native-host/src/store/authority/root_identity_tests.rs` | `af2c72c73486fbf64de5a22929f5e2dd999aa82f` |
| `.github/workflows/gogoke-native-host.yml` | `1819772eb324a12e9ca937268da2afd36f61a869` |

代码及测试文件按小范围分段读取。治理采用第 2、3、4、5、5a、6、8 节；读取记录有一处范围偏差：第二段治理读取的 121–155 行意外包含 8a/8b 内容，未据此执行长期窗口流程、读取并行收件箱或安排复核；没有读取第 9 节。未修改被审查代码。

## F1 — P1：取锁前的 poison 快速检查可以被并发失败清理穿过

**分类：本地 API 的 fail-closed / 生命周期缺陷。** 静态控制流可构造违反“本进程永久拒绝该身份重用”的交错；不等于已证明 IPC 可触发、数据损坏或实际双 writer。

### 精确证据

- [`apps/desktop/native-host/src/root/mod.rs:596-601`](https://github.com/taiyun668/gogoke/blob/fde3ad85cf6518e521c840ac1ca2e383e778d32e/apps/desktop/native-host/src/root/mod.rs#L596-L601)：`inspect_root` 后仅在这里调用 `root_is_poisoned`；此时尚未获取 named mutex。
- [`apps/desktop/native-host/src/root/mod.rs:606-667`](https://github.com/taiyun668/gogoke/blob/fde3ad85cf6518e521c840ac1ca2e383e778d32e/apps/desktop/native-host/src/root/mod.rs#L606-L667)：随后才启动锁线程、接收成功、执行 `open_root`，比较前后物理身份，最后返回 `Ok(Self { ... })`。这段成功路径没有再次检查 poison。
- [`apps/desktop/native-host/src/root/mod.rs:674-681`](https://github.com/taiyun668/gogoke/blob/fde3ad85cf6518e521c840ac1ca2e383e778d32e/apps/desktop/native-host/src/root/mod.rs#L674-L681)：`poison_identity` 的合同明确是原生关闭 UNKNOWN/失败后，整个进程生命周期永久拒绝重用。
- [`apps/desktop/native-host/src/root/mod.rs:19-36`](https://github.com/taiyun668/gogoke/blob/fde3ad85cf6518e521c840ac1ca2e383e778d32e/apps/desktop/native-host/src/root/mod.rs#L19-L36)：poison 表是本进程共享集合；一次查询的 Rust mutex guard 不跨越 OS mutex 获取。
- [`apps/desktop/native-host/src/root/mod.rs:758-780`](https://github.com/taiyun668/gogoke/blob/fde3ad85cf6518e521c840ac1ca2e383e778d32e/apps/desktop/native-host/src/root/mod.rs#L758-L780)：`Drop` 释放并 join 锁线程；预检查句柄使用 `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`，因此另一个线程可以在现有 root 持有期间进行身份预检查。

### 必要前提与反例交错

同一进程内存在两个 independently acquiring 的 custodian 线程。线程 A 持有物理 root R；线程 B 对 R 调用 `RootLock::acquire`。A 的失败处理会将 R poison，并最终释放其 RootLock。这里不需要把现有 RootLock 跨线程移动，因此 `!Send/!Sync` 不排除此交错。

1. B 完成身份预检查，读到 `root_is_poisoned(R) == false`，在启动/运行取锁 helper 前被暂停。
2. A 在原生关闭失败路径中调用 `poison_identity(R)`，随后完整 drop 旧 RootLock，包括其目录句柄。
3. B 恢复。此时 OS mutex 和路径绑定都可取得，且物理身份仍是 R。
4. B 通过 649 行的身份比较，在 659–667 行返回成功；poison 集合仍包含 R。

无需假定 `WaitForSingleObject(..., 0)` 会阻塞：暂停发生在等待之前，旧锁释放后才开始该次等待即可。`RootIdentityChanged` 也无法拦截，因为此处没有更换物理目录。

结果是一个已经进入 poison 集合的 root 获得新的 RootLock；若上层将其视为新的 native custody 准入，这会绕过预期隔离。**上层是否始终保留/泄漏旧 root 锁直到进程退出、是否串行化所有 acquire/poison，以及哪些 IPC 操作可达这一路径，不在允许读取的实现范围内，未验证。** 若旧锁永不释放，该具体交错不能完成，但这是额外的调用方保证，不是当前 RootLock API 自身保证。

### 最小修复方向与证伪测试

保留前置检查作快速拒绝，在已经拥有 OS mutex、完成物理绑定后、发布 `RootLock` 前，再检查同一身份是否 poisoned。发现 poison 时走已有 release + join 的错误清理路径。应同时确认生产 poison 的发布发生在旧 root 独占权释放之前；不要通过清空 poison 表修复。

在未来授权的 Windows 云端测试中，用 `cfg(test)` barrier 精确暂停 B 于前置 poison 检查之后，A 在自己的线程 poison 并完整 drop，再放行 B。断言 B 返回 `RootLockError::Poisoned` 而不是 `Ok`；对照组验证未 poisoned 的其他 root 可获取、已 poisoned 的普通后续获取被拒绝。当前成功路径预计不能满足第一个断言。必须记录实际执行数量和结果，不用 sleep 猜时序。

该测试设计 **NOT_RUN**。允许读取的 root 测试和三个 authority root-identity 测试没有这类 acquire/poison 交错；测试中名为 `poisoned_root` 的 NUL 输入也不是 registry poison 测试。

## F2 — P2：可预测的 Global mutex 名称可被无 root 写权限的本地主体预占

**分类：有条件的本地可用性 / 跨主体命名空间问题，不是权限提升。** 前提是威胁模型包含能创建同名全局同步对象、知道目标物理身份、但未获目标 root 写权限的本地进程。若部署明确只防合作进程竞争，应将其记录为合同限制，而不是宣称已有抗敌对本地进程能力。

### 精确证据

- [`apps/desktop/native-host/src/root/mod.rs:48-54`](https://github.com/taiyun668/gogoke/blob/fde3ad85cf6518e521c840ac1ca2e383e778d32e/apps/desktop/native-host/src/root/mod.rs#L48-L54)：名称完全由公开格式、volume serial 和 file ID 派生，为 `Global\Gogoke.Root.v1.<volume>.<file-id>`，不含受保护的创建身份边界。
- [`apps/desktop/native-host/src/root/mod.rs:1271-1291`](https://github.com/taiyun668/gogoke/blob/fde3ad85cf6518e521c840ac1ca2e383e778d32e/apps/desktop/native-host/src/root/mod.rs#L1271-L1291)：使用 `CreateMutexW(NULL, TRUE, name)`；创建失败直接返回 `CreateLock`，已有且占用的 mutex 返回 `AlreadyLocked`。没有检验创建主体或受控 private namespace。
- `apps/desktop/native-host/src/root/mod.rs:609-633` 与 `641-648`：named-mutex 错误会在真正路径绑定之前终止 root 获取。目标目录没有实际 writer，也仍可被这一外部对象阻止进入。

Microsoft 的 [CreateMutexW 文档](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw) 明确说明：NULL security attributes 使用创建者 token 的默认安全描述符；遇到既有 mutex 请求 `MUTEX_ALL_ACCESS`；同名 event/semaphore 等不同类型对象会导致失败；也明确警告预先创建命名 mutex 可阻止应用启动。[Kernel object namespaces 文档](https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces) 将额外的全局创建特权检查限定在 file mapping / symbolic-link 对象，不能将该特权当作这里 mutex 名称的保护。

### 必要前提、影响与证伪测试

另一本地主体事先获知或可查询 R 的物理身份，并能在相关命名空间创建对象。它在合法 owner 首次 acquire 之前保持同名 event；合法 `CreateMutexW` 因类型冲突失败。另一变体是预创建同名 mutex，以不允许合法 owner 打开的 DACL 保持对象，或允许打开但持续占用它。整个过程不需要写 R 的文件内容。

这只是 **root 准入拒绝服务**：现有代码仍 fail closed；没有证据表明占名者因此获得 R 的 RootLock、能调用产品 IPC 或能写数据库。实际进程 token、跨用户目录 ACL、AppContainer/沙箱约束与 IPC 身份认证均未审阅。

未来云端反例应先证明“无外部对象时合法 root acquire 成功”；再由隔离 helper 保持精确同名 event，观察合法 acquire 在目录无 writer 时失败；释放 event 后应恢复成功。跨主体版本另须证明 helper 没有目标 root 写权限，并记录对象创建是否真正成功。另测既有 mutex 的 DACL 拒绝和持有态，区分 namespace collision、access denied 与真实 cooperating writer 的 busy。当前代码按上述 Win32 语义会暴露该拒绝服务面；**本次未执行这些 helper 或测试**。

修复应先明确授权主体范围，再选择由这些主体控制创建权的命名空间/协调方式，保持同一物理 root 在所需 session/principal 范围内仍共享唯一排他域。仅在 `CreateMutexW` 新建时附加 DACL 不能清除已预占对象；简单改为每进程随机名或每用户不同名又可能破坏原来的物理 root 排他合同。具体部署选择留给 Controller，不在本次修改权限内。

## 对既有测试证明力的核查

| 已读证据 | 执行成功时实际能够反驳的错误实现 | 不能据此推出的保证 |
| --- | --- | --- |
| `root/mod.rs:1713-1845`，包括 `lock_uses_physical_identity_and_creates_no_lock_file`、`held_root_path_cannot_be_renamed_deleted_or_replaced`、`held_junction_and_target_cannot_be_retargeted` | 直接路径与一个 junction 选取不同物理身份/锁；持有期间末级入口仍可普通 rename/delete；释放后入口仍不可用 | 任意祖先路径稳定、原位 reparse 数据修改均被拒绝、跨用户 namespace 防预占、poison 并发拒绝 |
| `root/mod.rs:1857-2084`，existing/create/reject database pin 测试 | 已 pin 主文件仍可普通替换；CREATE_NEW 截断既有文件；接受明显 outside/nested/reparse/directory 子项；把主文件 pin 错当 sidecar pin | 所有写句柄被禁止；sidecar 内容或恢复已验证；数据库打开与父路径检查之间不存在重定向窗口 |
| `store/authority/root_identity_tests.rs:22-41`，`transaction_root_is_the_pinned_directory_not_the_database_file` | transaction/profile 使用数据库文件 identity 而不是 pinned directory identity | 多线程 acquire/poison 或跨进程故障恢复正确 |
| `store/authority/root_identity_tests.rs:43-65`，`unrelated_root_lock_cannot_initialize_authority_or_leave_partial_schema` | 接受另一个真实 root lock 初始化 authority，或拒绝后留下 authority 表 | 上层每条生产调用链都传递不可伪造的同一 authority |
| `store/authority/root_identity_tests.rs:67-85`，`current_profile_rejects_a_stored_root_that_is_not_the_pinned_root` | 接受已被改成 `forged-root` 的持久 profile，或在拒绝时擅自改写它 | IPC 身份认证、部署目录权限和所有 SQL/native 接缝已证明 |
| `root/lock_probe.rs:27-80` | helper 可报告 ACQUIRED/DB_PINNED/BUSY，直接 root 竞争有专用退出码 23 | helper 源码存在就是一次多进程测试；退出码 0 本身证明 non-inheritable。查询失败会被转为输出中的 false；消费者仍须核查输出 |

上表路径前缀分别是 `apps/desktop/native-host/src/`；所有条目绑定 review SHA。三个 authority 测试直接触及真实目录 identity、真实不同 root 和数据库持久值，不是仅比较路径字符串；但它们并不覆盖 F1/F2。没有读取其父模块/构建清单来证明 cfg/feature 注册，因此不从测试定义存在反推实际发现或执行。

## 静态上成立的局部防线，以及没有建立的结论

`root/mod.rs:782-896` 先持有目录句柄，再取 `FileStandardInfo`、final path 和 `FileIdInfo`；`649-657` 比较预检查与绑定身份，能拒绝该间隔内变成不同物理对象的结果。`13-16` 的 volume serial + 128-bit file ID 用于物理标识，锁名不是用户传入路径的 hash。相对路径、NUL、明显网络/设备命名空间在 `1306-1364` 一带进行准入检查，canonical path 也重新验证。这里未发现“仅换一个普通别名就选出另一个 root mutex”的静态证据。

`root/mod.rs:471-483` 的线程绑定 marker，以及 `490-512` 一带的 borrowed-root lifetime marker，构成 safe Rust API 的线程/借用约束；这不证明 C/IPC 调用方同样遵守。锁的 Win32 owner 是 helper 线程，该线程取锁并执行 `ReleaseMutex`（`606-622`），并非在任意 drop 线程直接释放 thread-owned mutex。普通 `open_root` 失败和身份变化路径会通知 helper 并 join（`641-657`）；owned handles 由 `421-445` 一带 RAII 关闭。静态上未发现这些正常返回路径漏掉 join 的独立 finding。

`RootLock::Drop` 先通知释放/join，再由字段析构关闭目录句柄；因此不能把这几个动作描述成跨对象原子交接。短暂共享冲突也不等于第二 writer 已获准。`WAIT_ABANDONED` 在 `1281` 与正常获得锁同样处理；其含义仅是取得排他，不是证明旧数据库状态已经恢复。

### 未验证点 / 待证伪假设（不计入上述 2 findings）

**U1 — 缓存 canonical path 与后续 path-based open。** `apps/desktop/native-host/src/root/mod.rs:1004-1026` 检查 requested parent 的身份，却返回缓存 `canonical_path.join(file_name)`；`946-960` 一带再用路径打开文件，而不是相对 root handle 打开。`810-819` 的 ordinary-directory 句柄允许 FILE_SHARE_WRITE。未来应以 barrier 暂停在父身份检查与 child open 之间，分别尝试把空的 ordinary root 原位变为 junction、修改祖先 reparse、改变可变的卷/盘符别名，记录真实 Windows 错误码和最终打开文件的实际父目录身份。安全预期是操作被阻止或 child open fail closed，绝不能返回标记为 R 的 pin 却打开别处对象。

这项没有 Windows 执行证据，尤其未证明每一种改名/FSCTL/卷映射操作在当前持有句柄下能成功。[CreateFileW 文档](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew) 支持分析 sharing 与 final-component reparse 行为；[FSCTL_SET_REPARSE_POINT 文档](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_set_reparse_point) 描述原位操作，但不足以证明本夹具中的所有条件；[FILE_RENAME_INFORMATION 文档](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information) 又列有含打开对象的目录改名限制。因此，不能仅凭未 pin 全部祖先就宣布已复现 root escape。

**U2 — native custody / IPC /部署。** 未读取 allowed paths 之外的 store/same-open/C VFS/host IPC 实现，未证明 poison 调用点、UNKNOWN close 后真实资源留存、SQLite 对 DELETE sharing 的握手、borrow marker 跨 FFI 的落实、各进程的 DACL 或 token。`root/mod.rs:486-489` 自己也要求 private IPC wiring 验证 share-delete handshake。F1 的生产风险和 F2 的跨主体风险都需在这些边界上继续核实，而不能用局部 Rust 类型事实替代。

**U3 — 平台与失败注入。** 未在 NTFS/ReFS/可移动卷、Volume GUID 路径、多 session、强制退出、abandoned mutex、Set/GetHandleInformation 或 CloseHandle/ReleaseMutex 失败条件下运行。错误返回值被忽略的底层关闭/释放行为没有本次故障注入证据；也不将其本身夸大为已证明可触发的独立资源泄漏漏洞。

## CI、交付状态与接续

`.github/workflows/gogoke-native-host.yml:3-4` 只有 `workflow_dispatch`，**推送这个文档提交不会自动触发该工作流**。其 Windows job 在 `192-246` 一带运行 `cargo test --lib` 并拒绝零 executed/缺失摘要，`129-136` 与 `223-229` 一带记录 exact commit 和 Windows Server evidence scope。工作流具有 timeout 和同分支取消旧运行配置；这些是配置事实，不是本次执行结果。

本审计没有 CI run URL、测试通过数或 Windows 11 实机 PASS 可报告。非零 aggregate lib tests 也不能单独证明上述每个 root 测试均已注册并执行；需要日志中的测试名称和对应 summary。Windows Server 的成功不得外推 Windows 11 正式安装包结果。

远端恢复事实：任务开始时指定 ref 查询返回 404；随后只创建 `gpt/web-chatgpt-wcs27-root-boundary-audit`，已回读到精确 base SHA。写入本文件前再次确认该分支 HEAD 仍为 `fde3ad85cf6518e521c840ac1ca2e383e778d32e`，结果路径不存在。写入前只读观察到 `main` HEAD 为 `138c4c0547ac39996c8033011dce92d0b3891158`；这不是对 main 额外提交的审查，也不是将审查基准换到 main。本次没有遇到需要重试的远端写入失败或持续权限阻断。

**NEXT_ACTION（Controller）：** 核验本结果提交的唯一 parent 为 base SHA、唯一变更路径为本文件；在该结果提交上触发并核验真实 CI run，核对 run/head SHA、artifact 内 commit_sha、实际 executed tests > 0，并检查 main 是否有意外提交。需要验证 F1/F2 或 U1 时，另行在授权写域增加定向 Windows 云端反例；不可把本报告中的测试设计计作已执行。风险采用与产品验收由 Controller/Owner 判断。

本文件作为唯一结果文件提交；提交后仅回读验证远端落地，不追加第二个结果提交、不改生产代码、不创建 PR、不合并、不发布、不打标签、不接触密钥或签名。
