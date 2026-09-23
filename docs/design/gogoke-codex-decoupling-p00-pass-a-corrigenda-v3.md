# P00 Pass A：R1–R6 与检查点的有效证据纠正

生产源码 B=`bc665a852833952b76d9508401193bedd2198436`；原始观察 H=`b88988fd9621b048e883336d79ab1bb433a4cefa`；上一检查点 P=`59dd05cfe1321da7650e6cbb61bb17858b083710`。本表按行ID纠正旧摘要，不修改历史原件、不删观察。有效证据为原观察 + 本表 + 六份共303行traceability + 133行surfaces + authorities/capabilities。旧报告的OPEN/frontier是当时状态，不能仅从历史文件未改推定新补证未消费它；也不能删掉旧记录来关闭缺口。以下source默认为apps/desktop下固定B。全部T均是后续要求，未执行产品修复/运行验收。

| ID／受影响观察 | 纠正事实、相邻调用 | 固定源码／首验与独立断言 |
|---|---|---|
| K01 R1-055；R2-039/040；R5-001；旧E01/E09 | 101个同名Tauri+daemon method不等于101个桌面转发器。19个shared入口桌面始终本机：settings3、prompts7、workspace4、doctor、accelerator、mac-debug query、notification fallback、remember-rule。其他方法按surfaces逐项，不从同名推等价。 | src-tauri/src/settings/mod.rs、prompts.rs、codex/mod.rs、workspaces/commands.rs、notifications.rs、menu.rs及5daemon handler；WP10b/T43.L：逐方法记录client/server目标，拒绝静默主机变更。 |
| K02 R1-052；R5-022/024；PA03 | read_settings→finalize_loaded_settings→归一化重写可写配置，错误只记日志。read_workspaces的内存归一化不因此等价于同一种持久写。 | src-tauri/src/storage.rs；WP02/T39.L：兼容数据读取遇写拒绝保原件，报告读诱发writer。 |
| K03 R2-030/044；R5-025/027；PA06 | 普通kill_session_by_id检查剩余Arc引用；daemon sync_workspaces_from_storage直接kill被移除key的Arc，无surviving-alias检查，两条释放链不同。 | bin/gogoke_daemon.rs与shared/workspaces_core/connect.rs；WP03/T63.L：A/B共进程、磁盘删A后list不能杀B或虚报connected。 |
| K04 R2-029/031；PA08 | version probe发生在后续app-server workspace cwd/CODEX_HOME设置前；probe身份不能替代spawn身份。wrapper隐藏console/补PATH不等于清洗env。 | backend/app_server.rs与shared/process_core.rs；WP03/T50.L：分开捕获probe/spawn program/env/home/cwd。 |
| K05 R1-007/010/012；PA09/10 | initialize外层15秒timeout会kill；response取消、initialized写失败是不同早退；raw JSON error不因Rust Result=Ok自动拒绝。PTY补偿/EOF/kill同样非统一终态。 | backend/app_server.rs、terminal.rs；WP03/T32.L分别注入协议error、取消响应、stdin失败；T52.L另验子孙，不能一条fake退出代替。 |
| K06 R6-036；PA16 | settings先best-effort写5项native config，再settings.json；getter读native覆盖值，桌面另apply theme。公共偏好不应继续有未披露native Home写入；save/theme成功分开。 | shared/settings_core.rs、settings/mod.rs；WP07/T26.L逐项拒绝native写；WP05/T12.L另验theme失败。 |
| K07 R6-027/028/029/031；PA17 | writer只在candidate.exists()时查symlink/canonical target；悬空链接分支缺同等校验，可进入directwrite。因此“strict策略拒绝所有外部symlink”过强。global AGENTS显式外部例外仍单列。 | files/io.rs:write_text_file_within、files/policy.rs；WP08/T22.L：workspaceAGENTS/globalconfig分别测existing/dangling，另测globalAGENTS兼容和替换竞态。旧Python模型不算生产Rust复现。 |
| K08 R6-002/026；R2-043 | 风险不止显式cwd fallback：Codex Home可保留相对/未展开路径，Files可mkdir后canonicalize；输入实际根受进程cwd影响。 | codex/home.rs、files/io.rs；WP02/T55.L：缺home、相对路径、未展开tilde不得默建另一公共可写根。 |
| K09 R6-032/033/034/035；PA18/19 | file_read/write remote分支操作daemon文件；write_text_file、remote/mobile guarded image read和external-open/icon仍是当前Tauri主机。save dialog不是任意IPC的授权收据。 | src/services/tauri.ts、src-tauri/src/files/mod.rs、workspace io；WP08/T22.L/T35.L：同名双主机路径、绕picker IPC、本机image分别验目标。 |
| K10 R6-038/039 | 普通Markdown允许http/https/mailto，fenced URL block只http(s)；file/thread/fragment分路；urlTransform先识别文件路径再排未知scheme，不能统称HTTP-only或任意scheme。 | src/features/messages/components/Markdown.tsx与hooks/useFileLinkOpener.ts；WP05/T54.L：各scheme、file、fragment、模型伪批准分开验；runtime未验。 |
| K11 R3-050；R6-039；C10 | native drop用路径扩展名/target rect；browser drop/paste用FileReader data URL；picker/useComposerImages捕获draftKey，没有统一public generation。 | src/features/composer/hooks/useComposerImages.ts、useComposerImageDrop.ts；WP08/T35.L：切workspace/thread后分别完成picker/drop/paste，不串原件与目标；读拒绝单验。 |
| K12 R5-040 | talescale是报告笔误，源码为tailscale；macOS launchctl asuser失败可direct重试。status可触CLI并返回tailnet身份，非纯文件读。 | src-tauri/src/tailscale/mod.rs、core.rs；WP10a/T56.L：startup/settings/status封存不调用外部Tailscale，保其系统身份。 |
| K13 R4-001/003/010/037；PA29/30 | Vite module evaluation可执行两个git rev-parse fallback；general ci.yml的PR触发不同于desktop/assurance过滤；root room:start每次install→seat build→server，非只读测试。 | vite.config.ts、根.github/workflows/{ci,assurance,gogoke-desktop}.yml、tools/start-room.mjs及package manifests；WP03/T50.L/T60.L固定假命令逐入口；WP01/T61.L区分CI范围/NOT_RUN。 |
| K14 R4-014/015/017；PA20 | updater有host/redirect/byte预算，但client()未设应用connect/total timeout；请求可占UPDATE_LOCK。250MiB不是时间上限，不猜库默认。 | gogoke_update.rs:client/get_bytes/gogoke_update_check；WP12/T59.L：受控无进度流/断流/错误redirect分别终止并保状态；不实际安装发布。 |
| K15 R6资源范围；旧E09/RT04/RT12 | prompts_list与两个dir查询会mkdir；useCustomPrompts在connected workspace自动refresh并log完整response；查询非纯读，桌面prompt7均不转发。 | shared/prompts_core.rs、prompts.rs、src/features/prompts/hooks/useCustomPrompts.ts；WP07/T25.L区分目录查询/隐含写权限；WP09b/T23.L核prompt内容诊断。 |
| K16 R6资源writer；旧E09 | 【R2纠正】prompt update先对next_path执行fs::write，再在改名时remove旧target；不是rename后write。跨设备move为copy后remove，失败可留两份；检查不构成事务。 | shared/prompts_core.rs；WP07/T25.L：next_path写成功/旧target删除失败与跨设备copy成功/remove失败分别保部分收据，禁盲重试。 |
| K17 R6-036相邻Agent/config；旧E08 | 【R2纠正】create先写role文件再persist总config，失败尽力删新文件；delete先备份并删managed文件再persist总config，persist失败尝试恢复。update也有内容/rename补偿；外部文件不得冒充managed删除。Agent read/write接口均只接受managed role，list外部引用不等于该接口可读外部内容；write新内容直接落盘，未先解析role TOML。 | shared/agents_config_core.rs、config_toml_core.rs；WP07/T26.L：总config失败及补偿失败分别留收据，delete恢复失败不得写全成功；外部文件不删；未知TOML字段保留。 |
| K18 R1许可/R6 config相邻；旧E05 | remember-rule桌面本机执行。rules是create-new锁文件、wait2s/stale30s删除，非持有OS锁；read失败回空再整文件write。ok/rulesPath不证native已重载。 | codex/mod.rs、shared/codex_core.rs、rules.rs；WP04/T19.L分记规则/答许可；WP07/T25.L分别验读失败/过期锁/写失败保原件。 |
| K19 K18相邻去重 | rules去重删除全部空白，包括引用内空格；remember入口trim并过滤空token。不能把文本归一化当语义等价或授权证明。 | rules.rs及remember_approval_rule_core；WP04/T19.L分别测含空格/无空格token、空token、重复规则，不能误判已存在。 |
| K20 R3-033；R6-041；旧E07 | account_read即使native失败仍可读auth.json JWT payload生成email/plan；helper未验签。metadata非已登录/官方额度，未读用户真实auth。 | shared/codex_core.rs、shared/account.rs；WP09b/T36.L：陈旧/改写payload不成为实时官方状态；T23.L不导出token。 |
| K21 R6-041；旧E07 | cancel API按workspace查PendingStart/存储LoginId，不由UI给任意ID；取消本地等待不证native登录未开始；新login替换旧LoginId无等价native cancel收据。 | shared/codex_core.rs登录start/cancel；WP04/T20.L：迟到响应不能复活取消流程；WP09b/T23.L核raw/auth URL传播。 |
| K22 R2-005；C31；旧E06 | 三生成器创建hidden thread、请求readOnly/approval never；60s collect后best-effort archive并非interrupt/树停止。输入/输出不同，必须三个case。 | shared/codex_aux_core.rs；WP08/T30.L分别测commit/run/agent及失败草稿；WP03/T21.L验真实只读；WP04/T18.L核archive非stop。 |
| K23 PA04；R1-037/R6-050相邻 | App初始label=main，effect才取真label；可能先MainApp再About。是源码顺序风险，不宣称运行复现。 | src/App.tsx、features/layout/hooks/useWindowLabel.ts、MainApp/bootstrap；WP05/T12.L：About不得触main workspace/ready副作用。 |
| K24 R1–R6身份/旧候选 | R2–R6中间提交生产树与B等价；303观察保ID；P的PA01–PA32是额外风险非新功能/测试。旧checkpoint标2026-09-19不作为本轮当地日期，本轮为2026-09-18。 | 固定compare及所有映射；WP00/T68.P：禁floating ref代SHA、作者自查代B、metadata日期代runtime；生产和现有测试不改。 |

