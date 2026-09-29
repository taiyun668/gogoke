# WCS-26 — 固定提交 Windows 原生进程启动边界审计

## 结果与精确版本

- Task: `WCS-26`；角色：independent fixed-commit Windows native process-launch boundary auditor。
- Repository: `taiyun668/gogoke`。
- 完整任务卡：`docs/trials/web-chatgpt-subagent/WCS-26.json`，提交 `f12f3cf1445dc1dd800893e40e09aedec2617875`，blob `39bc235dd52a28b159839216b861abc633562ed5`。
- **Base SHA = Review SHA = `ef01e5e35f80e6bdffd4ef0eb657652522ac388d`**。下文所有仓库源码行号均属于这个提交，不属于浮动分支或当前 main。
- 唯一写入分支：`gpt/web-chatgpt-wcs26-process-audit`；唯一写入路径：本文件。
- 结论：`STATIC_AUDIT_COMPLETE_WITH_FINDINGS`，共四项静态发现。没有执行本地 CLI、原生编译、Windows 11 测试或动态利用验证；没有修改候选代码。**不是项目验收、安全认证或发布结论。**
- 测试状态：审计新增/执行测试数为 **0 / NOT_RUN**，不是 PASS。结果提交的真实 CI 与非零执行数由任务卡指定的 Controller 后续核验；本报告不预先声称通过。

严重度是修复优先级，不是已验证可利用性的分数：F1 为具备命名空间写权限前提下的 P1 候选；F2–F4 为 P2。每项分别注明确定的源码事实、额外前提和可证伪测试。未证明任意不可信 IPC 调用者能够执行这些 Rust API。

## 范围与证据定位

| 固定提交下的文件 | Blob SHA | 本次关注范围 |
| --- | --- | --- |
| `apps/desktop/native-host/src/process/windows.rs` | `fdbe73aa1a9293840705786997978259cf6510f8` | 创建、引用参数、摘要与身份、托管状态转换、相关内嵌测试 |
| `apps/desktop/native-host/src/process/mod.rs` | `28c95f7e7607303ccb45770c18fc7235acc4beda` | 对外保证与公开导出 |
| `apps/desktop/native-host/src/root/mod.rs` | `6effb3ac57b0aee21f9d654cea584827fbd52701` | 根身份、句柄/互斥锁、生命周期与路径转换；没有扩展成完整数据库/VFS 审计 |
| `.github/workflows/gogoke-native-host.yml` | `1819772eb324a12e9ca937268da2afd36f61a869` | 触发方式、Windows job、实际测试计数与机器证据 |
| `AGENTS.md` | `9f10c92267383e4a1d961e0ee404af3aad381fd9` | 仓库治理 |
| `docs/governance/gpt-construction-window-rules.md` | `7ef89e7abab1ad4a6eaff603f5fa906561fd4e79` | 第 2、3、4、5、5a、6、8 节 |

采用分段 GitHub 文件读取，没有克隆、下载执行候选、读取其他仓库源码或审阅旧审计结论。仅另查微软/Rust 官方 API 文档来核对平台语义，见文末。IPC 路由、调用方持久化事务、权限策略、安装目录 ACL、完整依赖与外部集成测试实现不在获准读取范围；不以猜测补足它们。

## F1 — 两次路径摘要仍未绑定到实际映射的可执行文件对象

**优先级：P1，条件性安全发现；实际 Windows 竞态未运行。**

**证据：** `apps/desktop/native-host/src/process/windows.rs:560-575` 先对 `request.launch.application` 调用 `file_sha256`，再创建进程，随后对 `prepared.identity.image_path` 再次调用它。`windows.rs:1221-1235` 每次独立 `File::open`，返回时该文件句柄即释放。`windows.rs:1074-1082` 从进程句柄取得的是路径文本；`windows.rs:1060-1072` 记录 PID、创建时间和该文本，没有可执行文件 File ID 或持有的文件对象。官方 `QueryFullProcessImageNameW` 文档只承诺返回镜像名称，不承诺后续按名称重新打开的是同一对象 [M2]。

