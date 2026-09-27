我已独立读取候选源码的关键可达路径、签名/域隔离、安装冒烟、卸载收尾与字节比较工具。云端 run/artifact 级证据因 `gh` 网络调用在当前模式被拒而无法独立回读。以下为完整审查报告。

---

# R2-06a 异构独立复核报告（外部 Claude 席）

**复核性质：只读、独立、非验收。** 本报告不构成 Owner acceptance、Goal Acceptance、release 授权或对任何 PR 的合并意见。未修改文件、未构建、未测试、未推送、未评论。

## 0. 精确对象与 SHA 映射（已独立核实）

用只读 `git` 核对，结论与委托一致：

| 对象 | 声称 SHA | 独立核实 |
| --- | --- | --- |
| 工作树 HEAD | cfbb1ba7 | ✅ `git rev-parse HEAD` = `cfbb1ba7e8a492387e02752d2591c62223dff16b` |
| 冻结产品源码候选 | 7e1b4f8c | ✅ `7e1b4f8c915c64dd5d6a6afb6759ee323c5f650f`（commit「fix: keep generated module policy outside native product root」） |
| 测试 workflow 字节 | 8f4b0a27 | ✅ `8f4b0a27cd637f04ec025c0b6fde6cab88f7ef18`（commit「test: require product-created candidate AppData before sentinel」） |

**候选→HEAD 差异（`git diff --name-status 7e1b4f8c cfbb1ba7`）**：仅两项——
- `A artifacts/s1-r4/checkpoints/MC-120.json`（检查点）
- `M tools/ci/gogoke-r2-06-candidate-installed-smoke.ps1`（安装冒烟脚本，来自 8f4b0a27）

**独立确认：产品源码（Rust/TS/native/CI 打包器）在 7e1b4f8c 冻结，HEAD 相对候选只多出安装冒烟脚本修正与 MC-120 检查点。** 委托的映射成立。MC-120 的自述哈希/边界仅作参照，未当作独立证据。

## 1. 曾致 Windows sharing violation 的模块策略位置修复（重点轴，独立核实机制成立）

**对象**：`apps/desktop/src-tauri/src/public_runtime/product_entry.rs:451`（`ModulePolicy::new`），配合 `resource_trust.rs:149 pin_ancestors`、`resource_trust.rs:141 pin_generated_file`。