| K25 B01/R5-001 | local_usage_snapshot归workspace:literal，menu_set_accelerators归daemon:literal；literal分布6/25/39/7，Git常量27，77/104总数不变。原总数检查不足。 | rpc/workspace.rs:266、rpc/daemon.rs:18；WP00本轮修表；WP01/T61.L逐method+handler+arm-kind比较，并拒绝保持总数及分组数的标签交换。 |
| K26 B02/C14.2/C33.2 | F20/P17/P19不能代替实际Node关闭合同；Room8s、Seat10s、124/125、各provider清引用时序和二次close→release链展开。保留残留身份和writer隔离，不采用Room总服务。 | authorities§10 F23/P20–23/Q14；reuse-contracts.lifecycle；WP03/T32.L,T52.L,T63.L。 |
| K27 B03/C37.1/C42.2 | 选runtime会传递带入persistence/close/seat；dataRoot、credentialHome、显式real-home、具体文件writer/temp/fsync/RMW及Room辅助模块逐符号承接。显式本机CLI不是固有漏洞，但boolean不等于授权收据。 | authorities§10 F24/F25 RT26–32/W21–30/Q13/Q15；WP02/T39.L,T55.L；WP03/T50.L,T51.L,T63.L；WP04/T11.L,T49.L。 |