**问题与前提：** 两次摘要相等证明两次读取到相同内容，不证明中间 `CreateProcessW` 映射了该内容。若较低信任主体能在这两个读取之间重绑定可执行路径或相关命名空间，存在 A → B → A 的检查/使用竞态：首次读取 A，创建时映射 B，捕获名称后再次读取前让该名称恢复指向 A。这里不是声称能原地修改已映射 PE；漏洞假设依赖替换/重命名/命名空间重绑定在目标文件系统、共享模式和 ACL 下可行。若安装目录、整个解析链与文件对象都已由其他机制可靠固定，此攻击前提可能不成立；这些机制不在本次源码证据中。

**影响：** `binary_digest_sha256` 可能被登记为 A，而实际将恢复执行的进程映射 B；精确 PID 与创建时间不能补回缺失的二进制对象绑定。创建后仍然挂起提供了拒绝机会，但第二次按名称散列本身不是对象证明。

**最小处置方向：** 由现有启动 authority 在首次散列至创建后验证期间持有并验证可执行文件的物理身份，建立不可被重绑定的路径/对象约束，且拒绝无法证明一致性的创建结果。单纯再散列一次或调用一次 canonicalize 不足。不要为此引入第二授权系统；本任务不实施改动。

**可证伪测试：** 在云端 Windows CI 的专用受控夹具中准备摘要不同、仅写不同标记的 A/B 两个测试镜像，在首次散列之后、镜像身份捕获之后设置确定性同步点，尝试上述有序重绑定。通过条件是拒绝该启动或证明运行的只能是 A；若接受 A 的 binding 后出现 B 标记，则确认发现。必须记录 Windows 的实际重命名/共享拒绝结果；若相关操作被受支持部署中的强制对象约束阻止，则降低/撤销该部署下的安全严重度。普通重复启动或单纯“错误 digest 被拒绝”的测试不能证伪这个竞态。

## F2 — 生产激活接口将 prepare 的回显当作 durable 身份，并不自行强制持久化/根授权

**优先级：P2，API/契约边界发现；外部 IPC 可利用性 NOT_VERIFIED。**

**证据：** `apps/desktop/native-host/src/process/mod.rs:3-5` 声称调用者在 custody callback 成功前不能取得 runnable process，`:7-11` 公开重导出 Windows API。`windows.rs:518-535` 中带持久化回调的 `prepare_and_activate` 却仅存在于 `#[cfg(test)]`。生产路径 `windows.rs:560-599` 返回完整 `PreparedCustody`；`windows.rs:601-628` 的 `activate` 只检查 nonce、ticket/tombstone、与内存记录相等，然后直接 `prepared.activate()`。原样返回值已经满足这些检查，没有数据库提交证据、单独的受信持久化确认类型或 RootLock 参数。

**具体反例：** 调用 `ProcessCustodian::new()`、`prepare(&request)` 后立即 `activate(&prepared)`，中间不持久化任何内容，就能走到 `ResumeThread`。`windows.rs:1487-1560` 的现有 two-phase 测试正是准备后测试错误身份，再将原对象直接传回激活；它证明身份匹配，不证明生产持久化屏障。`NativeBinding` 的 profile/domain/generation 校验见 `windows.rs:962-988` 邻近的 `validate_binding` 实现：它做格式校验，不查询权限 authority；不得把这些字符串看成授权证明。

**跨文件边界：** `root/mod.rs:599-665` 的根锁取得与再次核对物理身份，及 `root/mod.rs:758-767` 的 Drop 释放，属于另一个生命周期。进程 prepare/activate 的类型没有借用该锁，因此根锁本身不能替这些入口证明调用者已取得或仍持有根 authority。这不等于已证明生产调度器会提前释放根锁。

**影响与限定：** 在能够直接调用公开 Rust API 的边界内，调用者可以省略持久化步骤；模块文档的无条件保证过强。另一方面，`activate` 自身注释明确说依赖 service 声称已持久化，因此如果它有意仅供完全受信任、始终先提交事务的 service 使用，本项应按信任边界/文档与 API 收口处理，而不是宣称已找到远程权限绕过。当前允许路径不足以证明 IPC 暴露、调用身份认证或整个产品的事务顺序。

