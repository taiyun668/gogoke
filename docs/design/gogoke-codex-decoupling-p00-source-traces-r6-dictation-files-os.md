# gogoke P00 接手摸排 R6：Dictation、Files、Plugin 与 OS 副作用

**日期：2026-09-18｜源码基线：`18de5126a7a58748133ad241ce37a6edbef61cd8`｜证据层级：SOURCE_BRANCHES_EXPANDED**

本批完成 R1 `RF08` 的dictation/files部分，并扩展`RF03/RF05`中的plugin、clipboard、menu、tray和window入口。语音首版目标是封存保留；当前命令、快捷键、权限、下载和录音路径仍active/source-present，不能标成已经封存。

- `P00 acceptance = NOT_ACCEPTED`
- `global orphan_sink_count = null / NOT_COMPUTED`
- 未请求麦克风、未下载/删除模型、未录音/转写、未写用户文件、未打开URL/app、未用剪贴板/对话框/系统菜单。
- 本文区分源码能力与实际运行，不将存在测试或权限配置写成runtime已验证。

## 1. Dictation root、model与download

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R6-001 | platform module | desktop/macOS/Linux/Windows编译`real.rs`；iOS/Android编译stub。不能由desktop实现外推mobile能力。 |
| R6-002 | model root | `app_data_dir()/models/whisper`；app_data解析失败回`current_dir()`再`models/whisper`。违反V3禁止fallback cwd。 |
| R6-003 | model catalog | tiny/base/small/medium/large-v3固定HuggingFace URL和SHA-256；用户不能任意传URL，但下载依赖外部host。 |
| R6-004 | status Ready判定 | `refresh_status`只检查目标文件exists；不会在启动/状态查询时重算SHA。损坏/被替换文件仍显示Ready，直到Whisper load/transcribe失败。 |
| R6-005 | model hook自动入口 | `useDictationModel` mount或modelId变化会自动调用status，并订阅download事件；不会自动下载。startup error被吞。 |
| R6-006 | download switch | 新model下载开始时，如另一个model正在下载会set旧cancel flag并abort旧task；随后覆盖global download handle/status。旧task与新status的竞态需runtime测试。 |
| R6-007 | download client | connect timeout10秒、总timeout30分钟；没有源码级max bytes。content-length只用于progress，磁盘容量/大响应由OS/HTTP失败处理。 |
| R6-008 | partial writer | `File::create(<model>.partial)`会截断既有partial；chunk边写边hash，约150ms更新状态event。emit失败被忽略。 |
| R6-009 | cancel/error cleanup | cancel/HTTP/write/hash/flush/rename错误多数best-effort删partial；删除错误被忽略。abort后command另行remove partial，但无法证明旧task已经完全停止访问该path。 |
| R6-010 | publish model | hash匹配、flush后rename partial→final；若同名final已存在或跨平台rename语义不同会error并删partial。没有backup/atomic replace合同。 |
| R6-011 | remove model | 直接remove final；随后如cached context同model就drop。没有trash/undo；进行中的transcription对Arc context仍可能继续。 |

## 2. Permission、capture、processing和快捷键

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R6-012 | macOS permission | explicit request command或start都可触发系统prompt；per-process AtomicBool限制denied/restricted后只再尝试一次。main-thread callback最长等60秒。 |
| R6-013 | non-mac permission | 源码直接返回true；Linux/Windows实际设备/OS权限错误只在CPAL open/build/play阶段显现。 |
| R6-014 | start | 必须model状态Ready且state Idle；先permission，再spawn capture OS thread并等待ready channel。start返回Listening只表示stream play已成功。 |
| R6-015 | capture | 读取default input device/config，转换多channel为mono，最多存120秒样本；超过上限后继续stream但不追加audio。另一个线程约每33ms emit level。 |
| R6-016 | capture error | CPAL error先emit Error、send stop，再async把state/session清Idle。emit失败不可见，stop/channel和state更新不是单事务。 |
| R6-017 | stop | state立即改Processing、take session、send stop并await capture thread；之后spawn async transcription，command返回Processing，不等待transcript/error。 |
| R6-018 | model context | 首次processing在spawn_blocking加载model并缓存Arc；cache没有文件mtime/hash binding。model文件被替换后，既有cached context继续用旧内存。 |
| R6-019 | transcription cancel | Processing时cancel只set flag、把UI state改Idle并emit Canceled；已经进入`spawn_blocking(transcribe_audio)`的CPU工作不会被中止，完成后仅检查flag并压掉输出。 |
| R6-020 | listening cancel | send stop、await capture、清audio并emitIdle/Canceled；send/wait result被忽略。 |
| R6-021 | transcript durability | backend只emit text；frontend生成Date+random ID并存React state，消费后clear。不是持久 transcript/Delivery，也不绑定Session。 |
| R6-022 | hold shortcut | window全局keydown开始、keyup停止、blur取消。按键在permission/start完成前释放时只保留1.5秒pending；若Listening晚于grace，可能不会自动stop，需runtime测试。 |
| R6-023 | hold errors | shortcut wrapper吞Promise rejection，依赖dictation event显示；如果command失败且event也emit失败，UI可能无反馈。 |
| R6-024 | source-active判断 | commands全部注册，bootstrap构建dictation hooks，快捷键监听存在（由enabled/ready guard）；因此当前只能写`active/source-present`，不是`preserved_disabled`。 |

