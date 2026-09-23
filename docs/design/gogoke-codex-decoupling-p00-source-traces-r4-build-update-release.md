# gogoke P00 接手摸排 R4：构建、更新、安装、签名与发布真实链

**日期：2026-09-18｜源码基线：`e9d67246d92b50c6d897377df39275dece81ae92`｜证据层级：SOURCE_BRANCHES_EXPANDED**

本批承接 R1 的 `RF06`、`RF07`。它把“检查、构建、准备、下载、签名、安装、回滚、上传、发布”分开记录，避免根据脚本名称、`--skip-*` 或“未发布”错误推定无副作用。

- `P00 acceptance = NOT_ACCEPTED`
- `global orphan_sink_count = null / NOT_COMPUTED`
- 未执行 npm/cargo/tauri、安装器、签名、注册表、GitHub release、ASC/TestFlight或真实更新。
- 本文只证明当前源码分支存在及其静态顺序，不证明构建、签名、安装、更新或回滚成功。

## 1. 后续施工必须接受的事实

1. **更新检查本身是下载和写入入口。** 生产启动可自动调用 `gogoke_update_check`；发现新版本时会下载完整 installer、写cache和prepared state。
2. **更新安装是跨进程事务，但目前没有统一产品Execution。** app写applying state、spawn PowerShell coordinator并立即exit；后续结果依赖文件receipt/failure log。
3. **“不发布”仍可能签名和写文件。** Windows publish helper在没有`-Publish`时仍校验、读取私钥、写signature。
4. **构建命令带 lifecycle writers。** `npm install/ci/dev/build`可触发postinstall/pre* icon同步；Windows build还调用Tauri icon生成。
5. **手工移动/iOS脚本是真实执行入口。** 它们会删build目录、复制图标、安装/启动app、上传ASC、修改TestFlight元数据；不能因不在package.json scripts里就从tracked工具闭包移除。

## 2. npm、Tauri配置与代码生成入口

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R4-001 | `package.json postinstall/predev/prebuild/pretauri:*` | 多个npm入口都会执行 `sync:material-icons`。脚本先递归删除 `public/assets/material-icons`，再从node_modules复制；source不存在时仅warn并exit0。`npm ci`不是只装依赖。 |
| R4-002 | Windows prebuild | `pretauri:build:win*`除同步material icons外还调用 `tauri icon ... --output src-tauri/icons/generated`；这是外部Tauri CLI写生成图标目录。 |
| R4-003 | `build` | `tsc && vite build`写dist；Tauri的beforeBuildCommand会再次走npm build。不同入口组合是否重复运行prebuild由npm/Tauri调用方式决定，不能按名称去重。 |
| R4-004 | doctor scripts | Unix `doctor.sh`只查cmake；Windows `doctor.mjs`查cmake+clang并打印安装建议，不执行安装。strict决定退出码。 |
| R4-005 | product identity check | 递归读文本、检查旧身份和merged Tauri config；无生产写入。它证明品牌/版本/配置条件，不证明Codex解耦或runtime行为。 |
| R4-006 | codemod scripts | 默认非dry-run会重写固定源码文件；missing path设置exitCode=1，但已处理文件不会回滚。`codemod:ds:dry`只读，`codemod:ds`是批量writer。 |
| R4-007 | base Tauri config | build前运行npm dev/build；bundle active all、asset protocol scope `**/*`、CSP null、devtools true。Windows config采用JSON Merge Patch替换window数组；最终构建配置需按实际命令合并核验。 |
| R4-008 | unsigned Windows config | 只把bundle target定为NSIS且不开Tauri updater artifacts；不代表生成物“没有安装/更新能力”，因为项目有自研update链。 |
| R4-009 | capability config | desktop/mobile均允许opener、dialog、notification；desktop还允许window effects/drag/close/zoom。它是plugin permission面，不等于每个调用都安全或已运行。 |
| R4-010 | Rust build.rs | 总是运行`tauri_build::build()`；iOS额外链接z/iconv。实际tauri-build生成/环境行为是固定依赖边界，不能从三行wrapper声称无副作用。 |