**最小处置方向：** 把实际唯一持久化/权限 authority 与激活调用衔接起来，令未提交的 prepare 回执不能冒充提交确认；或明确限制接口只供受信 service 使用、收窄导出并更正模块保证。沿用现有 authority，不复制权限系统。

**可证伪测试：** 对真实生产入口注入“持久化写入失败/提交未发生”，再尝试回显同一 prepared 记录；断言不恢复线程、无子进程标记，并保留或终止挂起托管。另测未持根授权的调用被现有 authority 拒绝。如果完整生产调用链能证明不可信调用方无法到达激活，且事务失败永不发送激活，则证伪本项的端到端权限绕过推论，但仍应修正文档与公开 API 所表达的保证。仅测试 `#[cfg(test)]` 回调返回 Err 不足以证明生产接口同样受保护。

## F3 — Job 分配失败后的清理忽略失败/超时，可能丢失未托管进程

**优先级：P2，失败路径托管完整性；没有声称子进程会自动恢复执行。**

**证据：** `apps/desktop/native-host/src/process/windows.rs:453-466` 中，`AssignProcessToJobObject` 失败后无条件调用 `TerminateProcess` 和固定 1,000 ms 的 `WaitForSingleObject`，两者返回值都未检查，随后仅返回原 `AssignJob` 错误。`windows.rs:436-442` 的 `OwnedHandle::drop` 只调用 `CloseHandle`。此时子进程没有加入刚创建的 kill-on-close Job，关闭该空 Job 无法替它完成终止。

**问题与前提：** 若终止失败且等待没有证明退出，或异步终止在等待预算内尚未完成，调用方只收到分配失败，没有可恢复的进程句柄或 residual-custody 记录。微软明确说明 `TerminateProcess` 是异步的、需要等待确认，关闭进程/线程句柄并不终止它们 [M5, M6]。若终止返回失败仅因为进程已经退出，且等待返回已退出，则不构成本项；不能把所有 TerminateProcess 失败都称为残留。

**影响：** 挂起或终止状态未知的进程及资源可能失去托管，后续无法用原始精确句柄确认清理。正常已加入 Job 的失败清理不能覆盖这个“尚未加入”的异常分支。

**最小处置方向：** 检查终止与等待结果；只有精确进程句柄已证明退出才释放最后的恢复信息。其余情况保存原始句柄与错误、返回显式 residual/unknown custody，并纳入现有失败收敛路径，而非把清理结果丢掉。

**可证伪测试：** 在 CI 专用故障注入 seam 中令 AssignJob 失败，并分别令 TerminateProcess 失败或令等待返回 WAIT_TIMEOUT/WAIT_FAILED；断言未退出的进程仍有受控 owner/恢复记录，或者已被明确确认退出。对照组为终止失败但等待已 signaled，允许正常清理。单测只检查返回 `AssignJob` 错误而不观察句柄归属，不能覆盖本项。未在本任务注入或运行这些系统调用故障。

## F4 — 可执行路径经有损 UTF-16 转换后参与身份与摘要判断

**优先级：P2，确定的表示/身份保真缺陷；主要影响是合法路径失败或记错路径，不单独证明任意代码执行。**

**证据：** `apps/desktop/native-host/src/process/windows.rs:1074-1082` 把 `QueryFullProcessImageNameW` 返回的 UTF-16 通过 `String::from_utf16_lossy` 转为 PathBuf；随后 `windows.rs:570-575` 对转换后的名称重新打开散列。`windows.rs:1163-1170` 还通过 `application.to_string_lossy()` 构造 argv[0]。对照 `apps/desktop/native-host/src/root/mod.rs:1366-1386`，同仓库根路径已使用无损的 `OsString::from_wide`。

**问题：** `OsStr`/Windows 宽字符串能表示非良构 UTF-16；Rust 官方文档说明 `OsString::from_wide` 可无损往返这些 code units [M3]。有损转换会把孤立代理码元换成替代字符，可能得到不存在的路径，或碰到另一个具有替代字符名称的对象。原始应用路径与 CreateProcessW 输入仍可保留原始宽字符，但捕获后的“精确”身份与 argv[0] 不再保真。

