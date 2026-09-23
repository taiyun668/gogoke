# P00 Pass A：固定证据的重放规范

这是审查规范和被动数据定义，不是执行产品命令的许可。R2另附B01–B03专用被动检查与隔离假件；不发布或重建历史IX01被拦截脚本。生产来源 B=`bc665a852833952b76d9508401193bedd2198436`，原观察 H=`b88988fd9621b048e883336d79ab1bb433a4cefa`，历史PA01–32来源 P=`59dd05cfe1321da7650e6cbb61bb17858b083710`。最终candidate/gate/Capsule给出同一个payload完整SHA；PR回执另给冻结登记SHA。使用固定SHA，不以分支名作证据。

## 1. 独立重建在前，候选对账在后

Pass B必须是未参与Pass A的新上下文。先读START_HERE和Capsule的输入/禁止/判据；自己从B重建入口和sink家族，然后才读取作者结论与下面的账本。独立重建至少包括HTML/React/native bootstrap、lifecycle、effects、direct browser/plugin、Tauri registry、adapter/core、daemon dispatcher/auth/CLI、package/build/config/include/dynamic import/exports、脚本与外部依赖。反向从file/Git/libgit2/process/stdin/PTY/kill/network/permission/OS/storage/install/sign/publish找caller和guard，不从候选表抄根集合。

任何会改变公共合同、root/writer、进程身份、权限、迁移、封存/删除范围的新可读源码路线，判CHANGES_REQUIRED，不得直接丢进EB。动态和外部内部实现不能形式证明时，保留明确边界和runtime检查；不要求捏造orphan=0。

## 2. 固定数据和连接规则

六份traceability-r1至r6 TSV使用UTF-8、TAB、LF、首行字段名。每个R ID恰好一次，数量分别55/44/50/55/47/52，总303。它们不复制旧观察全文，而是按ID连接H的同名R报告，继承实际入口、sink、当前事实、失败和证据。K01–K27有效纠正覆盖旧摘要（K16/K17已在R2更正）；新列补capability behavior、F/S、disposition、owning WP、first check、独立scenario。一个主要F/S不排除原观察和权威表列出的其他副作用。场景是验收要求，不是已通过测试。

capabilities TSV的完整ID集合是C01.1/C01.2至C42.1/C42.2，共84。capability_primary_wp保留V3能力持续主责；owning_wp是该行first_validation_checks的执行责任，不能把主责误当截止阶段。所有原V3测试及N1/I/M义务继续有效；本表是首验索引，不删其余测试。未来公共Session/Delivery/委托等写required_disposition，不把旧native接口当新实现。

surfaces TSV按method唯一，共133；129个Tauri，104个daemon business，共享101、Tauri-only28、daemon-only3；wire auth在business之外另1。5个handler必须按dispatcher真实顺序，literal arms共77（daemon6/workspace25/codex39/prompts7），git constant27必须解析shared/git_rpc.rs，不能漏常量或把参数match算method。36个retry在remote_backend/mod.rs逐值核对，不从read前缀推纯读。19个shared-but-desktop-local精确集合：get_app_settings、update_app_settings、get_codex_config_path、7个prompts_*、add_clone、apply_worktree_changes、open_workspace_in、get_open_app_icon、codex_doctor、menu_set_accelerators、is_macos_debug_build、send_notification_fallback、remember_approval_rule。

F01–F25、S01–S18、RT01–RT32、W01–W30、P01–P24、N01–N10、Q01–Q15、O01–O12、EB01–EB06在authorities中唯一。每个S至少有反向F；每类root/writer/process/network/permission/OS行必须有具体对象、调用根、失败/guard和owner/first check。检查数量不是证明这些关系正确的替代。

## 3. 阶段字典（首验，不替代V3完整测试表）

| 截止包 | L检查 |
|---|---|
| WP01 | T05,T61 |
| WP02 | T10,T39,T55 |
| WP03 | T21,T31,T32,T41,T50,T51,T52,T60,T63 |
| WP04 | T11,T13,T14,T15,T16,T17,T18,T19,T20,T24,T49,T53,T67 |
| WP05 | T07,T08,T09,T12,T54,T66 |
| WP06 | T28,T29,T65 |
| WP07 | T25,T26,T27 |
| WP08 | T03,T22,T30,T33,T34,T35 |
| WP09a | T06,T58 |
| WP09b | T23,T36,T40 |
| WP10a | T56,T57 |
| WP10b | T43,T44,T45 |
| WP11a | T04 |
| WP11b | T01,T02,T47,T62,T64 |
| WP12 | T37,T38,T42,T46,T59 |
| WP13 | T48,T68 |

T68.P→WP00；T68.D→WP01；T55.H→WP03；实际存在的N1→WP11a、I→WP11b、M→WP12。不得凭这段概括给原计划没有的检查随意新增阶段或称已通过。T60.M已用于发行身份边界。原计划固定B的第9节是测试时限权威。

## 4. 历史结构检查及负对照（不作为R2 handler正确性的证明）

