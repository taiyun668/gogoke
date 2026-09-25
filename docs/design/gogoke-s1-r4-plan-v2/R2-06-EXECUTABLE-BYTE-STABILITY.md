# R2-06 可执行文件字节稳定：S1-R4 v2 增补设计

**状态：DESIGN_PROPOSAL；未授权施工，未验收。** 本稿起点为 `main@0c2d601f5180eadd49e1ca4b295140438de0b135`，按 `main@9e1eb560a0b3e036f1de7ce313ae0c7cdbd0e33e` 的 [AGENTS.md「宗旨」](https://github.com/taiyun668/gogoke/blob/9e1eb560a0b3e036f1de7ce313ae0c7cdbd0e33e/AGENTS.md)修订，并参考 execution branch 的 `MC-082` 至 `MC-084` 平台证据。Owner 本人合并本设计 PR 后，才启动 R2-06 施工；合并前不修改生产代码、构建链、R2-05 现场或 PR #34。

本稿是 [v2 计划](PLAN.md)的**增补包**，不建立新的计划版本。R2-06 依赖 R2-04 的真实随包路径；R2-05 已形成的 WIP 保留，但其正式安装包实机收口须等 R2-06 候选。现有 `PLAN.json`、`MANIFEST.json` 与已合并的授权 receipt 字节不变，避免使 R2-01 至 R2-05 的当前授权失效。R2-06 的单独施工边界来自 Owner 本轮明确指令及其本人合并后的本文件精确字节；不把设计 PR 或现有 receipt 冒充完成验收。V01–V08 的旧状态也不因增补自动改写。

## 1. 起因、目标与边界

Owner Windows 11 的 Smart App Control 强制模式对 `041a4966` 安装程序和便携 `gogoke.exe` 分别产生 3033/3077，二者均在启动前被拦；`985e17d1` 对应安装程序曾成功安装，其便携 `gogoke.exe` 在同机同类启动流程中也运行成功、未见 3033/3077。四份对象的哈希和事件在 `MC-078`、`MC-082`、`MC-083`、`MC-084`。这只是不同字节、目录和时间下的观察，**不能推断 SAC 只看哈希、已建立放行名单，或旧字节未来必然放行**。[构建与发布治理](../../governance/gogoke-build-and-release.md)的云端原生构建、不关闭 SAC、不做代码签名、Owner 离线签清单及正式安装实测边界继续有效。

目标是让纯前端、服务脚本和其他非可执行资源的变更不改变 `gogoke.exe` 与 `gogoke-native-host.exe` 的字节，并让运行中的产品在验签后切换资源。任何 `.exe`/`.dll`、Node runtime、原生协议、编译期公钥/允许主机/协调器或 shell 安全逻辑变化，都属于**可执行文件更新**，不能标为资源更新。字节稳定降低因资源改动制造新可执行文件的次数，**不保证 SAC 放行稳定字节**。

## 2. 产品启动与资源信任链

当前 `tauri.conf.json` 的 `frontendDist=../dist` 经 `tauri::generate_context!()` 编入主程序；`lib.rs` 在 setup 前已有自动创建的主窗口。R2-06 将用户可见的 HTML/JS/CSS 移到安装资源代，生产 shell 不再嵌入 `../dist`。最多保留固定的原生错误提示；它不能承载产品前端或取得产品 IPC 权限。

启动顺序固定为：解析实际安装/便携资源根与明确的运行域 → 读取该域的签名清单和 `resource-index.json` → 用**编译期且相互独立的公钥**验签 → 检查清单用途、版本、索引 SHA-256 与运行域一致 → 从索引核对当前 shell、native-host、Node 与资源代的路径/长度/hash → 才注册只服务该已核资源代的本地 custom protocol 并创建主 WebView → 才准启动 T3 服务及 native-host。正式发行/更新域**只**接受 Owner 签署的 `SHA256SUMS.windows`；隔离候选域只接受下文 CI 签署、明确 `CI_CANDIDATE_RESOURCE` 用途的候选资源清单。两个验签入口不互相回退。签名、索引、文件或路径任何一步不成立，就不加载前端、不建立产品任务入口，保留有界错误证据。公钥、允许主机及更新协调器仍是编译期常量，不从资源、配置、Context 或数据库导入。

具体 Tauri seam 是让生产 `main` 窗口在验证后才创建，使用 `WebviewUrl::CustomProtocol` 与 `Builder::register_uri_scheme_protocol` 服务资源；`menu.rs` 的 About 窗口也改由同一已验证资源代加载。协议只接受索引列明的规范相对路径，拒绝 `..`、绝对路径、大小写碰撞、额外文件与 reparse/symlink；读取同一份已打开并算过 hash 的字节供 WebView 使用，避免“先验路径、后读另一份”。锁定的 Tauri 2.10.3 上 Windows origin、IPC 能力和 CSP 必须由**真实云端安装后的 WebView**验证；现有宽 `assetProtocol` scope 与空 CSP 不能直接扩展到这个资源目录。[Tauri custom protocol API](https://docs.rs/tauri/latest/tauri/struct.Builder.html)、[WebviewUrl](https://docs.rs/tauri/latest/tauri/enum.WebviewUrl.html)。

`resource-index.json` 由云端机械生成，记录 source commit、资源代 ID、每个文件的规范相对路径/长度/SHA-256，以及 shell、NSIS 安装后 shell、native-host 和 Node 的哈希；它不自称候选、发行或已接受。同一冻结索引可以先受 CI 候选资源签名约束，正式发布时再由 Owner 签署的 `SHA256SUMS.windows` 绑定；签名与 sidecar 变化不重编或改写 exe、installer 和资源包。资源入口使用同一索引，不允许第二套 DB/配置事实或可写“已验”标记替代实际签名和字节。

## 3. 候选与正式发行的签名边界

### 候选资源：CI 专用密钥，无逐候选 Owner 触点

CI 在无秘密的构建 job 冻结 source SHA、安装程序、两个可执行文件、资源包与 `resource-index.json`，并完成两次云端逐字节比较。随后由**默认分支上的受信签名 workflow**读取这些产物为数据，独立核来源仓库、允许的受控施工分支、exact SHA、run/artifact identity、成功的构建 job、文件清单与 hash；只把符合这些条件的资源索引和资源包作为签名输入，不接受调用者自选的任意 payload。它不 checkout 或执行待测 PR/分支代码。该 workflow 从仅允许受信分支使用的 GitHub Actions `candidate-resource-signing` environment secret 取得**候选专用私钥**，只输出带固定 `CI_CANDIDATE_RESOURCE` 用途、`TEST_ONLY` 标记及 exact source/run/资源索引 hash 的候选清单和签名。候选签名输入带独立的用途前缀；签名 job 不取得 release 发布权限，私钥不写仓库、构建树、日志或 artifact。环境不配置逐次人工 reviewer；普通 PR/fork 构建 job 永远拿不到该 secret，也不用 `pull_request_target` 去执行其代码。[GitHub 环境 secret 与分支限制](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments)、[不可信 PR 与高权限 workflow 边界](https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target)。

候选公钥与 Owner 公钥都固定编进**同一份**稳定 shell；候选 sidecar 只在显式隔离的候选安装/测试域验签，绑定测试 root、测试身份和许可写入的测试 ledger 分支。候选标记、命令行开关或 HKCU 登记本身都不是权限；native Product Authority 必须核测试身份与目标分支，拒绝候选域触及正式安装、正式产品数据和生产 fact write。该域不做正式更新、公开发布、生产事实采用或 Goal Acceptance。正式安装/更新域不调用候选验签器；缺 Owner 签名就拒绝。即使候选密钥持有者用通用签名算法签任意字节，**正式发行验证器也只认 Owner 公钥和正式清单格式**；“只能签候选”由受信 CI workflow 的输入约束、签名用途隔离和消费端拒绝共同实现，不谎称私钥的数学运算本身只能处理某类文本。候选密钥的生成与一次性 CI secret 配置由 Controller 在本设计合并后的授权范围内处理；Owner 发行私钥永不进入 CI。候选公钥轮换会改变 exe 字节，必须走完整可执行文件更新。

候选使用与最终正式发行**同一冻结** `setup.exe`、`gogoke.exe`、native-host 和资源包字节；区别仅是外部候选 sidecar 与运行域。运行候选安装程序前还须以可信 GitHub run/artifact 回读的精确 hash 核本地 `setup.exe`，候选资源签名不能代替安装程序来源核验；安装后再核实际 exe 字节。CI 和获授权的 Owner Win11 实机测试可直接验候选签名并运行产品，不请求 Owner 为每个候选签名、手动转交产物或点击测试控件。候选执行仍只是 `TEST_ONLY` 平台证据，不是正式 release、adoption 或 R2-05 最终验收。

### 正式发行：仅 Owner 清单签名

`SHA256SUMS.windows` 继续是 Owner **离线签署**的正式发布集合；增加严格的 `full` / `resources` 发行种类、版本、索引和资源包条目。`full` 必须恰有一个精确安装程序、便携 ZIP、资源包和索引；`resources` 必须恰有资源包和索引、**不得有安装程序**。解析拒绝重复字段、重复名称、缺项、混合种类、候选标记和版本不一致。资源索引把 Owner 签名传递到每个安装后的文件；正式清单直接绑定发行资产哈希。现有 `gogoke_update.rs`、`tools/sign-gogoke-release-manifest.ps1` 和 `tools/publish-gogoke-release.ps1` 在施工时按这两种精确形状调整；**Owner 发行私钥**不进入 CI、仓库、构建树或产物，正式发布仍需 Owner 的显式决定。候选签名及其公钥不能通过 release 检查、更新检查或资源加载的正式域。

构建/签名顺序必须避免自引用：云端先完成资源包、两个可执行文件与 NSIS 安装程序；从 NSIS 实际承载的 shell 字节，或经精确 token 数和内容校验的 Tauri `UNK→NSS` 变换，计算**预期安装后** shell 哈希，生成冻结索引和清单草稿。CI 候选签名先供精确产物的云端/Win11 实测；候选合格后 Owner 才对**同一资产字节**的最终 `SHA256SUMS.windows` 离线签一次，绝不重编/重包 exe 或 setup。候选 sidecar 与正式清单/签名/索引都在安装程序旁，不编入它所校验的安装程序；NSIS 只接受两种形状之一，复制 sidecar 和初始资源到隔离候选根或正式安装根对应的版本化资源代。缺项时在改动旧安装前拒绝。正式首装由受信的清单验证步骤先核 Owner 签名及安装程序 hash；程序启动后的自检不能倒过来证明安装程序原本可信。正式签名后的云端安装烟测再量**实际安装** shell 字节并与索引相等，且实际走 Owner 验签域；这一步不重新编译、不自动发布。sidecar 避开“安装包包含校验自身的最终清单”这一哈希循环。

现有版本的 `gogoke_update.rs` 只暂存安装程序，不能自动向新 NSIS 提供 sidecar。首个 R2-06 架构版本需要上述完整集合受控引导安装；经既有授权，Controller/agent 负责核字节、安装、取回执与失败回滚，不把 Owner 当人工安装或回执转发员。旧 updater 的 installer-only 路径不能说成已兼容；新版本进入后，后续完整可执行文件更新才由修改后的既有更新链暂存 sidecar、走协调器/正式安装程序与回滚。

## 4. 稳定且可复现的可执行文件

生产发布版本、source commit、CI run/build 时间、前端 hash 和 release notes 只从已验证的索引/清单读取，不进入 PE 资源、Rust 常量、Tauri 编译上下文或 `GITHUB_SHA`。当前 `gogoke_update.rs` 的 `CARGO_PKG_VERSION` 比较与 readiness、`product_entry.rs` 的 `option_env!("GITHUB_SHA")` 草稿 provenance，须改取已验发行身份；不能把旧执行来源改标为新候选。Windows 文件版本若工具链要求固定字段，只标 shell ABI/固定值，不代表产品发行版本；安装注册表 `DisplayVersion` 与 UI 使用签名版本。

编译期保留稳定的安全常量：用途互斥的 Owner/候选公钥、允许主机、协调器逻辑、NativeBinding/SQLite 固定实现及 native-host 对当前 Node runtime 的 digest 绑定。Node、SQLite 绑定、任一公钥或协议改变时 exe/host 字节变化，必须走完整可执行文件更新；不能为了让哈希相等而删掉这些校验。Windows 图标、Tauri capability、Rust/Tauri/链接器变化若进入 PE，也按 shell 更新处理。

云端从**同一 source SHA**在两个独立、干净的 Windows 构建工作区，用锁定依赖、工具链、目标与配置各构建一次；逐字节比较原始 `gogoke.exe`、NSIS token 处理后的安装版 `gogoke.exe` 和 `gogoke-native-host.exe`，任一不同即失败并保留首个差异证据。当前 Tauri bundler 将主程序的 `__TAURI_BUNDLE_TYPE` 从 portable 的 `UNK` 确定性改为 NSIS 的 `NSS`（`MC-077`）；不能拿 portable 哈希代替安装后哈希。资源/版本仅变化的云端场景还要证明重新产出的三个可执行文件哈希与上一冻结 shell 相同；真正 shell/host 变化则明确产生新身份。任何消除时间戳、路径或 linker 非确定性的调整都由这个字节测试驱动，不能用“理论可复现”代替实际比较。原生构建和测试只在云端运行；所有新增 job 保留 timeout 和同分支取消旧 run。

## 5. 由 `gogoke.exe --uninstall` 卸载

`--uninstall` / `--uninstall --quiet` 在 Tauri/WebView 初始化之前分流，只允许从已登记的正式安装或**隔离候选安装**的 `gogoke.exe` 执行；便携副本、未知注册根或已改指向的路径拒绝。候选与正式安装使用不同登记和数据根，不能相互卸载。先核当前 exe 与对应 HKCU 安装登记的规范根一致、安装清单/拥有文件集合有效、所有待删路径和祖先无 reparse、用户数据根不在任何待删子树，再停止受管进程。产品数据、Project、Context、settings 和用户自行加入安装目录的文件不因卸载被递归删除。

主程序不能删除仍映射的自身；它完成验证后只启动**固定的系统工具**执行受限收尾，等待主进程实际退出，再删除列明的产品文件、属于本产品的快捷方式与 HKCU 卸载登记，写有界结果回执。收尾的删除集合来自已验证的固定规则和安装文件清单，不执行可更新资源中的任意脚本；失败保留现场，不能以等待超时代替“文件已删除”的检查。此形状借鉴 [Sandglass `uninstall.py` 的安装根/manifest/reparse 与状态目录检查](https://github.com/taiyun668/Sandglass/blob/bf043c8afbf96540e72cfa3ef2c072bb03d2c200/sandglass/uninstall.py#L124-L250)及其父进程退出后收尾；不直接复制 Python 实现。

NSIS 使用锁定 Tauri 支持的[自定义 installer template](https://v2.tauri.app/distribute/windows-installer/#installer-template)：不生成、安装或登记 `uninstall.exe`；HKCU `UninstallString` / `QuietUninstallString` 改为 `"<install-root>\\gogoke.exe" --uninstall [--quiet]`，参照 [Sandglass NSIS 登记](https://github.com/taiyun668/Sandglass/blob/bf043c8afbf96540e72cfa3ef2c072bb03d2c200/packaging/sandglass.nsi#L1092-L1134)。旧版本已有的 `uninstall.exe` 只在迁移成功后作为旧安装文件处理；不得在失败回滚时删除旧安装或让新登记指向不存在的程序。云端正式安装/卸载烟测必须证明 `uninstall.exe` 不存在、登记与删除范围正确、用户数据逐字节保留。

## 6. 更新：可执行文件路径与资源路径分开

| 已签发行类型 | 判定与执行 | 成功边界 / 失败处理 |
| --- | --- | --- |
| `full` | 任一可执行文件、Node、原生绑定、编译期信任根或 shell 逻辑变化；沿既有 GitHub 固定源、签名清单、安装程序与 update coordinator 路径。协调器额外携带 sidecar，保留安装目录备份、安装/注册表恢复。 | 新正式安装的 Gogoke 验签、载入资源、启动所需子进程并写目标版本/资源代 readiness 才成功；安装程序、新 exe 或子进程被 SAC 拦、就绪错误/超时都回滚并记录精确组件/hash/事件。 |
| `resources` | 清单和索引明确证明当前 `gogoke.exe`、native-host、Node 及所有其他可执行字节**逐一相同**，资源包只含允许的非可执行文件。运行中的产品下载、验签、解包到新版本化资源代并切换；不下载、不启动新的安装程序或可执行文件。 | 既有 update 锁和状态机负责序列化；在无不可安全中断的任务时切换当前代，WebView 从新代重载、服务请求改用新代，观察同版本/资源代 readiness 后才退休旧代。签名/字节、任务安全点、启动或就绪失败则回到旧代；崩溃恢复也保留旧代。 |

资源代以不可变目录承载，`SHA256SUMS.windows`、签名、索引与资源文件随代存放，不原位覆盖当前 sidecar。正在运行的请求继续钉住旧代，真实 `beginCommitted` 后不因资源切换杀进程或盲重发；不能安全切换就延期，不能伪造 readiness。切换的是已验证代的原子指针，不对正在服务的文件原位覆盖。每次启动及每次从磁盘供给前端字节仍验签/验实际读取字节；旧代只在无句柄且新代就绪后清理。资源状态只是本地协调，不成为第二 accepted fact ledger。

`gogoke_update.rs` 当前只识别带 installer 的 release，`gogoke-update-coordinator.ps1` 也直接启动它；施工必须显式加 `resources` 分支，同时保留 `full` 的版本绑定、重核 GitHub release、受信 host、缓存 hash、用户确认、readiness 与 rollback。新发行版本须高于当前已验版本；只有受控失败回滚可恢复旧代。签名证明 Owner 授权字节，不单独证明它是最新版本。`tools/publish-gogoke-release.ps1` 当前要求安装程序与便携 ZIP；资源发行改为**不生成安装程序**的精确资产集合，仍由 Owner 离线签名及显式发布。资源更新不能绕过既有 release/Owner 控制，也不能自动采用 Dream 或 Goal Acceptance。

候选域不执行正式 release 检查或更新；其候选签名只用于实测已冻结资源。`full` / `resources` 两条正式更新路径均只调用 Owner 验签器，不能因为候选签名存在就降级到候选根或把候选资源提升为正式代。

## 7. Smart App Control 与收口证据

首个新架构版本的 `gogoke.exe` 仍是新字节，正式安装程序、shell、native-host、Node 各受 SAC 独立判断；复现与资源签名不能向 Windows 提供 publisher reputation，也不保证一次放行后持续放行。Owner Win11 强制模式若拦截任何对象：Controller 停止该尝试，记录 4551（若实际观察到）、3033/3077 的**真实目标路径、时间、组件与 SHA-256**，保全旧安装/用户数据；不重试变通、关闭/绕过 SAC、引入自签/OV/EV 或本机原生编译。可安全回滚的更新自行回滚；不能证明恢复的轴记 `BLOCKED/NOT_RUN`。普通平台失败由 Controller 处理，只有后续必须改变 Owner 冻结的安全或产品边界时才交 Owner 决定。

R2-06 的必要证据为：精确 SHA 的双云端可执行文件逐字节比较；只改资源/版本时 shell 与 host 哈希保持；CI 候选签名的实产品启动、隔离与正式域拒绝；Owner 正式签名后的相同 exe/installer 字节启动与篡改拒绝；云端正式安装和 `--uninstall`/数据保留；资源更新无安装程序进程、失败回滚与崩溃恢复；Owner Win11 的**正式安装包**实际产品入口、所需每个子进程与资源更新观察；独立复核。每项记录实际 run、进程、文件 hash、非零测试数、fail/skip。候选实测**不等待 Owner 签名**；云端 PASS、便携运行、单个旧版本放行或候选签名都不代替正式发行签名和最终 Owner Win11 证据。R2-05 仍待它自己的 Goal/结果链、PR/接受事实回读和最终审查；PR #34 在本设计 PR 中保持打开、不合并。

## 8. Owner 触点逐项核算

[最新 AGENTS.md 的宗旨](https://github.com/taiyun668/gogoke/blob/9e1eb560a0b3e036f1de7ce313ae0c7cdbd0e33e/AGENTS.md)要求 Owner 定方向、安全边界和不可逆动作，不承担候选签名、运行 CI、复制回执、点产品测试或盯普通失败。R2-06 的触点逐项如下；“无”是设计约束，不是把劳动藏在手工步骤里。

| 动作 | Owner 触点与仅由 Owner 做的理由 | 系统/Controller 承担 |
| --- | --- | --- |
| 本设计 PR #35 的一次合并 | **一次**。Owner 明确要求本人合并后才授予 R2-06 施工边界；这是范围授权，不是逐候选门禁。 | 写设计、核 diff/CI、处理复核意见；合并后按精确字节施工。 |
| CI 候选密钥的用途/信任边界 | **本轮方向决定已给出；无逐次触点。** 若以后扩大用途或更换安全根，需要 Owner 再定边界；不能把密钥维护调成每候选审批。 | 在已授权边界内一次性生成/配置 Actions environment secret 与固定候选公钥、审计权限；权限核验自动化。若 GitHub 管理权限真实不足，仅请求所缺的权限，不让 Owner 代贴私钥。公钥轮换按完整 exe 更新处理。 |
| 每个候选的构建、签资源、云端与 Win11 实测 | **无签名或手工测试触点。** CI 保管候选密钥；既有本机安装授权覆盖时由 Controller 执行，不让 Owner 点按钮或传截图。若新的本机副作用或 OS 安全确认超出现有授权，只就该边界申请一次授权。 | 无秘密构建 → 受信 CI 候选签名 → 精确候选测试 → 自动收集 SHA/run/进程/SAC 证据；普通失败修复与重测。 |
| 正式 `SHA256SUMS.windows` 离线签署 | **每次 `full` 或 `resources` 正式发行一次。** Owner 发行私钥按治理始终离线且只由 Owner 保管；放进 CI 会降低已冻结信任边界。 | 冻结资产与清单、机械核全部 hash、准备一次可审阅签名输入；签后自动验签和跑同字节云端烟测，不请 Owner 手工搬运回执。 |
| R2-05 最终验收 | **一次明确裁决。** Goal Acceptance 与产品验收权仍在 Owner；候选 CI PASS 或独立复核不能代替。 | 提供真实产品、精确字节和全部必要证据；自动收集与整理，不让 Owner 逐项复现。 |
| 公开 release | **每次发行一次明确决定。** 公开发布是不可逆对外动作，不随 R2-05 验收或候选 CI PASS 自动发生。 | 准备可审阅的精确发行集合；获明确授权后由 Controller 执行发布、核远端事实。日常 PR、CI 和评论不升级成 Owner 微审批。 |
| SAC 拦截或其他故障 | **默认无。** 只有证据证明继续必须改变冻结架构、安全设置或 Owner 产品能力时，才需要 Owner 方向决定。 | 停止该尝试、记录组件/hash/事件、受控回滚并推进独立安全工作；不因单个失败要求 Owner 选择命令或路径。 |

PR #34 的 Owner 个人合并约束属于既有 R2-05 决定，本增补不更改，也不为 R2-06 另造同类触点。