## 作者侧缺口关闭账本

G01：F01–F03/F06/F08/F11–F13/F16/F17/F21/F22、O01–O12与K09–K11/K23覆盖自动入口、菜单assembly/回调、plugin/browser直接sink。精确callsite inventory固定继承，补读已定位叶入口；不声称全函数机器可达性证明。

G02：F04–F11、RT/W/P/Q表、K02–K08/K15–K22补native/config/rules/prompts/agents/generation。注册command/daemon/API本身也是有效入口根，不伪造缺失的按钮。

G03：133行surfaces逐项列129 Tauri、104 business、auth、handler/主机/retry/effects。K01明确同名不等价。daemon进程、settings、native、Git与资源caller分别归属，不把104方法缩成一个RPC功能。

G04：F19/F20/EB固定HTML、alias/lazy import、cfg/path/include、package/export/build/script与依赖；六trace表保303观察，capability表保84行为，PA01–PA32按下文继承。【历史声明已被新B的B02/B03纠正】旧F20总括不足；本轮以K26/K27及authorities§10补具体跨层/传递合同，是否充分仍待定向复核，不再用该旧声明关闭缺口。新窗口必须从固定源码独立重建，找到新路线即可要求修正。

## PA01–PA32 消费关系

原PA事实/场景/检查在P的 `gogoke-codex-decoupling-p00-gpt-pass-a-v3.json` 原样保留，本表纠正优先：PA01/02/03→RT01–03/K02/K08；PA04→F01/K23；PA05/06/07/08/09/10→F04/F07/P01–06/K03–05；PA11/32→F14/N03–04；PA12→F08/Q11；PA13/14→F06/RT05；PA15→F05/W12；PA16/17/18/19→F09/F11/K06–09；PA20/21/22/23→F18/F19/W16–18/K14；PA24/25/26→F15–17；PA27/28→F08/F12/O09；PA29/30→F19/F20/K13；PA31→F04/P02。历史32项不是303个R观察的一部分，也没有因新表而被删除。

## R2处理范围

K25–K27承接最新CHANGES_REQUIRED。K16/K17按固定原源码同步纠正操作顺序与Agent read/write策略；这是相邻证据一致性纠错，不修改生产实现，旧版仍由原payload保留。原PF07增量不构成新B的通过依据，仍等待独立定向评价。