**独立读到的机制**：
- `product_root = app_data_dir()/product-authority`（product_entry.rs:246-251）。
- 旧代码 `product_root.join(".gogoke-module-policy-<uuid>.json")` 把策略文件写在 **product_root 内**。`pin_generated_file → pin_file → pin_ancestors` 用 `FILE_SHARE_READ`（拒绝 DELETE 共享）钉住该文件父目录及全部祖先（resource_trust.rs:158-163）。父目录即 product_root → 与 native-host `RootLock` 对 `--root`(product_root) 需要的 DELETE 冲突 → os error 32。此因果与 MC-120 `path_reflection` 一致，且我在源码中独立复现了冲突路径。
- 修复改为 `product_root.with_file_name(...)`（product_entry.rs:451），策略文件成为 product_root 的**同级**（落在 AppData 标识根 `app.gogoke.desktop.candidate\`）。`pin_ancestors` 此时钉住的最深目录是 product_root 的父目录，**不再钉 product_root 本身**，RootLock 遂可取得 product_root 的 DELETE。
- 修复**保留**了原有约束：策略文件仍 `create_new(true)` + `FILE_SHARE_READ`（product_entry.rs:454-455），仍经 `lease.pin_generated_file` 做自身文件字节钉（product_entry.rs:465），`Drop` 时删除（product_entry.rs:486-490）。native root 身份/Job 规则未在此 commit 触动。

**真实产品可达性（关键）**：安装冒烟 readiness 阶段启动真实 `gogoke.exe`（脚本 457-465），其 bootstrap 经 `managed_service::run()`（product_entry.rs:914）→ `create_dir_all(product_root)`（:926）→ `ModulePolicy::new`（:930）→ launch。若 os error 32 仍在，managed service 会失败、readiness 收据永不落地、冒烟必失败。故 readiness PASS 在真实 shell 内证明了此修复路径生效——**该修复由真实产品可执行体验证，而非替身**。

**结论**：机制正确、约束保留、由真实产品可达路径覆盖。无阻断缺陷。

## 2. 安装冒烟是否验收真实产品而非替身（重点轴，独立核实为真实产品）

**对象**：`tools/ci/gogoke-r2-06-candidate-installed-smoke.ps1`、调用者 `.github/workflows/gogoke-candidate-sign.yml`（本分支为**消费者**）、`tools/ci/gogoke-candidate-installed-preflight.ps1`。

**独立读到的依据**：
- 冒烟对**真实签名 `setup.exe`** 走 NSIS 实装（`/S /D=`，脚本 256），随后把安装后 `gogoke.exe`/`gogoke-native-host.exe`/`gogoke-service\runtime\node.exe` **逐一 hash 绑定**到期望值（脚本 371-373）。期望值非来自 unsigned 元数据，而由 preflight 从可信签名产物核验后输出（见 §3）。
- 真实 `gogoke.exe` 启动并等待版本/generation/setId 绑定的 bootstrap readiness 收据（脚本 457-484）；`--uninstall --quiet` 真实收尾并等 DELETED finalizer 回执（脚本 499-539）；AppData sentinel 字节保留校验（脚本 549-552）。
- 全程要求 GitHub-hosted Windows、Medium 完整性令牌（gsudo pinned digest，`gogoke-candidate-sign.yml:82`）、并断言凭据环境变量对候选产品不可见（脚本 164-169）。

**8f4b0a27 修正的独立评估**：diff 将 `New-Item -ItemType Directory -Path $appDataRoot` 改为 `if (-not (Test-Path ... -PathType Container)) { throw '...did not create candidate AppData root' }` + `Assert-NoReparseAncestors`（脚本 437-440）。此为**更强**断言：它要求真实安装产品在更早的 service 冒烟阶段（脚本 390-394 以 `appDataRoot/product-authority` 为 `--root`，经 Rust `create_dir_all(product_root)` 创建）**自建了数据根**，否则冒烟失败。修正把「测试自造目录」换成「证明产品造了目录」——增强而非削弱真实性。这与 MC-120 记述的失败原因（产品先建、脚本再 New-Item 冲突致 uninstall NOT_RUN）自洽。

**结论**：安装冒烟以真实签名产物 + 真实产品可执行体 + 真实 NSIS 安装/卸载为对象，非替身；8f4b0a27 修正提高了真实性保证。

## 3. 按 R2-06a 必要轴逐项结论

轴依据 R2-06 §7「必要证据」并遵 Owner 在 MC-120 前的 06a/06b 拆分（更新/资源更新/回滚/崩溃恢复属 06b，不在本轴收口）。

| 轴 | 结论（源码层独立核实） | 主要依据 | 云端 run 级 |
| --- | --- | --- | --- |
| 双云端可执行体逐字节比较 | ✅ 逻辑成立 | `gogoke-desktop.yml` frozen/repro 双 lane（/Brepro）+ job「Compare independent Windows executable bytes」调用 `gogoke_ci_frozen_artifact.py compare`；compare 做真实 `first_bytes != second_bytes`（py:281），COMPARED_FILES 覆盖 portable shell、NSIS 安装 shell、native-host、node、pack、index（py:31-38） | 未独立核（见 §5） |
| 仅改资源/版本时 shell/host 哈希保持 | ✅ 机制成立 | NSIS `UNK→NSS` 由 `INSTALL_TOKEN=__TAURI_BUNDLE_TYPE_VAR_UNK`→`_NSS`（等长）机械派生安装后 shell（py:219-223），双 lane 比较安装版 shell 字节 | 未独立核 |
| 候选签名的实产品启动 | ✅ | §1、§2；readiness 经真实 shell managed_service | run 36289400367 PASS 未独立核 |
| 候选/正式安装与数据根分离 | ✅ | 候选域 Tauri identifier 改 `app.gogoke.desktop.candidate`（lib.rs:167-168）→ 独立 AppData；HKCU 键名/InstallDomain 分离（resource_trust.rs:837-874，reject_opposite_registered_root:877-893） | 未独立核 |
| 正式域拒绝候选签名 | ✅ | 域由磁盘清单文件决定，二者并存报 ambiguous（resource_trust.rs:584-587）；候选用 `CANDIDATE_KEY`+`GOGOKE-CI-CANDIDATE-RESOURCE-V1\0` 前缀，正式用 `OWNER_KEY`+空前缀（:596-601, verify_signature:378-399）；密钥与签名字节均异，互不回退；`stage_resource_update` 要求 current+signed 均 Formal，候选得 `GOGOKE_CANDIDATE_RELEASE_DISABLED`（:1460-1468） | 未独立核 |
| 编译期信任根隔离 | ✅ | `CANDIDATE_KEY`/`OWNER_KEY` 均 `include_str!` 编入（resource_trust.rs:25-26），非从资源/配置导入 | n/a |
| 资源/执行体 hash 与路径绑定 | ✅ | `signed_index` 校验 index SHA、pack SHA、generation_id==pack SHA、version、source_commit（:606-625）；`verify_bootstrap` 校验 installed_shell/native_host/node + generation + installed files（:1128-1148）；custom protocol 只服务已核 set（read_frontend_request:274-289，safe_relative_path 拒 `..`/绝对/reparse） | 未独立核（实际字节值） |
| 候选安装域（拒便携/工作树/仅命令行根） | ✅ | 候选域 `verify_bootstrap` 恒要求 installed_shell 字节 + `validate_install_registration`（:1128-1138）；便携 shell 字节或缺 HKCU 候选登记即失败 | 未独立核 |
| NSIS 安装注册/卸载与用户数据保留 | ✅ 机制成立 | 无 `uninstall.exe`（脚本 366-367）；HKCU `UninstallString="…gogoke.exe" --uninstall`（脚本 128-135）；`--uninstall` 在 WebView 初始化前分流并 `process::exit`（lib.rs:96-105，早于 :159 `generate_context!`）；收尾用固定系统工具 `%SystemDirectory%\WindowsPowerShell\v1.0\powershell.exe`（gogoke_uninstall.rs:885-896，reparse 校验）；生命周期锁经继承句柄交接 + `LOCK:{nonce}` 见证（:906-936）；root 文件身份 TOCTOU 双检（:842,877-882）；删除集=签名清单 owned files + 快捷方式 + 注册键，**数据根(AppData/product-authority)不在其中**，sentinel 保留校验（脚本 549-552） | 未独立核（DELETED 回执/201.5s） |
| native-host/Node 启动 | ✅ | managed_service 显式 handle-list 继承、Job KILL_ON_CLOSE、CREATE_NO_WINDOW（product_entry.rs:407-420）；node 以 `--import` 预载 module_guard 白名单启动（:575-594） | 未独立核 |
| 反例绑定（counterexample） | ✅ | `module_guard.mjs` 对未列模块 `writeSync(2,'GOGOKE_MODULE_NOT_LISTED'); process.exit(78)`（:26-29），覆盖 dlopen（:40-45）与 resolve/load hook（:46-56）；冒烟负例断言 `poisonExecuted==false` 且 `GOGOKE_PRODUCT_SERVICE_FAILED:78`（脚本 424-430） | 未独立核 |
| 同账户候选 JS 无 OS 沙箱（不记 PASS） | ✅ 与设计一致 | 设计 §3/§7 明确候选服务 JS 有同账户 I/O 能力，独立产品根不构成沙箱；该轴按设计**不记 PASS**，本报告亦不将其列为通过 | n/a |
| 独立复核 | 进行中（本报告） | — | — |

## 4. 缺陷 / 观察项（含文件·函数·复现路径·严重度）

未发现 R2-06a 收口阻断级缺陷。以下为低severity 观察项：

**O-1（LOW，卫生/数据域残留）**
- 文件/函数：`apps/desktop/src-tauri/src/public_runtime/product_entry.rs:451`（`ModulePolicy::new`）+ `Drop`（:486-490）。
- 描述：策略文件现落在 AppData 标识根（`app.gogoke.desktop.candidate\.gogoke-module-policy-<uuid>.json`）。正常 `Drop` 删除；但进程崩溃/被强杀于 `Drop` 前时会残留。文件名每次新 uuid，跨崩溃可累积。该目录属**用户数据域**且卸载 finalizer 只删 owned install files，故此类残留在卸载后仍留存于 AppData。
- 可证伪复现路径：在候选安装下触发 managed service 启动后于 `Drop` 前强杀 `gogoke.exe`；检查 `%APPDATA%\app.gogoke.desktop.candidate\` 是否残留 `.gogoke-module-policy-*.json`；重复观察累积。
- 影响评估：非安全绕过（`create_new` + uuid 阻止预置/冲突利用；文件仅在各自进程内被钉）。为数据域卫生问题，不改变字节稳定、签名隔离或卸载数据保留的正确性。
- 严重度：LOW。

**O-2（观察，非缺陷，测试面收窄）**
- 文件：7e1b4f8c 从 `bin.ts`、`nativeHostClient.ts`、`product_entry.rs`、`resource_trust.rs`、`gogoke-package-service.mjs` 移除了 `GITHUB_ACTIONS` 条件的临时诊断（stderr 转发、进程清单、install-error 回执）。
- 评估：这些原是 CI 条件分支，移除使冻结产品字节**不含 CI 条件行为**，对字节稳定/最小攻击面为**正向**。安装冒烟负例（exit 78）与正例（readiness）核心断言仍在；被移除的 `installedProcessInventory` 属额外诊断控制，非核心断言。记为测试诊断面收窄，不构成缺陷。

**O-3（覆盖粒度观察）**
- 描述：poison-negative（exit 78）在通过运行中经 `gogoke-package-service.mjs candidate-installed-service` 谐调用安装态 node+generation+guard 层验证；真实 Tauri shell 的 `managed_service.run()`（含 ModulePolicy 写盘）由 readiness **正例**覆盖。即「真实 shell 内的 ModulePolicy 拒绝路径」未由本次通过运行的负例直接穿过 Tauri shell 展示。
- 评估：正例已强制真实 shell 走通 ModulePolicy→RootLock（§1），负例在 guard 层证明拒绝语义；组合足以支撑修复，但真实 shell 内负例为可加强项。非阻断。

## 5. 未执行 / 证据不足的轴（明确记录）

- **云端 run/job/artifact 级证据全部未独立核实。** `gh run view` / `gh api` 属网络外呼，在当前 don't-ask 模式被拒（多次尝试均被拦）。因此以下**仅依脚本/工作流源码推断可行，未回读实机结果**：desktop run 36287202820（5 jobs、非零测试计数、reproducibility artifact 10921522825）、source hygiene 36287640351、trusted signing 36288886778 与 signed artifact 10921512845、branch installed smoke 36289400367 与 machine artifact 10922170630、frozen source artifact 10920878319、以及 MC-120 中 setup/installed-shell/native-host/node 的 **SHA-256 具体值**与 finalizer 201.5s。这些记为 **NOT_INDEPENDENTLY_VERIFIED**。
- **默认分支(main)签名器不在本工作树。** 本分支 `gogoke-candidate-sign.yml` 是**消费者**；真正持候选私钥、须核来源/分支/SHA/run/artifact 且不 checkout 待测代码的签名 workflow 位于 main 同路径（`.github/workflows/gogoke-candidate-sign.yml @ main`，名「gogoke candidate resource signing」）。我只能核实**分支侧**要求签名 run 具备该来源（preflight `Assert-Run … 'main' … 'gogoke candidate resource signing'`，:219），以及候选密钥用途前缀隔离；**签名器内部的密钥隔离与不执行分支代码**无法从本工作树独立审，记为 NOT_VERIFIED。
- **未构建/未运行 Rust 单测与 CI**（受托只读约束）。以上 Rust 逻辑结论基于静态阅读，非执行结果。

## 6. 是否存在 06a 收口阻断

**否（源码可达层）。** 在我能独立读到的产品可达代码、签名/域隔离、字节比较工具、安装/卸载/数据保留逻辑范围内，未发现 R2-06a 收口阻断级缺陷；模块策略位置修复机制正确并由真实产品路径覆盖，安装冒烟验收真实产品。

**但收口不应仅凭本报告完成**，因两类必要证据未被我独立核实：①全部云端 run/artifact/机器回执与 MC-120 内具体 SHA 值；②main 签名器内部隔离。建议在这两项由具备 `gh`/main 只读访问的一方独立回读确认后，再推进已授权的 Owner Win11 SAC 实机候选测试。O-1 建议在不改冻结字节的前提下由后续 06b/清理轮处理（例如启动时清扫本产品自有的陈旧 `.gogoke-module-policy-*.json`）。

## 7. 与 R2-06b / R2-05 的范围分隔

- **R2-06b（未验收、延后）**：完整更新(`full`)、资源更新(`resources`)、回滚、崩溃恢复。`stage_resource_update`（resource_trust.rs:1456）、`gogoke_update.rs`、update coordinator 等更新路径**不在 06a 收口**；我仅为「正式域拒候选」轴触及其入口守卫，未复核其更新/回滚/崩溃恢复正确性。
- **R2-05（仍待）**：Goal/结果链、PR/接受事实回读、最终审查未动；受管测试账本写入路径未在本轮复核范围深查。
- **PR #34**：Owner-held OPEN，本复核不涉合并。PR #27 OPEN/DRAFT。
- **正式发行**：未授权、未推断；候选 PASS/本复核均不代替 Owner 离线签名与 Owner Win11 最终证据。

## 8. 非验收声明

本报告为外部异构席的**只读独立复核意见**，非 Owner acceptance、非 Goal Acceptance、非 release 授权、非对任何 PR 的合并/放行意见。凡标「未独立核实/NOT_VERIFIED」之处，均须由具备相应只读访问的一方回读后方可作为收口依据。委托所述 MC-120 自述哈希与边界仅作参照，未被当作独立证据。
