# R2-06 可执行文件字节稳定：S1-R4 v2 增补设计

**状态：DESIGN_PROPOSAL；未授权施工，未验收。** 本稿以 `main@0c2d601f5180eadd49e1ca4b295140438de0b135` 为设计基线，参考 execution branch 的 `MC-082` 至 `MC-084` 平台证据。Owner 本人合并本设计 PR 后，才启动 R2-06 施工；合并前不修改生产代码、构建链、R2-05 现场或 PR #34。

本稿是 [v2 计划](PLAN.md)的**增补包**，不建立新的计划版本。R2-06 依赖 R2-04 的真实随包路径；R2-05 已形成的 WIP 保留，但其正式安装包实机收口须等 R2-06 候选。现有 `PLAN.json`、`MANIFEST.json` 与已合并的授权 receipt 字节不变，避免使 R2-01 至 R2-05 的当前授权失效。R2-06 的单独施工边界来自 Owner 本轮明确指令及其本人合并后的本文件精确字节；不把设计 PR 或现有 receipt 冒充完成验收。V01–V08 的旧状态也不因增补自动改写。

## 1. 起因、目标与边界

Owner Windows 11 的 Smart App Control 强制模式对 `041a4966` 安装程序和便携 `gogoke.exe` 分别产生 3033/3077，二者均在启动前被拦；`985e17d1` 对应安装程序曾成功安装，其便携 `gogoke.exe` 在同机同类启动流程中也运行成功、未见 3033/3077。四份对象的哈希和事件在 `MC-078`、`MC-082`、`MC-083`、`MC-084`。这只是不同字节、目录和时间下的观察，**不能推断 SAC 只看哈希、已建立放行名单，或旧字节未来必然放行**。[构建与发布治理](../../governance/gogoke-build-and-release.md)的云端原生构建、不关闭 SAC、不做代码签名、Owner 离线签清单及正式安装实测边界继续有效。

目标是让纯前端、服务脚本和其他非可执行资源的变更不改变 `gogoke.exe` 与 `gogoke-native-host.exe` 的字节，并让运行中的产品在验签后切换资源。任何 `.exe`/`.dll`、Node runtime、原生协议、编译期公钥/允许主机/协调器或 shell 安全逻辑变化，都属于**可执行文件更新**，不能标为资源更新。字节稳定降低因资源改动制造新可执行文件的次数，**不保证 SAC 放行稳定字节**。

## 2. 产品启动与资源信任链

当前 `tauri.conf.json` 的 `frontendDist=../dist` 经 `tauri::generate_context!()` 编入主程序；`lib.rs` 在 setup 前已有自动创建的主窗口。R2-06 将用户可见的 HTML/JS/CSS 移到安装资源代，生产 shell 不再嵌入 `../dist`。最多保留固定的原生错误提示；它不能承载产品前端或取得产品 IPC 权限。

启动顺序固定为：解析实际安装/便携资源根 → 读取 `SHA256SUMS.windows`、签名和 `resource-index.json` → 用**编译期公钥**验 Owner 对清单原始字节的签名 → 检查清单唯一的版本、发行种类和索引 SHA-256 → 从索引核对当前 shell、native-host、Node 与当前资源代的路径/长度/hash → 才注册只服务该已核资源代的本地 custom protocol 并创建主 WebView → 才准启动 T3 服务及 native-host。签名、索引、文件或路径任何一步不成立，就不加载前端、不建立产品任务入口，保留有界错误证据。公钥、允许主机及更新协调器仍是编译期常量，不从资源、配置、Context 或数据库导入。