## 3. 公共/Workspace文件读写与Codex root耦合

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R6-025 | supported files | API只支持workspace AGENTS、global AGENTS、global config；workspace config拒绝。 |
| R6-026 | global root | global scope直接解析default`CODEX_HOME`。这是真实公共设置/文件UI对Codex root的专属依赖。 |
| R6-027 | workspace root | 由workspace map path取得，canonicalize且必须目录；workspace AGENTS拒绝外部symlink。 |
| R6-028 | global AGENTS symlink | policy明确`allow_external_symlink_target=true`；若`CODEX_HOME/AGENTS.md`是symlink，read/write允许目标位于root外。这是保留的现有语义，内化时必须决定兼容/迁移，不能误称所有file_write都root-contained。 |
| R6-029 | global config symlink | external symlink拒绝；与global AGENTS不同。 |
| R6-030 | file read size | 直接`read_to_end`且TextFileResponse.truncated固定false；没有大小上限。大文件可占内存，需P07合同/测试。 |
| R6-031 | file write semantics | 直接`std::fs::write`目标，没有temp+rename、backup或fsync；失败时目标可能已被truncate/部分改变，具体依OS。 |
| R6-032 | remote file read/write | remote mode通过RPC在daemon machine执行；local mode在app machine执行。相同UI动作作用于不同主机文件系统，必须在目标/确认中显示。 |
| R6-033 | arbitrary export writer | `write_text_file(path, content)`仅检查非空，递归创建parent并直接write；backend不验证用户是否通过save dialog选择、也不限制产品/workspace root。上游选择/确认必须保留。 |
| R6-034 | image read in remote/mobile | command只允许remote mode或mobile，但随后在**当前Tauri进程本机**调用Codex core读取normalized path，并未转发remote RPC。remote路径究竟属于哪台机器不能由命令名推定。 |
| R6-035 | generic capability location | `read_image_as_data_url`使用`codex_core::normalize_file_path/read_image...`承载一般图片能力；应内化/替换模块归属，保留格式与大小限制。 |
| R6-036 | settings save cross-writes | `update_app_settings_core`先best-effort写5项Codex config（errors全部丢弃），再写settings.json。Codex写可能已成功而settings写失败；反之Codex写失败仍返回settings成功。 |

## 4. Clipboard、opener、dialog和native menu

原P00精确inventory已经固定：11个直接`navigator.clipboard.writeText`+1个alias、9个`openUrl`、12个`revealItemInDir`、plugin-dialog 7 open/1 save/7 ask/3 message、10个native menu popup。R6不重计为新发现，只补真实入口语义。

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R6-037 | clipboard内容 | 包括workspace cd命令、全debug log、code/message/thread全文、file URL、copy folder、raw thread ID、commit SHA/path/file名和environment script。系统剪贴板是高隐私外部sink；copy不形成授权或Delivery。 |
| R6-038 | Markdown URL | fenced纯URL block只接受`http(s)`，点击fire-and-forget `openUrl`；普通Markdown link的完整renderer仍需独立scheme测试。模型正文可以触发用户点击后外部浏览器导航。 |
| R6-039 | file link | relative path按workspace拼接，绝对path保持，mounted path可重写；菜单/点击可reveal或交给配置app/command，还可复制`file://`链接。backend open command不强制path在workspace。 |
| R6-040 | open app target | 用户配置kind=app/command/finder；选择菜单项会先写localStorage selected target，再立即执行open。失败不会回滚选择。command/app及args由settings提供。 |
| R6-041 | account login URL | Codex login command返回authUrl后直接`openUrl`; cancel/login events另行追踪。auth URL是原生provider边界，不能成为公共任意URL executor。 |
| R6-042 | dialog语义 | open/save仅选择路径；实际write/mutation发生在后续command。ask/message只是本地UI确认/告知，不是Owner最终裁决、服务器权限或durable receipt。 |
| R6-043 | menu popup | 右键菜单handler可以在创建时封装mutation callback；“只显示菜单”本身不写，但后续action需按实际handler归类，不能只计10个popup。 |

