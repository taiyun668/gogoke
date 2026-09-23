# P00 Pass C R2 — B01–B03 证据修订与受影响轴重放

状态：AUTHOR_REPAIR_READY_PENDING_FOCUSED_REVIEW。P00 NOT_ACCEPTED；code-entry gate CLOSED_PENDING_GPT_P00_REVIEW；runtime_verified false；global_orphan_sink_count null；生产工作包[]。

## 固定输入与隔离角色

生产源码B：bc665a852833952b76d9508401193bedd2198436；tree：746024abeb2e6d35ac88fd1b2e315fceb0e9f54f。
原A payload：620b49270f99f18cd9d8d5e7ffa68b76c5f274df；原snapshot：55a8443ef98f1cc231f4f45d78c3bc3cad93a9e4。
本轮接收的B rerun为CHANGES_REQUIRED；完整包SHA256为402d68133e83528c1de614c4583d51b58330687c13f5ec709aa24a6a70f4f91c；冻结包SHA256为685936270e4b4ae45354140c0aebbc05079c63e46abe197944e5b34c58d88507。received/中保留原findings/result/chronology assessment/freeze receipt原字节。完整B ZIP保持在原Library交付，不声称已通过GitHub存储该19MB原包。

这是作者侧C，不是新的隔离B，不签发独立通过，不用本地哈希证明原始平台全量时间线。旧不可证明chronology的PASS_FOR_DEFECT_CLOSURE为NO_VALID_VERDICT，旧PF07 C保留但不作为本次开门链依据。本次从原snapshot分出新证据分支，不移动PR#6的被冻结分支。

## 修订与检验

### B01

直接修改有效surfaces两行：local_usage_snapshot → workspace:literal；menu_set_accelerators → daemon:literal。literal分布daemon6/workspace25/codex39/prompts7；另git常量27。逐行tuple验证，修后基线必须0错误；恢复旧两行得到两项错误。新增跨handler交换负例保持方法总数甚至分组数量不变仍拒绝。原77/104总数没有作为归属正确证明。更新replay、authority计数与发布对象blob/hash。

### B02

F23/P20–23/Q14及reuse-contracts.lifecycle具体承接8s宿主、10s宽限、5s观察和额外身份/kill时间。Room shutdown忽略数值；closeAndRelease初次保125但释放124；Codex/Grok返回后清child/session，Claude在等待前清child；第二次close可返回0绕过原125。持久化失败发生在清引用之后也是独立结果。关闭合同由唯一gogoke ExecutionHost负责，不引回Room总服务。

typed结果区分自然退出、已尝试强杀、拒绝、子已退/后代未知、残留、宿主期限、异常和无句柄但有旧custody。只有精确身份退出、后代处置、旧writer封口、耐久收据共同齐全才可释放home/owner。其余隔离保留，重试/缺句柄/宿主强退都不能默许另一个writer。最早WP03/T32.L,T52.L,T63.L，合同WP01、耐久WP02先行。

### B03

F24/F25、RT26–32、W21–30、Q13/Q15明确dataRoot/seats/seatId、credentialHome、GROK_HOME及AppData，owner/state/transcript/protocol/grok-acp/events/prompts与临时文件。40条相对import和37个持久化调用点绑定固定源；Claude不写独立protocol/state文件，events错误会吞，公开owner的Date.now与close的真实query也不混同。

persistence是mkdir→唯一temp(wx/0600)→write→fsync(file)→close→rename→best-effort fsync(dir)。前序错误留temp；目录fsync失败吞；append读整文件再替换不是跨writer序列化。显式real-home不是固有漏洞，但需要绑定provider/host/canonical-home/account-auth-revision/privacy-domain/generation/effects/expiry的Owner审批收据；不能只信boolean，也不自动迁凭据。

accounts/instances/proc/mask符号分别列明：纯mask可选，accounts/instances存储和认证流程不原样采用；传递导入的proc/原生probe及instance `.tmp`/坏池/ROOM_BOOT_ID语义保留具体登记。Room总server/scheduler与另一Rust kernel/store不采用。metadata迁移保原件和领域，不复制secret/home、root未核验不得开启第二writer。首验WP02/T39.L,T55.L；复用前WP03/T50.L,T51.L,T63.L；隐私WP04/T11.L,T49.L。

### 相邻证据纠错

固定源码显示旧K16把prompt update误写成rename→write，现改为write(next_path)→remove(old_path)。旧K17把Agent delete写成先config后file，现改为先备份/删managed file后persist config，失败恢复；create/update补偿也明确。同步纠正6条surface事实与RT23/W05/W07：Agent read/write接口限managed路径，write并未预解析新role内容；外部引用的列表展示不能推出该接口可读外部内容。原源码、旧R文件和旧payload不变。聚焦复核须包含K16/K17顺序和读写策略，不能只看B01字段变更。

## 仪器范围与失败保留

verify_repair.py是本轮B01–B03专用被动校验，未复用或绕过IX01被拦截脚本。safe_source_fixtures.mjs在假依赖下执行固定TS算法或server精确函数体，真实spawn/FS/home全部由测试替身承接；将FUTURE_CONTRACT_MODEL_ONLY和源算法分层。它检查原源码的缺陷仍存在，并未改变实现。

初版Node stripTypeScriptTypes + SourceTextModule组合连续两次进程退出139，基础Node/VM/type-strip探针均0，具体引擎原因未确定；未当作产品错误。改用本地TypeScript5.8.3纯JS转译和CommonJS VM受限加载后，初次因Claude未传显式fake executable退出1；补齐假输入后45场景完成；进一步补齐host/account/effects/explicit审批维度后，最终49场景完成且输出重复一致。Tauri校验器初版漏了无命名空间的is_mobile_runtime，退出1；修正解析后仍要求129项，不降低预期。四次非零仪器尝试日志/版本hash保留在交付包及FAILURES.json中。没有安装产品依赖、运行生产构建或执行真实provider。

## 新payload、验证与后续

新payload由GitHub返回完整SHA，后继登记提交让START_HERE/candidate/gate/Capsule引用同一对象；不伪造自引用SHA。原A/PR#6不移动，新branch与原snapshot比较只允许docs/design和根交接控制文件。

原303观察/84行为/133方法保全；仅4个相关capability行为增加F23-25与断言；原N1/I/M时限不删。运行详情见verification.json、safe-fixtures.json、replay-results.json、pass-a-validation JSON和SOURCE文件hash。

本轮结论只能为“作者侧修订及受影响轴重放已形成，待独立定向复核”。B01–B03的独立关闭状态仍PENDING；最新B对原payload的CHANGES_REQUIRED不被篡改成PASS。未执行：实际CLI/账号/付费、PID reuse/后代/旧writer、真实锁/junction/损坏/迁移、RPC/event gap/TLS/首连、OS/UI/音频/clipboard、remote/voice、用户Git/PTY/files、安装更新registry签名发布/TestFlight、平台构建性能与包provenance。Pass D未做，Owner未开门。