## 3. 生产自动更新：check、prepare、install和ready

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R4-011 | `useUpdater` mount | release Tauri环境、enabled且尚未尝试时，先`takeGogokeUpdateFailure`，再按setting自动check。failure读取会旋转日志并尝试更新state，不是只读。 |
| R4-012 | updater menu event | native `updater-check`事件还可手工触发`checkForUpdates(announceNoUpdate=true)`；与mount自动入口分开。 |
| R4-013 | `gogoke_update_check`平台 | 非Windows返回None；Windows使用全局mutex。发现有效新release前会请求GitHub feed、manifest和signature。 |
| R4-014 | update trust boundary | URL仅允许4个HTTPS host，redirect最多5次且每跳再检查；release必须非draft/prerelease、SemVer更高、三个唯一asset齐全、manifest signature与embedded public key通过。 |
| R4-015 | check是writer | 若已有prepared state与installer digest完全匹配则复用；否则下载最多250MiB installer，写`.part`、删除旧installer、rename发布，再写pretty JSON prepared state。任何阶段失败可留下旧state、partial或cache，需故障测试。 |
| R4-016 | state replace语义 | `write_update_state`先写part；若final存在先remove再rename。remove成功、rename失败会丢final state；这不是原子replace在所有平台上的等价保证。 |
| R4-017 | frontend check状态 | check开始把UI置checking；backend已经下载完才返回available。用户看到“检查更新”期间可能发生大量网络/磁盘I/O。 |
| R4-018 | `install` revalidation | 安装前重新取当前release identity并重新核staged installer digest；prepared版本过期或release变化会拒绝。 |
| R4-019 | install staging | 写embedded coordinator到cache；解析current exe和target；创建temp ready路径、failure/lock路径；写state=`applying`。 |
| R4-020 | coordinator spawn | 使用SystemRoot（缺失则C:\Windows）下PowerShell，ExecutionPolicy Bypass，hidden窗口。spawn成功后app立即`exit(0)`，没有等待coordinator接管或installer启动确认。 |
| R4-021 | spawn失败 | 尝试把state改failed但忽略该写失败，然后返回error；此时coordinator文件和prepared installer仍留cache。 |
| R4-022 | startup ready signal | 每次MainApp native启动都调用signal-ready；没有ready arg时no-op。有arg时严格限制temp目录/文件名，create-new part、sync、rename为final，然后best-effort把state改installed。receipt成功而state失败仍返回成功。 |
| R4-023 | failure consumption | 读取最多8KiB failure，若reported已存在先删除，再rename current为reported；随后best-effort将state改failed。rotate失败会让UI拿不到failure。 |
| R4-024 | post-update frontend state | install点击前先在localStorage保存pending version；新版本启动时若版本匹配会另外fetch GitHub release notes。dismiss清localStorage。这个frontend fetch独立于Rust signed update feed。 |

## 4. PowerShell coordinator和回滚

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R4-025 | parameter/lock | 要求installer/current/target/ready/lock/failure均绝对路径；取得FileShare.None lock。只锁该路径，不证明没有另一个不同root coordinator。 |
| R4-026 | wait parent | 按PID等待30秒；没有核创建时间/executable identity。PID已复用或ParentPid指错仍是后续风险轴。 |
| R4-027 | installer verification | parent退出后重新hash installer；防授权后文件变更。 |
| R4-028 | target ownership | 拒reparse target和已有backup；现有target只有含`gogoke.exe`才视为owned。然后把整个target move到backup。 |
| R4-029 | registry backup | query/export HKCU uninstall key；reg.exe调用有独立退出码处理。export成功后磁盘已有registry backup。 |
| R4-030 | install/launch | Start-Process installer（可/S）并wait；随后启动新exe带ready arg。installer成功不等于app ready。 |
| R4-031 | readiness | 最长30秒轮询receipt和new process。版本匹配后删除receipt、backup、registry backup和failure并exit0。cleanup任一步throw会进入catch并可能回滚一个其实已ready的版本。 |
| R4-032 | catch stop | 若new process仍活，force stop并wait5秒；结果未检查。 |
| R4-033 | rollback无旧target | 安装已经开始但未move旧target时，如果有uninstaller就运行/S，忽略其ExitCode；之后递归删target。 |
| R4-034 | rollback有旧target | 删除新target、move backup回来；恢复/删除registry registration。任一错误会重写failure为“rollback failed”。 |
| R4-035 | old app restart | 只要标记oldApplicationStopped、未NoRestart且CurrentExe存在，就Start-Process旧exe；未核old exe digest/identity或启动成功。 |
| R4-036 | lock cleanup | finally dispose lock，但lock file本身保留；下一次可重新打开。stale file不等于active lock。 |