本轮用临时内存中的标准CSV/JSON读取与断言，检查了303/84/133记录、ID唯一/引用、全部非空场景、WP/首验一致、登记交并集、handler分类、36retry和19主机例外。对8份TSV按Git blob规则计算身份，与create_blob返回值逐一相等。Markdown发布版人工核表和固定对象；较长本地起草版不作为发布字节证据。

以下每项都只改一份临时数据副本，预期REJECTED，实际REJECTED；原候选保持不变。不是生产mutation、不是Rust/TS源码解析器负测，不证明语义充分。

| ID | 单一修改 |
|---|---|
| N01 | 删除一条R观察 |
| N02 | 用重复ID替换另一条R观察 |
| N03 | 把一个C改成C99.1 |
| N04 | 把一个F改成F99 |
| N05 | 把一个S改成S99 |
| N06 | 将已有首验的owner改成错误WP |
| N07 | runtime_verified改true |
| N08 | code_entry_gate改OPEN |
| N09 | global_orphan_sink_count改0 |
| N10 | 删除一个method |
| N11 | prompts_list桌面路由改为remote-when-enabled |
| N12 | send_user_message加入disconnect retry |
| N13 | 删除transport-only auth |
| N14 | 将一个git constant arm改记为literal |
| N15 | 删除C42.2 |
| N16 | 两条不同R共用完全相同的scenario断言 |

读取器可自行实现这些明确的集合/字段检查；不得把作者的结果当独立复核。所有Git对象ID在Pass A JSON中；对blob运行Git自身身份校验，或直接从固定commit读取，并用标准TSV/JSON工具检验。没有要求安装依赖、执行package生命周期、运行产品或读取用户root。

## 4A. R2实际重放与坏基线防护

以原基线完整离线checkout作为SOURCE，包含本修订的payload为CANDIDATE。用Python3标准库执行：

```text
python3 docs/design/p00-pass-c-b01-b03-r2/verify_repair.py --source SOURCE --candidate CANDIDATE --out OUT/verification.json
node docs/design/p00-pass-c-b01-b03-r2/safe_source_fixtures.mjs SOURCE
```

Node假件所需为Node22与TypeScript5.8.3（默认读取同Node安装目录的global TypeScript；可用P00_TYPESCRIPT_PATH指向已经安装的编译器，不执行npm安装）。编译器只转译读取到的固定TS，module require限定为假FS/假spawn/假home/纯path/假UUID。完整server不加载，只截取shutdown和closeAndRelease函数。记录源码SHA、实际工具版本，真实产品runtime保持false。

必须先证明修后基线0错误，再执行原N01–N16；另恢复原错误标签可得到恰好两错；逐行method+handler+arm-kind比较；保持方法总数、乃至各handler数量都不变的交换仍应拒绝。不以带两错的原基线让所有负例自动失败。

B02/B03假件的PASS是“如预期检出原问题/合同oracle有效”，不是关闭生产缺陷。冻结新对象后须定向复核修订有效表、相邻调用、真实依赖以及源算法/模型分层；不得仅运行该checker代替语义复核。

## 5. 原仪器与IX01

B已有tools/gogoke-p00-census/census.py和test_census.py，原14项自测与旧census不被本轮改写；这是原A时的未运行记录；R2的实际重放与固定源码身份在新验证记录中单列，不能回填成旧A已经运行。源码反例仍须重放：cfg(test)只修饰其项，不得截断后续生产代码；daemon git常量arms；clipboard alias；dynamic import；cfg_attr/path/include；脚本动态program。旧census是种子，不是闭包结论。

上轮IX01是新Python校验脚本上传被工具安全检查阻断。该脚本仍未发布，本轮没有换编码、换路径、嵌入Markdown或其他通道上传它，也不拿其旧12案例结果作证。当前候选的重放依据是已发布的被动数据、固定源对象、明确集合/负对照及独立源码重建，不依赖某个未提交脚本。安全拦截未被宣称解除。Pass B认为证据/仪器仍不足时必须写INSTRUMENT_INVALID或CHANGES_REQUIRED；不把结构成功提高为P00接受。

## 6. 隔离复核输出

VERDICT: PASS_FOR_DEFECT_CLOSURE | CHANGES_REQUIRED | INSTRUMENT_INVALID
FIXED_SNAPSHOT_COMMIT: 完整SHA
PASS_A_PAYLOAD_COMMIT: 完整SHA
ENTRY_FAMILIES_REBUILT: 自行重建结果
REVERSE_SINK_FAMILIES_REBUILT: 自行重建结果
FINDINGS: 固定path/symbol/条件/失败及影响
UNEXECUTED_RUNTIME_AXES: 逐轴
EXTERNAL_BOUNDARIES: 固定身份/owner/首验
NO_CONTEXT_INHERITED_PASS: true仅在确实隔离时

Pass B报告不是生产施工许可。之后Pass C修缺陷并针对新固定对象复查受影响轴，Pass D才能判断开门；Owner保留最终权威。当前不得合并PR、删除Codex、改生产或现有测试。
