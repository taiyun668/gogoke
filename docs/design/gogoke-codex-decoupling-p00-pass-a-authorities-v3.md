# P00 Pass A：入口、反向副作用与六类权威表

Source basis = `bc665a852833952b76d9508401193bedd2198436`（B）。所有仓库路径均固定在 B；`src/`、`src-tauri/` 默认为 `apps/desktop/` 下。R1–R6 原始观察固定在 `b88988fd9621b048e883336d79ab1bb433a4cefa`（H）。本表记录源码事实和施工约束，不是已实现修复。逐观察场景和阶段见六份 `gogoke-codex-decoupling-p00-pass-a-traceability-r1-v3.tsv` 至 `...-r6-v3.tsv`；逐方法见 `gogoke-codex-decoupling-p00-pass-a-surfaces-v3.tsv`。全部后续 T 是未执行要求。纠正优先级见 `gogoke-codex-decoupling-p00-pass-a-corrigenda-v3.md`。

**Pass C R2 生效补充：** B01–B03 来源为本轮 CHANGES_REQUIRED；§10 是 F20/P17/P19 选择性复用的必读展开，不能用旧总括行替代。新增源码锚点、选择矩阵及逐写入点见 `p00-pass-c-b01-b03-r2/reuse-contracts.json` 和 `source-identities.json`。原版保持在 `620b49270f99f18cd9d8d5e7ffa68b76c5f274df`。

## 1. 正向入口与反向 caller 的共同索引

家族不是函数数量。启动钩子、注册 command、CLI、导出 API 和真实 UI 回调都可以成为根，不人为补不存在的按钮。动态回调结合实际 assembly/adapter 注入，不只按同名推边。原观察中的事实、失败分支和证据按 ID 继承，再应用纠正表。