具体 Tauri seam 是让生产 `main` 窗口在验证后才创建，使用 `WebviewUrl::CustomProtocol` 与 `Builder::register_uri_scheme_protocol` 服务资源；`menu.rs` 的 About 窗口也改由同一已验证资源代加载。协议只接受索引列明的规范相对路径，拒绝 `..`、绝对路径、大小写碰撞、额外文件与 reparse/symlink；读取同一份已打开并算过 hash 的字节供 WebView 使用，避免“先验路径、后读另一份”。锁定的 Tauri 2.10.3 上 Windows origin、IPC 能力和 CSP 必须由**真实云端安装后的 WebView**验证；现有宽 `assetProtocol` scope 与空 CSP 不能直接扩展到这个资源目录。[Tauri custom protocol API](https://docs.rs/tauri/latest/tauri/struct.Builder.html)、[WebviewUrl](https://docs.rs/tauri/latest/tauri/enum.WebviewUrl.html)。

`resource-index.json` 由云端机械生成，记录发行版本、source commit、资源代 ID、每个文件的规范相对路径/长度/SHA-256，以及 shell、NSIS 安装后 shell、native-host 和 Node 的哈希。它自身的哈希进入 Owner 签署的 `SHA256SUMS.windows`。资源入口使用同一索引，不允许第二套 DB/配置事实或可写“已验”标记替代实际签名和字节。

## 3. 一次离线签署与安装包的哈希顺序

`SHA256SUMS.windows` 继续是 Owner 离线签署的发布集合；增加严格的 `full` / `resources` 发行种类、版本、索引和资源包条目。`full` 必须恰有一个精确安装程序、便携 ZIP、资源包和索引；`resources` 必须恰有资源包和索引、**不得有安装程序**。解析拒绝重复字段、重复名称、缺项、混合种类和版本不一致。资源索引把签名传递到每个安装后的文件；清单仍直接绑定发行资产哈希。现有 `gogoke_update.rs`、`tools/sign-gogoke-release-manifest.ps1` 和 `tools/publish-gogoke-release.ps1` 在施工时按这两种精确形状调整；私钥不进入 CI、仓库、构建树或产物，发布仍需 Owner 显式动作。

构建/签名顺序必须避免自引用：云端先完成资源包、两个可执行文件与 NSIS 安装程序；从 NSIS 实际承载的 shell 字节，或经精确 token 数和内容校验的 Tauri `UNK→NSS` 变换，计算**预期安装后** shell 哈希，生成索引与最终 `SHA256SUMS.windows`；Owner 离线签这一份最终清单。清单、签名、索引作为**安装程序旁的独立 sidecar** 发布，不编入它所校验的安装程序。NSIS 从同目录复制三者及初始资源到安装根内同一个版本化资源代；缺项时在改动旧安装前拒绝。产品在任何前端资源加载前验签并核字节。更新缓存同样将三者与安装程序并列暂存。首装应按“安装程序 + 三个 sidecar”作为一个可审阅下载集合交付；单独的 `setup.exe` 不再是有效的 R2-06 首装输入。手动启动未签名安装程序前，受信的清单验证步骤先核 Owner 签名及安装程序 hash；程序启动后的自检不能倒过来证明安装程序原本可信。签名后云端安装烟测必须再量**实际安装** shell 字节并与索引相等，否则候选失败。签名前的构建/打包测试或另用测试公钥的诊断包都不冒充这份生产字节的启动 PASS；生产入口不加验签绕过。这样保留一份 Owner 签名，也没有“安装包包含校验自身的最终清单”这一哈希循环。

现有版本的 `gogoke_update.rs` 只暂存安装程序，不能自动向新 NSIS 提供 sidecar。**首个 R2-06 架构版本须由 Owner 使用上述完整集合受控引导安装**；不得把旧 updater 的 installer-only 路径说成已兼容。新版本进入后，后续完整可执行文件更新才能由修改后的既有更新链暂存 sidecar、走协调器/正式安装程序与回滚。云端得到 Owner 的公开签名后还须对**冻结产物**运行一次签名安装/前端加载烟测；这一步不重新编译、不自动发布。

## 4. 稳定且可复现的可执行文件

生产发布版本、source commit、CI run/build 时间、前端 hash 和 release notes 只从已验证的索引/清单读取，不进入 PE 资源、Rust 常量、Tauri 编译上下文或 `GITHUB_SHA`。当前 `gogoke_update.rs` 的 `CARGO_PKG_VERSION` 比较与 readiness、`product_entry.rs` 的 `option_env!("GITHUB_SHA")` 草稿 provenance，须改取已验发行身份；不能把旧执行来源改标为新候选。Windows 文件版本若工具链要求固定字段，只标 shell ABI/固定值，不代表产品发行版本；安装注册表 `DisplayVersion` 与 UI 使用签名版本。

编译期保留稳定的安全常量：Owner 公钥、允许主机、协调器逻辑、NativeBinding/SQLite 固定实现及 native-host 对当前 Node runtime 的 digest 绑定。Node、SQLite 绑定或协议改变时 host 字节变化，必须走完整可执行文件更新；不能为了让哈希相等而删掉这些校验。Windows 图标、Tauri capability、Rust/Tauri/链接器变化若进入 PE，也按 shell 更新处理。

云端从**同一 source SHA**在两个独立、干净的 Windows 构建工作区，用锁定依赖、工具链、目标与配置各构建一次；逐字节比较原始 `gogoke.exe`、NSIS token 处理后的安装版 `gogoke.exe` 和 `gogoke-native-host.exe`，任一不同即失败并保留首个差异证据。当前 Tauri bundler 将主程序的 `__TAURI_BUNDLE_TYPE` 从 portable 的 `UNK` 确定性改为 NSIS 的 `NSS`（`MC-077`）；不能拿 portable 哈希代替安装后哈希。资源/版本仅变化的云端场景还要证明重新产出的三个可执行文件哈希与上一冻结 shell 相同；真正 shell/host 变化则明确产生新身份。任何消除时间戳、路径或 linker 非确定性的调整都由这个字节测试驱动，不能用“理论可复现”代替实际比较。原生构建和测试只在云端运行；所有新增 job 保留 timeout 和同分支取消旧 run。

## 5. 由 `gogoke.exe --uninstall` 卸载

`--uninstall` / `--uninstall --quiet` 在 Tauri/WebView 初始化之前分流，只允许从已注册的正式安装 `gogoke.exe` 执行；便携副本、未知注册根或已改指向的路径拒绝。先核当前 exe 与 HKCU 安装登记的规范根一致、安装清单/拥有文件集合有效、所有待删路径和祖先无 reparse、用户数据根不在任何待删子树，再停止受管进程。产品数据、Project、Context、settings 和用户自行加入安装目录的文件不因卸载被递归删除。

主程序不能删除仍映射的自身；它完成验证后只启动**固定的系统工具**执行受限收尾，等待主进程实际退出，再删除列明的产品文件、属于本产品的快捷方式与 HKCU 卸载登记，写有界结果回执。收尾的删除集合来自已验证的固定规则和安装文件清单，不执行可更新资源中的任意脚本；失败保留现场，不能以等待超时代替“文件已删除”的检查。此形状借鉴 [Sandglass `uninstall.py` 的安装根/manifest/reparse 与状态目录检查](https://github.com/taiyun668/Sandglass/blob/bf043c8afbf96540e72cfa3ef2c072bb03d2c200/sandglass/uninstall.py#L124-L250)及其父进程退出后收尾；不直接复制 Python 实现。

NSIS 使用锁定 Tauri 支持的[自定义 installer template](https://v2.tauri.app/distribute/windows-installer/#installer-template)：不生成、安装或登记 `uninstall.exe`；HKCU `UninstallString` / `QuietUninstallString` 改为 `"<install-root>\\gogoke.exe" --uninstall [--quiet]`，参照 [Sandglass NSIS 登记](https://github.com/taiyun668/Sandglass/blob/bf043c8afbf96540e72cfa3ef2c072bb03d2c200/packaging/sandglass.nsi#L1092-L1134)。旧版本已有的 `uninstall.exe` 只在迁移成功后作为旧安装文件处理；不得在失败回滚时删除旧安装或让新登记指向不存在的程序。云端正式安装/卸载烟测必须证明 `uninstall.exe` 不存在、登记与删除范围正确、用户数据逐字节保留。

## 6. 更新：可执行文件路径与资源路径分开

| 已签发行类型 | 判定与执行 | 成功边界 / 失败处理 |
| --- | --- | --- |
| `full` | 任一可执行文件、Node、原生绑定、编译期信任根或 shell 逻辑变化；沿既有 GitHub 固定源、签名清单、安装程序与 update coordinator 路径。协调器额外携带 sidecar，保留安装目录备份、安装/注册表恢复。 | 新正式安装的 Gogoke 验签、载入资源、启动所需子进程并写目标版本/资源代 readiness 才成功；安装程序、新 exe 或子进程被 SAC 拦、就绪错误/超时都回滚并记录精确组件/hash/事件。 |
| `resources` | 清单和索引明确证明当前 `gogoke.exe`、native-host、Node 及所有其他可执行字节**逐一相同**，资源包只含允许的非可执行文件。运行中的产品下载、验签、解包到新版本化资源代并切换；不下载、不启动新的安装程序或可执行文件。 | 既有 update 锁和状态机负责序列化；在无不可安全中断的任务时切换当前代，WebView 从新代重载、服务请求改用新代，观察同版本/资源代 readiness 后才退休旧代。签名/字节、任务安全点、启动或就绪失败则回到旧代；崩溃恢复也保留旧代。 |

资源代以不可变目录承载，`SHA256SUMS.windows`、签名、索引与资源文件随代存放，不原位覆盖当前 sidecar。正在运行的请求继续钉住旧代，真实 `beginCommitted` 后不因资源切换杀进程或盲重发；不能安全切换就延期，不能伪造 readiness。切换的是已验证代的原子指针，不对正在服务的文件原位覆盖。每次启动及每次从磁盘供给前端字节仍验签/验实际读取字节；旧代只在无句柄且新代就绪后清理。资源状态只是本地协调，不成为第二 accepted fact ledger。

`gogoke_update.rs` 当前只识别带 installer 的 release，`gogoke-update-coordinator.ps1` 也直接启动它；施工必须显式加 `resources` 分支，同时保留 `full` 的版本绑定、重核 GitHub release、受信 host、缓存 hash、用户确认、readiness 与 rollback。新发行版本须高于当前已验版本；只有受控失败回滚可恢复旧代。签名证明 Owner 授权字节，不单独证明它是最新版本。`tools/publish-gogoke-release.ps1` 当前要求安装程序与便携 ZIP；资源发行改为**不生成安装程序**的精确资产集合，仍由 Owner 离线签名及显式发布。资源更新不能绕过既有 release/Owner 控制，也不能自动采用 Dream 或 Goal Acceptance。

## 7. Smart App Control 与收口证据

首个新架构版本的 `gogoke.exe` 仍是新字节，正式安装程序、shell、native-host、Node 各受 SAC 独立判断；复现与资源签名不能向 Windows 提供 publisher reputation，也不保证一次放行后持续放行。Owner Win11 强制模式若拦截任何对象：停止该尝试，记录 4551（若实际观察到）、3033/3077 的**真实目标路径、时间、组件与 SHA-256**，保全旧安装/用户数据；不重试变通、关闭/绕过 SAC、引入自签/OV/EV 或本机原生编译。可安全回滚的更新回滚；不能证明恢复的轴记 `BLOCKED/NOT_RUN`，由 Owner 决定后续。

R2-06 的必要证据为：精确 SHA 的双云端可执行文件逐字节比较；只改资源/版本时 shell 与 host 哈希保持；签名资源启动及篡改拒绝；云端正式安装和 `--uninstall`/数据保留；资源更新无安装程序进程、失败回滚与崩溃恢复；Owner Win11 的**正式安装包**实际产品入口、所需每个子进程与资源更新观察；独立复核。每项记录实际 run、进程、文件 hash、非零测试数、fail/skip。云端 PASS、便携运行、单个旧版本放行或签名校验都不代替 Owner 的正式安装实测。R2-05 仍待它自己的 Goal/结果链、PR/接受事实回读和最终审查；PR #34 在本设计 PR 中保持打开、不合并。