**影响与限定：** 正确镜像可能在创建后摘要阶段被错误拒绝；即使同摘要别名文件使它通过，记录的名称也不是原始对象名称。与 F1 的可变命名空间问题不同，本项仅由表示转换即可发生。没有验证目标文件系统和产品入口是否允许该路径类别，也没有声称该问题单独绕过摘要校验。

**最小处置方向：** 身份路径沿用 `OsString::from_wide`/`OsStr::encode_wide`，命令行直接以宽字符构建，或在产品契约入口明确拒绝非良构 UTF-16，不能先有损转换再把结果用作 authority/摘要路径。诊断显示的 lossy 转换与身份用途分离。

**可证伪测试：** Windows CI 中用 `OsString::from_wide` 构造含孤立代理码元的受控夹具路径，并另建替代字符名称的对照对象；要求捕获路径按 code units 与真实路径完全相同、摘要仍读取正确对象、argv[0] 不被静默改写。可先用 API 返回值夹具做确定性转换测试，再做实际文件系统测试；若产品入口明确拒绝该类别，测试应证明拒绝发生在创建前而非创建后误读别名。所有测试均为建议，未执行。

## 已覆盖但不另报漏洞的边界

**命令行：** `windows.rs:1172-1200` 的普通参数引用对空串、空格/tab、嵌入双引号及引号前/尾部反斜杠的处理，与微软 C 运行时规则相符 [M4]；`windows.rs:1836-1847` 邻近已有相应字符串测试。创建使用非空独立 application_name，而非从命令行猜测 exe，避免了空格路径的经典 Program.exe 歧义（`windows.rs:999-1057`；[M1]）。这不是对 cmd、PowerShell 或自定义解释器语法的通用转义保证，也没有动态 round-trip 证据。NUL、过长参数及非 CRT parser 的负面轴仍需实际测试；未仅凭“没有额外检查”捏造 NUL 绕过。

**句柄与环境：** 同一 CreateProcessW 调用把 `inherit_handles` 设为 0；进程/线程安全属性为 null，且进程、线程、Job 另清除并检查继承位（`windows.rs:453-493`）。根身份/绑定/互斥句柄也有继承检查（`root/mod.rs:683-699` 及 `root/mod.rs:1388-1410` 邻近实现）。静态未见通用句柄继承开启。可是 `environment = null` 仍继承宿主环境，省略 current_directory 仍继承宿主工作目录 [M1]；ProcessLaunch 没有显式环境策略。不能把“句柄不继承”解释为环境隔离。可用无敏感内容的 `WCS26_ENV_PROBE` 和受控 cwd 回显测试验证，未读取任何真实环境秘密；是否违反产品策略因上层策略不在范围而未独立报漏洞。

**挂起/恢复：** 正常路径为 CREATE_SUSPENDED → 加入 kill-on-close Job → 捕获身份 → 保存 prepared → activate；句柄封装没有公开 raw handle。`windows.rs:496-499` 只把 ResumeThread 的 -1 当失败；其余返回值是前一次挂起计数 [M7]。本范围没有找到能够导致非 1 计数的普通调用路径，因此没有把调试器/其他进程干预假设单独升级为已确认漏洞。

**根 authority：** 根身份来自目录句柄与 FileIdInfo，取得互斥锁后再次绑定核对；数据库 pin 带 RootLock 生命周期，RootLock 有线程约束。这些局部措施不能单独证明进程调用者的权限，见 F2；亦不等价于把子进程限制在某个目录、令牌或文件系统沙箱。

## 测试、CI 与未验证点