## 5. Native menu、tray与window生命周期

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R6-044 | app native menu | 构建About/Update/Settings/New Agent/Workspace/Composer/View/Window等项。menu event多数先show/focus main window再emit frontend event；emit/show/focus errors被丢弃。 |
| R6-045 | check update menu | native menu直接emit`updater-check`，后续R4链会下载/write；菜单action不是只改变UI。 |
| R6-046 | quit/close | Linux custom quit直接`app.exit(0)`，最终仍进入RunEvent cleanup逻辑；close/window lifecycle按平台不同。失败结果多不反馈。 |
| R6-047 | tray data | macOS tray保存最近thread workspace/thread labels、raw IDs和usage labels到进程内state并构建OS menu。敏感项目名/会话名会离开主窗口。 |
| R6-048 | tray click | menu ID映射到workspace/thread payload并emit `tray-open-thread`；前端再切/连接/refresh。raw ID不应成为未来公共Session全局authority。 |
| R6-049 | window appearance | settings read和write都会调用apply theme；genericset_theme error被丢弃，macOS main-thread调度error才上报，但内部appearance error仍丢弃。settings保存成功不证明外观已应用。 |
| R6-050 | single-instance | 第二实例只show/focus现有main window；不会转发cwd/args给业务层。 |
| R6-051 | window-state plugin | active plugin可能持久化窗口状态，但repo源码未定义其native root/format；是固定external boundary，不能计为已迁移数据。 |
| R6-052 | platform startup | Linux写进程环境，macOS tray/close-hide/reopen，Windows decoration/menu，iOS native inset各是独立入口；Windows首发测试不能背书其他平台。 |

## 6. 当前闭合状态

- `RF08 dictation/files`：主要commands、backend状态机、frontend hooks和root/writer已展开；仍需Whisper/CPAL/reqwest固定版本实现、所有dictation UI按钮/setting切换与stub行为作external/platform边界确认。
- `RF03/RF05 direct OS/plugin`：高风险和代表性调用已展开；原inventory的完整精确调用列表继续作为callsite census，fresh审计需逐项核对其caller。
- 语音当前不满足`preserved_disabled`；WP10a必须封命令/UI/快捷键/权限/download/start入口并保留model资产。
- `CF01–CF05`仍OPEN；runtime未验。

## 7. 已读源码索引

固定生产源码来自`18de5126a7a58748133ad241ce37a6edbef61cd8`，dictation backend来自其父source未变提交`2422f32796bc4c9771bf6e92c9fcb39c667b4ffe`：

- `src-tauri/src/dictation/{mod,real,stub}.rs`
- `src/features/dictation/hooks/{useDictation,useDictationModel,useHoldToDictate}.ts`
- `src-tauri/src/files/{mod,policy,ops,io}.rs`, `src-tauri/src/shared/files_core.rs`
- `src-tauri/src/shared/settings_core.rs`
- `src/features/messages/{components/Markdown,hooks/useFileLinkOpener}.tsx/ts`
- `src/features/app/components/{MainHeader,OpenAppMenu}.tsx`
- `src/features/app/hooks/useAccountSwitching.ts`
- `src-tauri/src/{menu,tray,window,lib}.rs`
- `src-tauri/capabilities/default.json`
- `docs/design/gogoke-codex-decoupling-p00-entry-inventory-v3.md`的精确direct sink表

后续不得把“文件选择/菜单/通知/clipboard/opener”统称为纯UI；它们连接真实OS和隐私边界。