| 家族 | 固定入口与已核路线 | sink/caller、guard 与失败语义 | 处置、主责与首验 |
|---|---|---|---|
| F01 HTML/React | index.html → src/main.tsx → App.tsx → MainApp.tsx → bootstrap/useAppBootstrapOrchestration.ts → useAppBootstrap.ts；About lazy import、useWindowLabel | S01/S09/S11/S13；StrictMode、初始main label、mount readiness、settings/dictation hooks不等于单个主窗口完全就绪 | 保留壳；WP05/T12.L；错误窗口启动effects单验 |
| F02 native lifecycle | src-tauri/src/main.rs → lib.rs::run/setup/app.run；single-instance、Linux env、macOS hide/reopen、Windows chrome、iOS inset、menu/tray | S04/S05/S11；启动、关闭视图、退出宿主是不同根；show/focus/stop/emit多处忽略错误；fix-path-env单列 | 内化生命周期；WP03/T50.L、T52.L；WP05/T12.L |
| F03 Workspace UI/自动恢复 | MainApp → useWorkspaceController/useMainAppWorkspaceLifecycle → useWorkspaceRestore/useWorkspaceRefreshOnFocus/useWorkspaces/useWorkspaceCrud；useMainAppModals → AppModals → WorktreePrompt/ClonePrompt/WorkspaceFromUrlPrompt/NewAgentPopover/MobileRemoteWorkspacePrompt | S01/S03/S04；restore/focus/poll/submit/create均为根；dialog转发注入callback，失败/加载/连接状态分开 | 目录可用不绑原生连接；WP08/T03.L、T34.L |
| F04 Workspace core | workspaces/commands.rs → shared/workspaces_core/{connect,crud_persistence,worktree,runtime_codex_args,helpers,io,git_orchestration}.rs；daemon workspace handler → DaemonState同core不同实参 | S01/S03/S04/S05；本地finder=None、daemon=Some；共享Arc、全部key换进程、部分回滚/补偿分别记录 | 保留能力、替换身份/事务接线；WP03/T51.L、T63.L；WP08/T34.L |
| F05 Git/GitHub | useMainAppGitState/useGitPanelController/useGitCommitController/useGitActions、status/log/diff/PR hooks、GitDiffPanel；service → git/mod.rs宏 → shared/git_ui_core/{commands,github,diff,log,context}.rs、git_core.rs/git_utils.rs | S03/S04/S07；libgit2和git/gh均覆盖；poll/debounce/menu/branch/apply/commit+push分开；rename双路径与复合命令部分成功 | 原位保留；WP08/T03.L、T22.L、T34.L；外部hooks/env WP03/T50.L |
| F06 PTY/scripts | useTerminalTabs/useTerminalController/useTerminalSession；xterm onData、ResizeObserver、visible effect；单/多launch与useWorktreeSetupScript → terminal.rs/mobile stub | S04/S05/S08/S13；hide/dispose不close PTY；EOF不是child exit；stdin和mark-ran非成功收据 | 公共PTY、显式脚本意图；WP08/T33.L、T34.L；WP03/T52.L |
| F07 native session/control/generation | codex/mod.rs → shared/codex_core.rs/codex_aux_core.rs → backend/app_server.rs；daemon codex/git handler；composer/thread/review/approval hooks → service | S04/S06/S08；query/send/steer/interrupt/fork/compact/review/三个隐藏生成器分别处理；request300s、initialize15s；background collect60s后archive不是stop | 原生差异归adapter；WP04/T13–T20.L；WP06/T28–T29.L；WP08/T30.L |
| F08 Event fanout | stdout/stderr、event_sink.rs、daemon broadcast/transport、services/events.ts → useAppServerEvents/useThread*Events/useThreads、account/live/tray/通知消费者 | S08/S10/S13；坏帧、无ID、raw debug、hidden thread、重复hub、late unlisten、archive cascade、unread分别有事实来源；无耐久投递保证 | WP04/T16.L、T18.L；WP05/T66.L；WP06/T65.L |
| F09 settings/account/models | useAppSettingsController/useAccountSwitching/useHomeAccount/useModels、Settings sections → settings/mod.rs、codex/mod.rs、local_usage.rs；shared/settings_core.rs/account.rs/codex_aux_core.rs | S01/S04/S06/S10/S11；桌面settings3和doctor始终本机；load归一化可重写settings；account读取可用auth.json元数据回退，不验JWT签名 | 公共和native配置分开；WP02/T39.L；WP09a/T06.L；WP09b/T23.L、T36.L |
| F10 resources | useCustomPrompts/useMainAppPromptActions/SettingsAgents → prompts.rs/files/mod.rs/codex/mod.rs → prompts_core/agents_config_core/config_toml_core/rules.rs；skills/MCP/apps → native RPC | S02/S06/S10；prompt list/dir mkdir；桌面prompt7和remember-rule本机执行；Agent文件/总配置不同步；发现不是启用权限 | 公共资源内化、保留原生来源；WP07/T25–T27.L；WP04/T19.L、T24.L |
| F11 Files/export/open/image | services/tauri.ts picker/export、Files UI、MainHeader/OpenAppMenu/useFileLinkOpener → files/{mod,policy,ops,io}.rs、workspace io/macos、image core | S02/S04/S11；file_read/write可远程，export/image/open/icon有本机路径；dialog不提供backend持久权限；symlink/大小/temp/argv独立 | 保留离线能力；WP08/T22.L、T35.L；WP03/T50.L |
| F12 notification/tray/clipboard/audio | completion/sound/response-required hooks、useSystemNotificationThreadLinks/useTrayRecentThreads/useTraySessionUsage/useCopyThread/useDebugLog → wrapper/fallback/tray.rs/clipboard/notificationSounds | S08/S10/S11/S12；duration/focus/dedup/permission/filter不同；resolve/emit/play不是显示或已读；精确调用族见§7 | 公共目标与隐私内化；WP05/T12.L、T66.L；WP09b/T23.L |
| F13 menu/shortcut/direct API | useSidebarMenus/useMainAppSidebarMenuOrchestration；Git/Files/Prompts/file-link menus；composer/interrupt/archive/branch shortcuts；MainHeader/window/zoom/material hooks | S04/S06/S11/S13；popup action调用真实callback；daemon accelerator no-op；ask/message不是server授权；IME/焦点保留 | WP05/T07.L、T12.L；WP04/T19.L；WP08/T34.L |
| F14 transport/RPC | adapter call_remote → remote_backend/{mod,tcp_transport,transport,protocol} → daemon transport/auth → dispatcher daemon/workspace/codex/git/prompts五handler | S07/S08/S05；104 business+独立auth；36 retry、32 in-flight、512 client queue、2048 broadcast、server unbounded output；19个shared-but-local desktop入口见surfaces | 封外部remote，内化本地链；WP10a/T56.L；WP04/T14.L、T16.L、T53.L；WP10b/T43.L |
| F15 daemon/ctl/Tailscale | lib setup/settings/exit；tailscale/{mod,daemon_commands,rpc_client}；bin/gogoke_daemon.rs和gogoke_daemonctl.rs start/stop/status/preview | S01/S04/S05/S07/S10；loopback与0.0.0.0不同；token argv/root fallback/PID无ctime；list pruning是process sink；shutdown100ms不排空 | 保留本地daemon、封外部入口；WP10a/T56.L；WP02/T55.L；WP03/T52.L、T63.L |
| F16 mobile legacy | main.tsx viewport/gesture、useMobileServerSetup、remote workspace dialog、menu_mobile/terminal_mobile、dictation stub、iOS native layout | S01/S07/S11；自动连接测试先保存remote配置；未测平台不称通过 | 封连接入口、保留资产兼容说明；WP10a/T56.L；WP10b/T45.L |
| F17 voice | bootstrap/controller/model hooks、hold-to-dictate/blur、composer toggle/settings → dictation real/stub → reqwest/CPAL/Whisper | S02/S05/S07/S08/S09；exists状态不是hash；sample cap不stop；Processing cancel不终止blocking CPU；模型不删 | 封入口保资产；WP10a/T57.L；WP12/T37.L、T42.L |
| F18 product update | useUpdater/useUpdaterController/MainApp ready/native menu → gogoke_update.rs → include_str coordinator → installer/newexe/uninstaller/oldexe/reg.exe | S07/S14/S04/S05；check是writer；state remove-before-rename；签名/host/版本/ready分开；cleanup失败可触发rollback | 保留信任链；WP12/T38.L、T42.L、T46.L、T59.L |
| F19 build/scripts/CI | desktop package hooks、Vite/build.rs、base/platform/unsigned配置、capabilities、icon/doctor/codemod/iOS/macOS/release/sign/smoke脚本和3workflows | S04/S07/S14/S15；建议文字与执行分开；noPublish/skip不等于dry-run；general PR CI不同于desktop过滤；生成Apple/NSIS/依赖边界固定 | WP03/T60.L；WP01/T61.L；WP12/T46.L、T60.M |
| F20 repo packages/legacy | root package room:start → tools/start-room.mjs；pnpm packages/*；seat-runtime exports/dist/demo、room server、protocol adapter/runner/controller/codegen；root Cargo crates/*与desktop独立workspace | S04/S07/S15；root启动逐次install→build→server；并非desktop自动导入；Room调度/独立kernel不整体成为gogoke authority；未来选用须固定资产 | 非整体采用但保留源码；WP03/T60.L、T41.L；WP04/T67.L；不是外部UNKNOWN |
| F21 storage/cache/plugin | threadStorage/promptHistory/layout/sidebar/sort/transparency/update/mobile keys、in-memory buffers、window-state plugin | S01/S10/S13；22 key逐类迁移；输入/provider参数/UI偏好不能混为server authority；插件精确格式固定外部边界 | WP02/T39.L；WP05/T07.L、T66.L；WP12/T37.L |
| F22 content/attachment/browser | Markdown/urlTransform/file/thread callbacks；useComposerImages/useComposerImageDrop → native paths或FileReader data URLs；asset/SVG/xterm | S02/S10/S11/S12；格式/模型内容不授权；异步选取/粘贴绑定generation；普通mailto与代码块HTTP不同 | 保留渲染附件；WP05/T54.L；WP08/T35.L；WP04/T24.L、T49.L |

反向 S01–S15：S01 JSON/root writer；S02资源/文件/export；S03 Git/libgit2/gh；S04 process/env/stdin/PTY；S05 kill/exit/旧writer；S06 native RPC/control/provider；S07 TCP/HTTP/listener；S08 event/delivery；S09 voice/permission；S10 clipboard/log/privacy；S11 opener/dialog/menu/window/tray；S12 browser audio/assets/render；S13 storage/cache/temp/plugin；S14 install/sign/publish/registry；S15 build/dependency/codegen。每个S均反向落入上表的caller家族及下面权威行；不是全函数零孤立证明，global_orphan_sink_count=null。

## 2. Root 权威表

| ID | 解析/对象 | reader；writer/删除者 | fallback/当前锁 | 迁移、主责与首验 |
|---|---|---|---|---|
| RT01 | desktop app.path().app_data_dir() | AppState/load；storage/settings/workspace cores | 失败→current_dir→`.`；Mutex非跨进程OS锁 | 禁cwd回退、固定rootId/schema/真实锁；WP02/T55.L |
| RT02 | RT01/workspaces.json | desktop/daemon list；CRUD/worktree/settings/order/rename/remove | 缺失空表，损坏上层default；direct write | 保留ID/path/关系、损坏fail-closed；WP02/T39.L |
| RT03 | RT01/settings.json | app/daemon/ctl/startup；update与read_settings归一化反写 | read可能writer，直接write，无全局单写者 | UI/secret引用/provider/信任策略分层；WP02/T39.L；WP09b/T23.L |
| RT04 | RT01/workspaces/id/prompts | list/dir/CRUD；list和dir也mkdir | settings_path parent；无跨进程锁 | 公共根、冲突/重复导入保原件；WP07/T25.L |
| RT05 | RT01/worktree-setup/id.ran；worktrees按workspace/global目录配置优先 | status；mark-ran、worktree add/rename/remove、AGENTS copy | mkdir+timestamp，marker不是exit，无整体事务 | 不复制/误删原worktree，marker带来源；WP08/T34.L |
| RT06 | app_data/models/whisper/model及.partial | status/context；download/cancel/remove | app-data失败回cwd、exists-only Ready、state锁非file generation锁 | 封入口保path/hash/bytes；WP10a/T57.L；WP12/T37.L |
| RT07 | app_cache_dir()/update-cache | check/install/failure；installer/state/coordinator/failure/lock | cache错误传播，UPDATE_LOCK进程内，remove→rename | 不导入信任根，复核复用字节；WP12/T59.L、T38.L |
| RT08 | OS temp gogoke-update-*.ready与part | app ready writer、coordinator reader/deleter | temp绝对约束/create_new/sync/rename | 短期收据绑定版本/操作，不当耐久数据；WP12/T59.L |
| RT09 | current exe parent或LOCALAPPDATA/Programs/gogoke；update-backup | coordinator/installer/uninstaller/rollback | owned-target/reparse拒绝；路径FileShare.None锁 | 保升级后新增数据，旧exe不读新schema；WP12/T42.L、T46.L |
| RT10 | managed显式settings parent；standalone XDG或HOME/.local/share/gogoke_daemon；ctl平台identifier目录 | daemon/ctl load及workspace/settings/prompt writers | 缺HOME/USERPROFILE用`.`；无产品OS锁证明 | 统一实际root/owner；WP02/T55.L |
| RT11 | WorkspaceEntry.path | Files/PTY/native cwd/skills/Git；工具/worktree/AGENTS | 各链canonical/权限不等价 | 用户工作区不自动迁移复制/删除；WP08/T22.L；WP03/T50.L |
| RT12 | workspace/parent native home或CODEX_HOME/HOME/.codex | config/AGENTS/prompts/native/auth metadata | normalize可保留相对/未展开输入；非产品根 | adapter边界，公共编辑不暗写；WP02/T55.L；WP07/T26.L |
| RT13 | RT12/sessions/**/*.jsonl | usage/history兼容read；native CLI write | 原生格式/账号路径，非public DB | 授权导入、保来源/域/未知统计；WP09b/T36.L、T40.L；WP12/T37.L |
| RT14 | save/export上游路径 | dialog/service；write_text_file mkdir+write | 非空检查，无backend save-ticket/root限制 | 显式目标，不成任意模型写权；WP08/T22.L |
| RT15 | WebView localStorage22家族 | frontend getters/setters/removers | best effort、坏值/旧ID/origin，无服务端锁 | UI/隐私/native映射分开；WP05/T07.L、T66.L；WP12/T37.L |
| RT16 | window-state plugin内部root/format | pinned plugin reader/writer | 调用面无精确root，不能虚构 | EB01；WP05/T12.L，首次迁移前WP12/T37.L |
| RT17 | native install/credential位置含RT12/auth.json | native login/install write；account.rs读claims | CLI权威；回退不验签不证新登录 | 只迁引用不迁token；WP09b/T23.L；WP09a/T58.L |
| RT18 | Git remotes/GitHub/Tailscale系统身份/外部app | git/gh/tailscale/opener | 外部配置与身份无本地DB事务 | 不自动迁外部权威；WP08/T34.L；WP10a/T56.L |
| RT19 | temp gogoke-image-*.jpg与gogoke-icon-*.png | HEIF/icon→sips/defaults，host读取、尽力remove | 毫秒命名、忽略cleanup错误，非密封快照 | 不迁移；私有像素残留、env与清理；WP03/T50.L；WP08/T35.L |
| RT20 | bundled音频URL、AudioContext、OS output | fetch/decode/play；GC/系统设备 | 内存/外部设备，无耐久writer | 不迁移，单验可听/autoplay/隐私；WP05/T12.L |
| RT21 | HKCU uninstall/gogoke与failure.registry.reg | reg query/export/import/delete、backup清理 | registry/文件非事务，coordinator路径锁 | 导入不改信任；WP12/T46.L、T42.L |
| RT22 | workspace.settings.git_root相对workspace或绝对目录 | Git/libgit2/gh所有repo mutation | 绝对目录只需存在，不保证包含workspace | 保留外部repo选择但单独授权；WP08/T22.L |
| RT23 | RT12/config.toml、agents/*.toml、外部role引用 | Agent list/read、managed role/config CRUD | list可展示外部引用/存在性；read/write接口只接受managed role并拒绝symlink分量；write不预解析新内容TOML；文件/config非事务 | 公共模板替换、未知字段和外部原件保留；WP07/T26.L |
| RT24 | RT12/rules/default.rules与default.lock | remember-rule/native CLI；append+lock删除 | create_new锁文件、stale30s、wait2s；非OS进程身份锁；读失败空值 | 范围/argv/去重单验；WP04/T19.L、T24.L；WP07/T26.L |
| RT25 | repo public/icons/dist/target/gen/apple/release-artifacts及工具temp | npm/tauri/tsc/Vite/codemod/iOS/macOS/sign/publish/CI | rm/cp/build/upload非原子；secret非普通artifact | 固定source/lock/config/产物；WP03/T60.L；WP12/T60.M |

具体源码：state.rs/storage.rs；shared/{settings_core,prompts_core,agents_config_core,config_toml_core,codex_core,account,local_usage_core}.rs；files/{policy,ops,io}.rs；codex/home.rs；rules.rs；gogoke_update.rs；dictation/real.rs；workspace cores；daemon/ctl；R1–R6和原entry inventory§4/§6，均固定B/H。

## 3. Writer 权威表

| ID | 入口→目标 | 事务/部分成功/重试 | 处置、owner/check |
|---|---|---|---|
| W01 | F03/F04 CRUD→RT02及内存 | 内存/磁盘/session顺序不同，无全局事务，失败不盲重放 | WP02/T39.L；WP08/T34.L |
| W02 | F09 settings→native config5项→RT03 | native错误可忽略，前项已写后项失败 | 拆公共/native writer；WP02/T39.L；WP07/T26.L |
| W03 | F02/F09/F15 read_settings→RT03 | 归一化读诱发write，失败仅stderr | 禁坏数据回空覆盖；WP02/T39.L |
| W04 | F10 prompt list/dir→RT04/RT12 mkdir | 查询也是写，不在36项retry表，无全局锁 | 显式创建语义/公共根；WP07/T25.L |
| W05 | F10 prompt create/update/delete | exists+write竞态；update先write(next_path)再remove旧target，后步失败可留两份；delete无undo | 分步骤保原件与结果；WP07/T25.L |
| W06 | F10 prompt move→另一scope | 跨盘copy→remove，后者失败留两份 | 冲突/去重/来源；WP07/T25.L |
| W07 | F10 Agent CRUD→RT23及config | create先角色文件再config，失败尽力删新文件；delete先备份/删managed文件再config，失败恢复；外部文件不删 | 公共模板、未知字段保全；WP07/T26.L |
| W08 | F10 flags/remember→RT12/RT24 | direct write；读失败空；stale锁可与活writer竞态；无reload证明 | native配置不授公共权；WP04/T19.L；WP07/T26.L |
| W09 | F11 file_write→AGENTS/config | exists分支才查symlink；dangling缺同等检查；globalAGENTS允许外部；direct truncate | 保授权兼容、补路径边界；WP07/T25.L；WP08/T22.L |
| W10 | F11 export→RT14 | mkdir/directwrite，失败可有目录/部分内容 | 显示主机/用户目标；WP08/T22.L |
| W11 | F07/F09 input/login→RT13/RT17 | stdin与原生durability非事务，timeout接受不明 | 不复制token；WP04/T14.L；WP09b/T23.L |
| W12 | F04/F05 Git/gh→RT11/RT22/remote | index/worktree/refs/repo/branch多步，pull fallback、push/PATCH、libgit2部分成功 | 保Git；WP08/T34.L、T22.L |
| W13 | F04/F06 setup→RT05/AGENTS.temp/terminal | copy/rename残留；stdin后mark不等于exit | execution receipt和marker分开；WP08/T34.L |
| W14 | F06 PTY/launch→shell stdin/cwd | flush非完成，三种script的restart失败处理不同 | 分用户终端/模型工具；WP08/T33.L、T34.L |
| W15 | F17 model→RT06 | partial truncate/hash/flush/rename；abort非放锁；cached Arc可存活 | 封入口保模型；WP10a/T57.L |
| W16 | F18 check/failure/ready→RT07/RT08 | 下载writer、remove-before-rename、receipt/state独立失败 | WP12/T38.L、T59.L |
| W17 | F18 coordinator→RT09/RT21 | install/registry/ready/cleanup/rollback跨进程非原子 | 数据不得改信任；WP12/T42.L、T46.L |
| W18 | F19 lifecycle/build/sign/upload→RT25/remote | rm/cp；sign可先改manifest；noPublish也写；ASC多步 | 分模式/授权；WP03/T60.L；WP12/T59.L、T60.M |
| W19 | F08/F12/F21→localStorage/clipboard/debug/notices | 多窗口/迟到/隐私/best-effort，非server authority | WP05/T66.L；WP09b/T23.L；WP12/T37.L |
| W20 | F02/F11/F12/F13→OS/plugin/RT16/RT19/RT20 | popup/show/emit/play/convert只证调用，temp/plugin持久化另列 | WP05/T12.L、T54.L；WP03/T50.L；WP12/T37.L |

## 4. Process 权威表

继承env/cwd/home是风险事实，不是已隔离。process_core只隐藏Windows console，不清环境、不提供授权；PID或Arc不是完整进程身份。

| ID | program/env/home/cwd | caller/身份/共享键 | stop、失败及子孙 | owner/check |
|---|---|---|---|---|
| P01 | selected codex --version/app-server --help、node --version；PATH增强，继承host cwd/home | F09 doctor、F07 spawn前probe；未应用后续workspace cwd/CODEX_HOME | 5s timeout非子孙已停；probe是真执行 | WP03/T50.L、T32.L；WP09a/T06.L |
| P02 | selected codex app-server；workspace cwd，可设CODEX_HOME，余env继承；Windows cmd/bat | F04/F07 desktop+daemon多key同Arc；缺profile/auth/domain共享键 | initialize15s timeout显式kill，其他早退和协议error另核；EOF清pending不wait子孙；300s接受不明 | WP03/T51.L、T52.L、T63.L；WP04/T14.L |
| P03 | portable_pty shell；workspace cwd、TERM/LANG/LC | F06 visible/tab/launch/setup；workspace+terminal ID | EOF无exit status；close忽略kill；双open补偿失败 | WP08/T33.L；WP03/T52.L |
| P04 | git+git_env_path、RT22 cwd、hooks/credential helpers | F04/F05按repo操作，不等public Session | output/status/wait与复合步骤不同；无统一硬deadline/树停止合同 | WP08/T34.L；WP03/T50.L |
| P05 | git check-ignore/stdin、git apply --3way、reader线程 | F05 diff、workspace git orchestration | stdin失败kill+wait；apply可部分写；thread/child清理单验 | WP08/T34.L；WP03/T52.L |
| P06 | gh issue/pr/api/repo/release、repo cwd、继承凭据 | F05查询/checkout/create；F19发布 | query可联网；publish无整体rollback | WP08/T34.L；WP12/T59.L；WP09b/T23.L |
| P07 | 用户command/app；Win cmd /D /S /C、macCLI/open -a、其他direct | F11 open；target不强制在workspace；desktop本机 | 等output/截断日志非隔离，无隐式模型权 | WP03/T50.L；WP08/T22.L |
| P08 | /usr/bin/sips HEIF→temp JPG；继承env/图片path | F11 image、F07附件编码 | 读取和remove失败可残留私有像素 | WP08/T35.L；WP03/T50.L |
| P09 | defaults read→PATH sips icon→temp PNG | F11 desktop+daemon icon；两metadata key可重复defaults | temp名非内容身份，cleanup尽力 | WP03/T50.L、T52.L |
| P10 | selected daemon --listen --data-dir --token；stdio null、env继承 | F02/F09/F15；root/token/version/ownership preflight | spawn Running不是bind；kill需probe；外部PID无ctime | WP10a/T56.L；WP03/T52.L、T63.L |
| P11 | daemonctl/tailscale/launchctl asuser/lsof/ss/netstat/Unix signals | F15独立CLI；arg/env/settings token；start/stop/status/preview分开 | auth/name/PID guard无ctime；nonUnix external force-stop拒绝；status触网 | WP10a/T56.L；WP03/T50.L、T52.L |
| P12 | osascript title/body | F12 app/daemon各自macOS debug | 正式/其他平台拒绝；成功不等显示；隐私跨OS | WP05/T12.L；WP09b/T23.L |
| P13 | SystemRoot PowerShell→coordinator→installer/newexe/uninstaller/oldexe/reg | F18 prepare后app exit；path/version/hash/receipt/parentPID | parent wait无ctime；force-stop/rollback可失败；非全局单写者 | WP12/T46.L、T59.L；WP03/T52.L |
| P14 | npm/pnpm/tauri/tsc/Vite/git/xcrun/asc/codesign/PS发布 | F19/F20 package/CI/manual；cwd和lock固定，secret另授权 | 每步可留artifact/remote状态；noPublish/skip非无副作用 | WP03/T60.L；WP12/T60.M |
| P15 | fix_path_env::fix固定revision | native main prelude，影响后续PATH，错误仅stderr | 不猜内部shell；需库源和实际env | EB02；WP03/T50.L、T60.L |
| P16 | CPAL capture线程、Whisper spawn_blocking，非CLI子进程 | F17 state/context/cancel；缓存model ID无hash绑定 | cancel输出不等于停CPU；sample cap不stop麦克风 | WP10a/T57.L；恢复另授权 |
| P17 | room:start→pnpm install→seat build→room server；seat/protocol exports | F20独立旧执行产品/候选资产，非desktop已采用第二宿主 | 不整体导入Room scheduler/store；选用worker须固定模块/env/入口 | WP03/T60.L、T41.L；WP04/T67.L |
| P18 | brew list/upgrade、npm list/install -g、前后codex probe；继承PATH/home | F09 codex_update→shared/codex_update_core.rs；desktop-only，非doctor | timeout/非零可已有安装改变；前后版本不证可回滚 | WP09a/T58.L；WP12/T59.L；WP03/T50.L |
| P19 | Windows taskkill /PID /T /F后child.kill；Unix child.kill/管理signals | shared/process_core.rs，F04/F07/F15；taskkill受程序解析env影响 | status/kill结果可忽略；无ctime不得裸杀，返回不证树停 | WP03/T52.L、T63.L、T50.L |

## 5. Network 权威表

| ID | endpoint/认证 | entry/retry/timeout/gap | 边界、处置与首验 |
|---|---|---|---|
| N01 | provider由native binary/profile/env决定，原生Home认证 | F07/F09 stdin；probe5s/init15s/request300s/生成collect60s，无端到端幂等/费用收据 | 不触发登录/付费；WP09a/T06.L；WP04/T14.L；每家N1准入 |
| N02 | Git remotes/gh GitHub API，凭据独立于模型账号 | F05/F19复合操作、无统一deadline，外部hooks/helper | 受控repo先验；WP08/T34.L；WP09b/T23.L |
| N03 | settings host TCP、可选token/auth，无应用TLS证书验证 | F14 queue512/send15s/response300s，精确disconnect重试1次、36方法，client创建非完整锁保护 | WP10a/T56.L；WP04/T14.L、T53.L；接受不明不盲重放 |
| N04 | standalone127.0.0.1:4732或managed0.0.0.0:port，token/显式无auth | transport外置auth；32并发，unbounded out，broadcast2048 Lagged静默continue；EOF不取消business | WP10a/T56.L；WP10b/T43.L、T44.L；WP04/T16.L |
| N05 | Tailscale CLI/OS服务/tailnet身份 | F15 status/probe，maclaunchctl→direct fallback，DNS/host/IP敏感 | 不迁改系统身份；WP10a/T56.L；WP09b/T23.L |
| N06 | api.github.com/repos/taiyun668/gogoke/releases/latest，4HTTPS hosts、embedded公钥 | F18 redirect最多5逐跳检查，feed1MiB/installer250MiB；client无应用connect/total timeout；check写cache | WP12/T59.L、T46.L；发布库不是施工库 |
| N07 | 前端GitHub tag notes、html_url→opener | F18/F22 v版本404再无v；generation只抑UI、无AbortSignal；独立auto-check | WP05/T54.L；WP12/T59.L；非签名证明 |
| N08 | 固定HuggingFace URL/SHA256 | F17 connect10s/total30min，无源码maxbytes；切换/abort/partial | 封入口保资产；WP10a/T57.L；恢复另授权 |
| N09 | 包/Actions分发、GitHub release、ASC/TestFlight | F19/F20 locks/ref固定，失败可部分成功，noPublish本地签名 | WP03/T60.L；WP12/T60.M；不拿latest网页代替固定身份 |
| N10 | bundled audio/assets、Markdown/auth/About/Git外链、Vite dev/HMR | F12/F19/F22 browser/OS，audio失败debug；host/1420/HMR1421 | 不授任意scheme；WP05/T54.L、T12.L；WP09b/T23.L |

## 6. Permission/Secret 权威表

| ID | 来源/存储/传播 | 当前事实与风险 | owner/check |
|---|---|---|---|
| Q01 | remote token settings/profile→React/AppState→TCP auth/managed argv；ctl arg/env/settings | transport token非每操作/Session许可；argv/log/export暴露，未读真实值 | WP09b/T23.L；WP04/T19.L；WP10a/T56.L |
| Q02 | native auth.json/login/auth URL/JWT metadata回退 | payload不验签，错误可回退旧email/plan，非认证/额度真值 | WP09b/T23.L、T36.L；WP09a/T58.L |
| Q03 | Files/workspace/Git/open path/picker/config | canonical/globalAGENTS例外/dangling/absoluteGitroot/export不同；dialog非server capability | WP08/T22.L；WP07/T25.L |
| Q04 | native sandbox/readOnly/approval never、argv/env/hooks | 传选项不证明OS限制；隐藏生成器有provider执行/历史；正文不授权 | WP03/T21.L、T50.L；逐家N1/I |
| Q05 | approval/input/login continuation与raw requestId/result | 参数通过非正确用户/域/代次；reply与本机remember-rule写分开 | WP04/T19.L、T20.L、T24.L |
| Q06 | AVFoundation prompt、CPAL设备/权限 | command/start两个入口；nonmac true非设备许可；keyrelease/cancel竞态 | WP10a/T57.L；本轮不申请 |
| Q07 | notification permission/debug fallback、clipboard/audio | denied/exception尝试fallback；resolve非可见；隐私跨OS | WP05/T54.L、T12.L；WP09b/T23.L |
| Q08 | private release key/public trust root/allowlist/ASC env | noPublish读key；NewKey双写；skip仍远端变更；数据不改信任 | WP12/T59.L、T60.M；未读取秘密 |
| Q09 | parent env/PATH/HOME/CODEX_HOME/proxy/socket/凭据/toolchain | wrapper不清env、probe/spawn不同；显式profile验证继承 | WP03/T50.L；WP09a/T58.L |
| Q10 | Skills/MCP/Apps/Agent/AGENTS/rules文本 | 发现/展示/启用/执行/批准分开；native subagent非public席位；规则锁/去重不授权 | WP07/T27.L；WP04/T19.L、T24.L、T67.L |
| Q11 | transcript/debug/promptHistory/storage/tray/notice/export | 完整debug/prompt response/path/authURL/正文可能分享；私聊不默认下发 | WP09b/T23.L；WP04/T49.L；WP05/T66.L |
| Q12 | Markdown/thread/file链接、paste/drop、menu action | 文本/链接/弹窗不是可信授权来源，需重核域/目标/代次 | WP05/T54.L；WP08/T35.L；WP04/T24.L |

## 7. OS sink 已知调用族

下列精确调用数继承固定B的原entry inventory，不是本轮全仓重扫或成功显示计数。已读caller/assembly补足语义；平台内部转固定EB，不用计数证明闭包。

| ID | sink/caller | 内容、guard、失败、owner |
|---|---|---|
| O01 | clipboard11 direct+1 alias：MainHeader、useDebugLog、Markdown、useMessagesViewState、useFileLinkOpener、ClonePrompt、useSidebarMenus、GitDiffPanel SHA/basename/path三点、useCopyThread、SettingsEnvironmentsSection alias | cd/debug/代码/消息/会话/fileURL/path/threadID/script各自隐私出口；catch不一；WP09b/T23.L逐caller case |
| O02 | openUrl9：About、Markdown普通/代码块、update2、login、Git commit/PR/issue | 普通http/https/mailto、代码块http(s)，auth敏感，fire-and-forget非成功；WP05/T54.L；WP09b/T23.L |
| O03 | reveal12：Files/file-link/OpenApp/MainHeader/prompts/worktree/clone/config/Agent动作 | 外部绝对/native根不能统一套workspace包含；逐caller失败保留；WP08/T22.L |
| O04 | dialog open7/save1/ask7/message3：workspace(s)/image/export picker、settings paths、Git/worktree/clone确认 | chooser不写，后续command才writer；取消不写；ask非daemon auth；WP08/T22.L、T34.L；WP04/T19.L |
| O05 | popup10：Git3、Files1、Sidebar4、Prompts1、file-link1 | action闭包非纯显示；Sidebar实际接archive/remove/apply/rename；WP05/T12.L；WP08/T34.L |
| O06 | startDragging/minimize/toggleMaximize/close各1；single-instance/reopen/show/focus/hide | 视图非Session/host，多best effort；WP05/T09.L、T12.L |
| O07 | setEffects4、theme/NSAppearance | save非外观已应用，保通用视觉/无障碍；WP05/T12.L |
| O08 | setZoom1、mobileviewport/gesture、iOS4inset分支 | DOM与native分开，Windows不背书mobile；WP05/T12.L、T54.L；WP10b/T45.L |
| O09 | native notification send1及completion/response-required/test wrapper；app+daemon osascript2 | permission/debug/fallback分开，body含私有文本；WP09b/T23.L；WP05/T66.L |
| O10 | macOS tray labels/menu/action→tray-open-thread→connect/refresh | rawID不是public authority；隐私/未读/目标重核；WP05/T08.L、T66.L |
| O11 | AudioContext create/resume/fetch/decode/gain/destination/start；agent声音/设置测试2类caller | 非durable receipt，autoplay/device失败、活动时间隐私；WP05/T12.L |
| O12 | window-state plugin、HEIF/icon、OS file manager/external app | 精确plugin root在EB01；temp在RT19；program P07–P09；WP03/T50.L；WP12/T37.L |

## 8. localStorage 22 家族

open-workspace-app；composerEditorExpanded；codexmonitor.promptHistory.<scope>；gogo-party.pendingPostUpdateVersion；codexmonitor.threadLastUserActivity；codexmonitor.pinnedThreads；codexmonitor.threadCustomNames；codexmonitor.threadCodexParams；codexmonitor.detachedReviewLinks；codexmonitor.collapsedGroups；codexmonitor.sidebarWidth；codexmonitor.rightPanelWidth；codexmonitor.chatDiffSplitPositionPercent；codexmonitor.planPanelHeight；codexmonitor.terminalPanelHeight；codexmonitor.debugPanelHeight；codexmonitor.sidebarCollapsed；codexmonitor.rightPanelCollapsed；codexmonitor.threadListSortKey；codexmonitor.threadListOrganizeMode；reduceTransparency；mobile-remote-workspace-recent-paths。

43调用（17get/24set/2remove）是继承census。偏好归UI；输入历史按privacy domain；native参数进Binding extension；thread/review关系显式迁移；pending版本非安装证明；mobile旧配置不能恢复封存。原inventory§6给reader/writer，R3和R1-042给失败链，逐行traceability给测试。首验WP05/T07.L、T66.L，迁移WP12/T37.L，坏值WP02/T39.L。

## 9. 固定外部边界

| ID | 固定身份 | 已读调用面与未声称行为 | owner/首个阻塞检查 |
|---|---|---|---|
| EB01 | B的apps/desktop/src-tauri/Cargo.lock与apps/desktop/package-lock.json，Tauri/plugins/window-state/xterm/renderer条目 | registry/API/capabilities已核；native root/OS permission/CSP/OSC52未实测 | WP05/T12.L、T54.L；迁移前WP12/T37.L |
| EB02 | fix-path-env revision c4c45d503ea115a839aae718d02f79e7c7f0f673，B的desktop Cargo.lock | main prelude已读；内部shell/env与实际加载版不猜 | WP03/T50.L、T60.L；WP12/T60.M |
| EB03 | B的desktop Cargo.lock：portable-pty/tokio/libc/git2/reqwest/CPAL/Whisper及checksum | wrapper/caller/读写取消清理已归属；OS子孙/文件原子性/权限设备/HTTP默认未验 | WP03/T52.L；WP08/T22.L；WP10a/T57.L；恢复voice另授权 |
| EB04 | B的root/desktop/npm/Cargo/pnpm locks、3workflow实际Actions ref | scripts/exports/workspaces/Vite/build.rs/mode已核；生成Apple/NSIS/外部Action及实际产物身份未产出 | WP03/T60.L；WP12/T46.L、T60.M |
| EB05 | OS sips/defaults/osascript/launchctl/reg/PowerShell及用户git/gh/codex/Tailscale/app | 已知program/env/cwd/stop家族登记；未读用户机器实际binary/version/ctime/digest | WP03/T50.L、T52.L；WP09a/T58.L/N1；WP12/T59.L |
| EB06 | native provider、GitHub/Git remote、ASC、Tailscale身份系统 | 已核命令/URL来源和认证传播；未访问真实账号/费用/私钥/remote状态 | WP09b/T23.L；WP08/T34.L受控环境；WP12授权发行 |

仓库内可读文件没有改称第三方。F20是已分类的独立旧执行产品/非整体采用资产，不授权另起Room宿主。没有形式化全函数可达性证明；Pass B必须从B独立重建家族和反向caller，新增改变边界的路线即CHANGES_REQUIRED。


## 10. B01–B03 修订：选择性 Node 机制的具体权威（必读）

下面是源码缺口的承接，不是批准复用、运行通过或另立 Room 权威。所有源码仍固定 B。`reuse-contracts.json` 是逐符号选择、40 条相对 import 和 37 个持久化调用点的机器可读展开；完整 SHA/blob 在同目录 source-identities.json。不是只因“未导入 server.ts”就视为无传递副作用。

| 家族 | 固定入口与路线 | sink/caller、当前行为和要求 | 责任与首验 |
|---|---|---|---|
| F23 | Room server.ts:4900-4938 shutdown、2783-2840 closeAndRelease；Seat.close → 三类 provider close → close.ts | S17；8s 宿主期限与10s grace冲突；124不证明死亡、125和二次close不得释放残留；唯一 gogoke ExecutionHost 承接整个预算与耐久custody，不采用Room总服务 | WP03/T32.L,T52.L,T63.L；typed outcome WP01，耐久记录WP02 |
| F24 | seat-runtime.ts/claude-seat.ts/grok-acp-seat.ts 构造、start/writePrompt/emit/close；imports persistence.ts/close.ts/seat.ts | S16/S17；dataRoot与credentialHome分开；owner/state/transcript/protocol/events/prompts、temp/fsync/rename及home例外逐项归属，代码存在不等于允许直接调用 | WP02/T39.L,T55.L；WP03/T50.L,T51.L,T63.L；WP04/T11.L,T49.L |
| F25 | Room accounts.ts/instances.ts/proc.ts/mask.ts 的显式选符号边界；index.ts barrel与demo.ts只作反例 | S18；accounts→mask，instances→proc/accounts/mask；AccountStore/InstanceStore和ROOM_BOOT_ID、busy/坏文件清空不是公共权威；mask纯函数不代表调用者权限 | WP00逐符号；WP02/T39.L,T55.L；WP03/T50.L,T63.L；WP09b/T23.L |

| 反向家族 | 反向 caller 与范围 | 权威 |
|---|---|---|
| S16 | F24 的 Node persistence 写入；所有37个直接写入/helper调用点，不把同步函数/rename误当单writer | RT26-30、W21-28 |
| S17 | F23/F24 的 native close、身份query、kill、重复close、宿主退出及writer移交 | P20-23、Q14 |
| S18 | F25 的 account/instance 存储、probe/login/logout/remove 和纯mask区分 | RT31-32、W29-30、P24、Q15 |

### 10.1 root 与迁移

| ID | root | 具体 resolver/对象 | 首验 | 迁移与边界要求 |
|---|---|---|---|---|
| RT26 | dataRoot | resolve(options.dataRoot), hence relative input depends on process cwd; not credentialHome | WP02/T55.L | Validate canonical rootId/schema/alias; no second product data authority |
| RT27 | dataRoot/seats/seatId | owner.json/state.json/transcript.jsonl/protocol.jsonl/grok-acp.jsonl/events.jsonl | WP02/T39.L | Retain source+hash+provider+privacy+generation; migration to sole store idempotent, originals preserved; never merge homes implicitly |
| RT28 | credentialHome or seatRoot/home | Codex/Claude keep supplied string; Grok resolves it; explicit boolean bypasses overlap assertion | WP03/T50.L,T51.L | Do not copy secrets or migrate native home; authorization bound to exact canonical profile/host/revision/domain |
| RT29 | credentialHome/.grok and AppData/{Local,Roaming} | Grok mkdir(.grok); Codex mkdir(AppData/Local); env defines HOME/USERPROFILE/APPDATA/LOCALAPPDATA/GROK_HOME | WP03/T50.L | Provider-native cache/history/auth remain native-owned; verify actual child environment later |
| RT30 | seatRoot/prompts/<generated-name> | writePrompt -> atomicWriteFile; prompt files distinct from account home | WP02/T39.L | Public material store needs content/privacy identity, retention and collision policy before reuse |
| RT31 | dataRoot/accounts/<id>/{account.json,home} | AccountStore constructor mkdir; create/save/softRemove write JSON; native login/logout/probe separate | WP02/T39.L | Do not adopt store; migrate metadata only with explicit rules, never clone credential contents |
| RT32 | InstanceStore.root/{instances.json,instances/<id>,instances.json.tmp,*.broken-*} | load/carryOver/save/claim/release/remove and OS identity helpers | WP02/T39.L,T55.L | Non-adopted store; inventory only; preserve corrupt data and uncertain busy identity; no auto-release on probe failure |

### 10.2 writer、失败与锁

| ID | 目标 | reader/caller/writer | 当前失败/锁语义 | 责任/首验 |
|---|---|---|---|---|
| W21 | owner.json | Codex/Grok start -> real-query owner; Claude start uses String(Date.now()) for public owner while close uses a separate queried promise | No multi-process writer lock; failed persistence can follow spawn | WP02/T39.L;WP03/T51.L,T63.L |
| W22 | state.json | Codex/Grok close after clearing child/session refs | exitCode=0 -> stopped else failed, not proof of process gone; Claude does not write this file in inspected implementation | WP03/T32.L,T63.L |
| W23 | transcript.jsonl | all three: start/turn/close/aux markers via appendJsonLineAtomic | Whole-file RMW; competing snapshots can overwrite; close logging can throw after custody lost | WP02/T39.L;WP03/T63.L |
| W24 | protocol.jsonl and grok-acp.jsonl | Codex/Grok request/notification/stdout/stderr -> append; Claude no separate protocol path in inspected class | Raw provider output may be private; read/write/rename failure differs from transport acceptance | WP02/T39.L;WP04/T49.L |
| W25 | events.jsonl | emit -> append in three classes | Claude catches append error then continues event delivery; Codex/Grok append can throw; callback success not durable event proof | WP02/T39.L;WP04/T16.L,T49.L |
| W26 | prompts/* | writePrompt -> atomicWriteFile in three classes | Write completion not provider acceptance; material must stay under seat/public root, not bound home | WP02/T39.L;WP04/T11.L |
| W27 | native home and native-created files | start mkdirs, provider child writes after injected env; Codex/Claude/Grok roots differ | Host-selection flag is caller-provided, not an approval receipt; no copy/migration of tokens; exact native internal writes external platform validation | WP03/T50.L,T51.L |
| W28 | persistence temp and final destination | mkdir parent -> unique pid+UUID .tmp opened wx/0600 -> write -> fsync file -> close -> rename -> best-effort fsync directory | write/fsync/rename errors propagate with temp possibly left; directory fsync error swallowed; not a cross-writer lock, no multi-file transaction, append reads whole previous file | WP02/T39.L,T55.L |
| W29 | AccountStore account.json / homes | create/save/softRemove; fs.writeFileSync direct; login/probe separate process effects | No store-level file transaction/lock seen; bad get/list data null/skipped; softRemove preserves home. Not adopted implementation | WP02/T39.L;WP09b/T23.L |
| W30 | InstanceStore JSON / temp / dirs / quarantine | save fixed .tmp then rename; load attempts broken rename then empty pool; claim/release/remove writes/deletes | No cross-process serialization; instance ID/ROOM_BOOT_ID not full execution identity; unknown query must not release old writer; no automatic native-home deletion. Not adopted implementation | WP02/T39.L,T55.L;WP03/T63.L |

### 10.3 process、残留和权限

| ID | program/caller/identity | 当前语义与必须保留的风险 | 责任/首验 |
|---|---|---|---|
| P20 | 三 provider close → closeSeatProcess/terminateProcessTree | stdin.end→10s grace→身份检查→kill命令→5s观察；还含identity/kill各自时间，不能说15s为总期限。124仍可在exit Promise未完成时返回；POSIX只kill直接PID | WP03/T32.L,T52.L,T63.L |
| P21 | close.ts queryProcessStartTicks/runToCompletion | Windows PowerShell、Linux /proc、其他ps；记录为空可仅句柄放行，当前查询失败与身份不符不同；必须捕获实际身份/平台和子孙，helper名字不是证明 | WP03/T50.L,T52.L,T63.L |
| P22 | 非采用Room的shutdown/closeAndRelease/safelyCloseSeat | shutdown忽略数字；closeAndRelease初次保125却释放124；provider清引用后二次close=0可误释放；second signal强退。仅用作被选机制的caller反例，不引入Room宿主 | WP03/T32.L,T63.L |
| P23 | PersistentCodexSeat/PersistentClaudeSeat/PersistentGrokSeat start/close | Codex/Grok公共owner使用query；Claude owner字段是Date.now而close另存query promise；cwd/home/env/child identity按provider分开，不能按同一字段名视为等价 | WP03/T50.L,T51.L,T63.L |
| P24 | accounts.ts native discovery/probe/login/logout、proc.ts execFileSync | 选pure parser不自动许可进程调用；proc查不到可因查询失败，不等于已退出；原account/instance存储与native认证流程不原样采用 | WP03/T50.L,T63.L；WP09a/T58.L；WP09b/T23.L |

| ID | 权限对象 | 要求（尚未生产实现） | 首验 |
|---|---|---|---|
| Q13 | explicit host-home exception | Owner selection must produce scoped approval naming provider/host/canonical home/account auth revision/domain/generation/allowed effects/expiry; true boolean is not that receipt. No implicit fallback, no automatic secret migration | WP03/T50.L,T51.L;WP09b/T23.L |
| Q14 | root/writer release and close result | Quarantine unresolved process/home and descendants; retain immutable custody across retries and restart. A missing handle is not proof of no owner | WP03/T52.L,T63.L |
| Q15 | selective helpers and raw logs | No wholesale accounts/instances import by barrel; transitive process/probe/logging effects declared per selected symbol. Redaction is not declassification or permission | WP04/T11.L,T49.L;WP09b/T23.L |

### 10.4 关闭结果与释放判据

必须区分：自然退出确认、非零退出确认、已尝试kill、拒绝kill、直接子进程已退但后代未知、残留未知、宿主期限到、close异常、无句柄但仍有未解custody。原0/124/125和Promise resolve都不能单独代替这些事实。

唯一ExecutionHost在WP01/02/03合同下持有rootId/binding/generation/instance/credentialHomeId/PID+startTicks/handle/descendantScope/writerLease/lastSeen/closePhase/deadline/exit evidence和耐久收据。仅当精确身份已退出、后代处置有证据、旧writer已封口、收据可耐久读取时释放或迁移。其余保持quarantine，禁止二次close、进程重启或缺句柄绕过。宿主总期限预算含身份查询、强杀、观察和落盘；强制提前退必须先记录不确定性，不能仅把8改成10解决。

测试分层：`safe_source_fixtures.mjs`在假FS/假时钟/假child下执行固定TS算法与精确截取的两个server函数；未来合同oracle独立标注。它会确认原源码的坏路径仍存在，不把检出缺陷的PASS计成生产修复。真实OS/PID/后代/锁/fsync/跨进程竞争/凭据域/平台运行仍NOT_RUN。