1. **本次审计实际执行：0 个测试。** 没有运行本地 CLI、Rust/C 编译、Win11、签名、发布或候选变更；四项证伪测试均为待实施建议。源码中的测试存在不算 executed evidence。
2. 固定 review SHA 的远端 Actions 查询使用 `GET /repos/taiyun668/gogoke/actions/runs?head_sha=ef01e5e35f80e6bdffd4ef0eb657652522ac388d&per_page=2`，返回 `total_count=2`：`36224330707`（web ChatGPT subagent skill）和 `36224330698`（gogoke public source hygiene），元数据均 completed/success。这两个不是 native-host workflow；没有检查它们的实际测试计数，也不将其成功标记转换为本审计的原生 PASS。该次精确 SHA 列表未提供 native-host 执行证据。
3. `.github/workflows/gogoke-native-host.yml:1-4` 只有 workflow_dispatch，因此不能假设本结果文件提交会自动触发该流水线。`:101-139` 与 `:201-256` 的逻辑要求机器摘要/非零 executed 并绑定 commit_sha、run_id、attempt；`:252-259` 邻近的证据上传步骤提供对应 artifact。这是配置检查，不是本结果提交已经运行的证明。
4. **结果提交 CI：PENDING_CONTROLLER / NOT_VERIFIED。** 本文件内容形成时结果 commit 尚不存在，不能自称已有该 SHA 的 CI。按任务卡 Controller 应解析包含本文件的唯一结果提交，在该 exact SHA 上核验真实 CI、`executed > 0`、失败/跳过数和机器证据，并检查 main；不得拿 review SHA 或别的分支的绿灯代替。Windows Server 云端执行不替代 Owner Windows 11 正式安装包结论。
5. 未验证：F1 的真实文件系统竞态；F2 的生产 IPC 身份/权限与事务路径；F3 的系统调用异常组合；F4 的部署路径准入；动态 argv 往返、环境/cwd 策略、非标准继承/令牌情形、SAC 行为。没有把这些未知点写成已通过或已证实利用。

## 远端状态、恢复与结束边界

开始时指定 ref 查询返回 404；随后只创建 `gpt/web-chatgpt-wcs26-process-audit`，指定起点为 base SHA，并回读确认其 object SHA 为 `ef01e5e35f80e6bdffd4ef0eb657652522ac388d`。这是本次最早的 durable 固定起点。任务卡要求“一个结果文件提交后停止”，因此没有另造 WIP/checkpoint 提交。

唯一结果写入前再次回读：指定分支仍在 base SHA，本结果路径返回 404，可按新增文件提交；main 初次与写入前均为 `138c4c0547ac39996c8033011dce92d0b3891158`。本任务没有对 main、其他分支、PR、tag、release、密钥或签名进行写操作。结果提交后的 ref、单文件 diff 与 main 可由远端收据回读以及 Controller 独立核验；不为了在本文件嵌入自身 commit SHA 再制造第二次提交。

一个只读 CI URL（workflow-name 子路径）被连接器以 400 allowlist 拒绝，已改用上面的 repository Actions runs 接口得到精确 SHA 结果；这是工具 URL 层限制，不是 GitHub 写权限结论，没有因此停工或重复有副作用的写入。结果写入前无持续权限/平台阻塞。

读取纪律偏差如实记录：定位治理第 8 节时，请求第 121–150 行的响应包含了相邻 8a/8b 开头；没有据此执行长期窗口机制，也未继续读取第 9 节。后续读取保持固定 SHA 和本任务允许路径。

完成此唯一结果文件提交后停止；不修复候选，不发 PR，不合并，不等待/代执行 Controller 的后续验收。

## 平台语义来源（只用于核对 API，不是动态测试证据）

- [M1] Microsoft, CreateProcessW：独立 application_name、句柄继承、null environment/current_directory 语义。`https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw`
- [M2] Microsoft, QueryFullProcessImageNameW：返回可执行镜像路径名称。`https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew`
- [M3] Rust, OsStringExt::from_wide：非良构 UTF-16 的无损转换和 code-unit 往返。`https://doc.rust-lang.org/std/os/windows/ffi/trait.OsStringExt.html`
- [M4] Microsoft, Parsing C command-line arguments：空白、双引号和反斜杠解析规则。`https://learn.microsoft.com/en-us/cpp/c-language/parsing-c-command-line-arguments?view=msvc-170`
- [M5] Microsoft, TerminateProcess：异步终止及等待确认。`https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess`
- [M6] Microsoft, CloseHandle：关闭进程/线程句柄不终止进程/线程。`https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-closehandle`
- [M7] Microsoft, ResumeThread：返回前一次挂起计数。`https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-resumethread`