## 5. CI、产物、签名与Windows发布工具

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R4-037 | workflow trigger | push只监听`codex/gogoke-shell`且限定app/tools/workflow paths；PR也受path filter。当前P00 evidence PR只改docs，**不会由该workflow自动验证**。 |
| R4-038 | web job | Windows npm ci（会postinstall同步icons）、identity check、trust-boundary check、typecheck、tests和build。不是Linux/macOS runtime证明。 |
| R4-039 | unsigned job | restore cache后主动删旧release exe/bundle；安装cmake/llvm；npm ci；生成icons；跑focused Rust update test；构建NSIS。 |
| R4-040 | local-only artifact | 复制unsigned exe+license/notices，要求Authenticode NotSigned，写单独hash清单并upload。它不是完整installer smoke候选。 |
| R4-041 | release artifact staging | 复制installer、portable exe/notice，验证unsigned，zip并写checksums；随后实际运行full update smoke，再upload smoke-verified unsigned artifacts。 |
| R4-042 | smoke update | 创建temp target，实际启动短命parent和coordinator，安装并启动gogoke；再注入错误版本验证rollback；finally可能启动uninstaller并递归清temp。不是纯单元测试。 |
| R4-043 | smoke process matching | cleanup按`gogoke.exe`+ExecutablePath停止；不含创建时间。仅用于临时target，仍应记录真实process mutation。 |
| R4-044 | sign manifest `-NewKey` | 在repo外默认路径创建private D/X/Y，并把public XY写repo文件；拒覆盖既有private key。是密钥/源码writer，必须显式授权。 |
| R4-045 | sign existing manifest | 读取private key和manifest，必要时向manifest插version行，再写`.sig`。不发布也会写。 |
| R4-046 | publish helper无`-Publish` | 校验artifacts，解压portable到temp并删除，调用sign helper、验证signature；最后输出PASS。仍读取私钥并写manifest/signature。 |
| R4-047 | publish helper带`-Publish` | 额外检查release不存在，再执行`gh release create`上传installer/portable/manifest/signature。发布没有自动rollback。 |

## 6. macOS/iOS手工构建与分发入口

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R4-048 | simulator helper | 默认同步iOS icons、删除`gen/apple/build`、运行Tauri iOS build；随后启动Simulator、boot/install/terminate/launch app。`--skip-build`仍安装和启动。 |
| R4-049 | physical device helper | 可读取local config和signing team；build后使用devicectl安装并terminate-existing启动。`--open-xcode`会打开工程；`--skip-build`仍安装/启动。 |
| R4-050 | TestFlight helper auth | source可选gitignored env文件，运行`asc auth status --validate`，按bundle解析app。脚本会读取联系信息等敏感元数据但不应进入日志/通用迁移。 |
| R4-051 | TestFlight `--skip-build` | 仍上传给定IPA、查询/修改export compliance、group、localization和review contact；skip-build不等于dry-run。 |
| R4-052 | TestFlight `--skip-submit` | 上传、group/metadata更新已经完成后才跳过external review submit。它不是不发布/不修改ASC。 |
| R4-053 | full TestFlight | 可创建encryption declaration/group/localization，assign build，update review contact并confirm submit。多个 `|| true` 分支允许局部错误被忽略，远端状态可能部分完成。 |
| R4-054 | macOS OpenSSL/sign script | 复制daemon/daemonctl进app、复制dylib、改install names/rpath，并对dylib、三个binary和app bundle重新codesign。缺daemon只warn，可能产出不完整但已改写app。 |
| R4-055 | iOS config | tracked config含具体developmentTeam；local override可存在且gitignored。最终签名身份来自多层配置/env，需在release证据中记录实际合并值。 |

## 7. 当前闭合状态

- `RF06`：主要tracked scripts/config/workflow入口已展开；仍需其他workflow、Vite/package-lock lifecycle、Tauri external build/plugin实现与generated Apple project逐项边界确认。
- `RF07`：Rust updater、coordinator、smoke/readiness/rollback核心源码已展开；仍需NSIS实际生成脚本/installer行为、真实Windows运行和registry/process identity测试。
- `CF01–CF05`：保持OPEN。
- 当前PR没有自动运行desktop CI；不能把docs-only GitHub状态当验证通过。

## 8. 已读源码索引

固定提交均为 `e9d67246d92b50c6d897377df39275dece81ae92`：

- `apps/desktop/package.json`
- `apps/desktop/scripts/{sync-material-icons,doctor,check-product-identity}.mjs`, `doctor.sh`
- `apps/desktop/scripts/codemods/{modal-shell-codemod,utils}.mjs`
- `apps/desktop/scripts/{build_run_ios,build_run_ios_device,release_testflight_ios,macos-fix-openssl}.sh`
- `apps/desktop/src/features/update/hooks/useUpdater.ts`, `utils/postUpdateRelease.ts`
- `apps/desktop/src-tauri/{tauri.conf,tauri.windows.conf,tauri.gogoke.unsigned.conf,tauri.ios.conf}.json`
- `apps/desktop/src-tauri/capabilities/default.json`, `build.rs`
- `apps/desktop/src-tauri/src/gogoke_update.rs`
- `apps/desktop/src-tauri/update/gogoke-update-coordinator.ps1`
- `.github/workflows/gogoke-desktop.yml`
- `tools/{smoke-gogoke-update,publish-gogoke-release,sign-gogoke-release-manifest}.ps1`

后续不得把这些入口压缩成单一“发布链”；每种skip/prepare/check模式的实际副作用不同。
