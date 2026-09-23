# P00 Pass C R2 独立定向复核报告

## 判定

**CHANGES_REQUIRED**

| 项目 | 本轮定向结论 |
|---|---|
| B01 | 证据缺口可关闭（CLOSED_AS_EVIDENCE） |
| B02 | 证据缺口可关闭（CLOSED_AS_EVIDENCE） |
| B03 | CHANGES_REQUIRED，仅剩 C2-F01 的两处逐符号映射及其直接承接 |
| K16 | 证据缺口可关闭（CLOSED_AS_EVIDENCE） |
| K17 | 证据缺口可关闭（CLOSED_AS_EVIDENCE） |

P00: NOT_ACCEPTED；code-entry gate: CLOSED_PENDING_GPT_P00_REVIEW。
runtime_verified=false；global_orphan_sink_count=null；authorized_production_work_packages=[]。

这里的关闭仅表示指定证据缺口得到承接，不表示生产缺陷已修、运行时通过、能力可安全直接复用或允许合并。本轮没有重新进行整仓盲审，没有扩大旧冻结集合，也没有把作者的检查通过当作本轮独立结论。

## 1. 固定对象与保全

仓库：taiyun668/gogo-party。

| 对象 | 固定身份 |
|---|---|
| 生产源码 commit | `bc665a852833952b76d9508401193bedd2198436` |
| 生产 tree | `746024abeb2e6d35ac88fd1b2e315fceb0e9f54f` |
| 生产 parent | `94254b6c8aac11ce52931a0f882bc89f708557b7` |
| 原 Pass A | `620b49270f99f18cd9d8d5e7ffa68b76c5f274df` |
| 原 snapshot | `55a8443ef98f1cc231f4f45d78c3bc3cad93a9e4` |
| R2 payload | `5d15a11ecd33763df31172dbdd3426f9f740ab0c` |
| payload tree | `5f76373bfbf8588db55a2714ffca46979fac12e4` |
| 新登记 snapshot | `3ed20ef3b84e8b26ac2985337da6c87aae3e7ed2` |
| 登记 tree | `c3e602ffa43049306ea4943b80372a6399e849a4` |
| 原独立冻结归档 SHA-256（前后相同） | `685936270e4b4ae45354140c0aebbc05079c63e46abe197944e5b34c58d88507` |

GitHub 原始 commit 接口分别返回 payload 的 parent=原 snapshot、登记的 parent=新 payload。对离线 source 的 2,806 个 tracked 文件重新比较原始字节与 Git blob，0 个不一致；HEAD/tree/parent、git fsck --full（exit 0）、无 remote 均复核。这里是对象身份检查，不是重新从零审计 2,806 个文件。

从源码 tree 与实际旧/新文档字节独立重建四棵 Git tree；新 payload 和登记 tree 均与 GitHub 返回值一致，不仅仅相信作者 SHA 列表。原 snapshot 到新登记 26 个变更路径；生产基线到新登记 39 个路径，全部是 docs/design/ 或根交接文档。25 个发布对象逐字节验证，未循环包含的 index/verification 也通过整体 tree 身份检查，不靠自引用摘要。

原冻结归档只读保留，当前 received/POST_FREEZE_FINDINGS.json、FREEZE_RECEIPT.json、CHRONOLOGY_VALIDATION.json 与本会话原交付逐字节一致。只读摘取旧 IF01/IF10/IF11/IF12/IF16、E17/E31、AU05/AU07 作来源锚点，见 results/RETAINED_FREEZE_LINKS.json。本轮不重新认证旧时间线，不改写任何旧发现或 verdict；本轮新发现仅存于 DELTA_FINDINGS.json。

原离线 ZIP SHA-256：`cbbe3204755c6066fc0f3a07d9bcab1ef7283c56def81f1e22f2023e844f5f0e`。
R2 Library 交付 ZIP SHA-256：`e3a5d24dc779f6b748739001dd6f0c738a55e29bc2c7bfa085d1b0d93a8620a4`。包内校验和已验证，payload/registration 字节又经过独立 Git tree 对账，因此不是把 Library 文件名当作 Git 对象身份。

## 2. 阅读次序与独立性范围

先通过已连接 GitHub 完整读取新登记 START_HERE 与 Capsule；再读取固定 payload REPORT 与 received findings。初次 findings 工具显示被截断，之后从已验 Git 身份的交付包读完整原件。然后阅读有效 surfaces、authorities §10、capabilities、corrigenda、replay、reuse-contracts、source-identities、FAILURES、两份脚本及其依赖边界，再执行。

本轮允许读作者增补，是明确的定向增量复核，不称为新的上下文隔离盲审。两份原脚本没有改写；其输出中的 AUTHOR_PASS_C 标签原样保留，独立复核身份通过本报告、运行收据和额外独立检查区别。未使用旧结果替代新对象验证。

CHRONOLOGY_RECEIPT.jsonl 是本轮追加式本地记录，首条明确说明它在最初几次 GitHub 读取之后创建；这些读取的原始顺序在对话工具记录中，未回填伪造早期 UTC。收据不是第三方签名时间戳或平台全量原始日志导出，也不用于重认证原 Pass B chronology。

## 3. B01：证据缺口可关闭

固定源码 `rpc/workspace.rs:266` 为 local_usage_snapshot，`rpc/daemon.rs:18` 为 menu_set_accelerators；新 surfaces 分别登记 workspace:literal 和 daemon:literal，正确。

本轮独立编写的受限 Rust token 扫描器从 dispatcher 实际五模块读出顶层 match-method arms，解析 Git METHOD 常量，逐 method/handler/arm-kind 对比。结果 104 方法，literal 分布 daemon 6、workspace 25、codex 39、prompts 7，另 git constant 27。新候选 0 差异。原两行错误各被拒绝；交换两行标签、同时保持方法总数和每个分组数量不变，也得到两项拒绝。没有以 77/104 总数证明归属正确。

这不是一般 Rust AST 或全函数可达性证明；范围就是固定源码的五个实际 method dispatch 模块。独立脚本没有导入作者 verify_repair.py。

## 4. B02：证据缺口可关闭，原生命周期缺陷未修

核对 `server.ts:2800-2834,4900-4938`、`close.ts:25-47,95-171,198-274` 和三 provider 的实际 close 后，R2 lifecycle/F23/P20–P23/Q14 已承接：

- 宿主总期限 8 秒、Seat 宽限 10 秒、后续 5 秒不是 15 秒总墙钟上界；还包括 startTicks 等待、身份查询、kill 命令时间，already-exited 但 exited Promise 持续 pending 的分支可经历两次后续等待。
- shutdown 忽略数值 close 结果并删除/记录关闭；closeAndRelease 首次保留 125，却释放 124。Codex/Grok 在返回后清 child/session，Claude 在 await 前清 child；二次 close、缺句柄都可能返回 0。回写失败又可发生于清引用之后。
- 新公共合同由唯一 gogoke ExecutionHost 负责，区分自然退出、强杀尝试、拒绝、后代未知、残留未知、宿主期限、异常与无句柄但仍有旧记录。只有精确身份的退出、后代处置、旧 writer 排空以及耐久收据均满足，才允许 owner/home 再分配。WP01 typed contract、WP02 耐久保管、WP03/T32.L/T52.L/T63.L 首验保持，不导入 Room 总服务。

本轮除重放原假件，还独立提取精确方法体补测缺席位仍有旧 owner、调用异常、124/125、三 provider 的 helper 异常/持久化异常/缺句柄。测试观察到的是固定源码当前行为，而非安全合同已经实现。真实进程、后代、跨平台强杀和 durable custody 均未执行。

## 5. B03：主体补证成立，但 C2-F01 仍需修正

### 已得到具体承接的部分

dataRoot 与 credentialHome/GROK_HOME/AppData、Seat owner/state/transcript/protocol/events/prompts 分开登记；Claude 不写独立 state/protocol 的差别未抹平。persistence 的 mkdir→唯一临时文件 wx/0600→write→fsync(file)→close→rename→best-effort fsync(dir) 顺序与错误留临时文件、目录 fsync 错误吞掉、append 为整文件读改写而无跨 writer 序列化的界限均明确。

本轮使用 TypeScript 5.8.3 AST，而不是作者的正则，独立确认登记的 40 个相对 import/export 边与 37 处三 provider 中指定 helper 的实际调用一致；边中含 type-only/re-export，它们不是 40 个运行时加载，也不是全仓传递调用图完成的证明。

显式 real-home 例外要求 provider/host/canonical-home/account-auth-revision/privacy-domain/generation/effects/expiry 的 Owner 收据，而不是仅布尔值；metadata 迁移不自动复制凭据或 Home。accounts/instances store、Room 总服务和另一 Rust kernel/store 的不整体采用政策保留。上述主体证据不因以下错误而全部推倒重来。

### C2-F01：两处具体符号无法与固定源码连接

| 新 payload 登记 | 固定源码实际对象 |
|---|---|
| reuse-contracts.json:76，instances.ts 的 confirmOwner | `InstanceStore.noteProcess`，instances.ts:325-338；没有 confirmOwner 声明 |
| reuse-contracts.json:97，server.ts 的 startSeat | `ensureSeat` / `ensureSeatInner`，server.ts:1488、1498；没有 startSeat 声明 |

实际相邻链为 `ensureSeat → ensureSeatInner → seat.start → instanceStore.noteProcess → save`，`ensureSeat` 的 catch 又会调用 releaseAllFor。noteProcess 在无 PID、seat binding 不匹配或身份查询返回空时直接返回；query 返回空也可能是查询错误，不是已退出证据。save 可以在内存 busy 身份已更新后失败。这些不是一个可凭名称认为存在的“confirm owner”操作。

独立 AST 原始扫描产生 5 个名称候选。人工语义判定后仅上述 2 个具体 camelCase 名称列为缺陷；demo 的 run、accounts 的 login/logout 可作行为概称，不因没有同名函数自动升级为漏洞。建议将 operation_label 和 source_symbols 分开，避免这种混用。两处限定错误见 results/SYMBOL_ADJUDICATION.json；内存假想改名的正对照只验证名称可解析，既未改候选，也不是接受一个尚未提交的新 payload。

`verify_repair.py` 本轮 exit 0 不否定此发现：它核对 source hash、40 imports、37 helper 调用及部分 schema，却不检查 selection_matrix 中每个具体源码符号。因此不存在“作者脚本 PASS → 逐符号证明 PASS”的推导。独立 AST 检查 exit 2 是检出证据问题，不是 TypeScript 解析失败（解析错误 0）。

### 所需最小修订

修正上述两行或提供明确的语义标签→真实符号映射，绑定实际文件/行号/blob；把 noteProcess 的身份未知、回写失败与 ensureSeat catch/release 的区别接到已有 owner/writer 合同，不要求采用或修复整个 Room。给逐符号检查增加负例：正确基线 0，恢复不存在的名字或注入不存在名字必须拒绝，即使 40/37 等计数不变。更新受影响发布对象摘要并冻结新 payload。

下一轮只需定向复核 B03 此源符号/调用方/回写轴、受影响对象摘要和关闭门禁；未变的 B01/B02/K16/K17 不必重开，原独立冻结集合不必重做。

## 6. K16/K17 相邻纠正

K16：`prompts_update_core` 在 406 行先 write(next_path)，408 行才 remove(old target)。删除失败可以留下双文件，不是 rename→write。`move_file` 的 rename→EXDEV copy/remove 是另外的函数，跨设备检测在固定源码中有 Unix cfg 限定。新 K16 和 prompts_update surface 已同步，证据缺口可关闭；没有执行真实磁盘/故障/跨平台测试。

K17：create 先写 role，再 persist global config，失败时尝试删除新文件；update 的 rename/写入/回写配置有不同补偿分支，补偿可失败；delete 在要求删 managed 文件时先备份并删除该文件，再持久化全局 config 删除，失败尝试恢复且可能报告复合失败。内存 document 的先行删除不等于配置已写盘。直接 read/write 接口只接受 managed path，并非列表显示外部 config_file 就能由该接口读外部内容；直接 write supplied text 不预解析新 TOML，不能与另一个 upsert 解析旧文档的逻辑混为一谈。

本轮独立字节/表格差异检查确认：133 surface 记录中仅 8 行改变＝B01 两个归属字段＋6 条上述相邻事实。303 原观察对应的 6 份 traceability TSV 逐字节不变。84 个能力行为 ID、主责与首验检查全部保持；仅 C14.2/C33.2/C37.1/C42.2 的入口及断言扩充，没有删原义务。

## 7. 实际执行、退出码与证据层次

工作目录 `<workdir>/p00_pass_c_r2_review`；源码根 `offline-source/p00-offline-source`；候选根 `r2-package/payload`。完整命令/argv/cwd/UTC/退出码/原始 stdout、stderr 摘要在 ACTUAL_COMMANDS.jsonl、ACTUAL_COMMANDS.tsv 和 logs/。

```sh
python r2-package/payload/docs/design/p00-pass-c-b01-b03-r2/verify_repair.py \
  --source offline-source/p00-offline-source \
  --candidate r2-package/payload \
  --out results/verify-repair-independent-run.json
# exit 0；23 绑定源码、104 tuple、25 发布对象、19 metadata 负例、2 graph 负例。

node r2-package/payload/docs/design/p00-pass-c-b01-b03-r2/safe_source_fixtures.mjs \
  offline-source/p00-offline-source
# exit 0；49 场景。原始 stdout 单独保留，未把作者保存的结果当本轮运行。

python scripts/independent_source_delta.py
# exit 0；独立 token 对账与原/新账本差异、保留义务核对。

node scripts/independent_ts_review.mjs offline-source/p00-offline-source \
  r2-package/payload results/INDEPENDENT_TS_AST.json
# exit 2；40 imports /37 helper calls 相符、TS 解析错误0，具体源符号登记失败。

python scripts/adjudicate_symbols.py
# exit 0；原始5候选中确认2条错误，3个行为泛称不升级；不是 B03 通过。

node scripts/independent_targeted_fixtures.mjs offline-source/p00-offline-source \
  results/INDEPENDENT_TARGETED_FIXTURES.json
# exit 0；17 个独立精确方法体+假依赖场景，0 个新增未来模型场景。
```

实际环境：Python 3.13.5、Node v22.16.0、TypeScript 5.8.3（预安装）。未安装产品依赖。

| 层次 | 本轮结果与限制 |
|---|---|
| 静态源码/对象/账本 | Git tree/字节、tuple、AST、差异及相邻语义核对；不等于运行时或全调用图证明 |
| 原固定源码假依赖测试 | 49 中的 29 项执行固定算法/提取调用方，OS、FS、child、时钟等由假件承接；PASS 可表示原缺陷被正确观察 |
| 未来合同模型 | 49 中的 20 项仅测试 release/approval 等拟定条件模型，不能证明生产实现、真实路径canonical化或审批执行 |
| 本轮新增独立假件 | 17 项均精确源方法体+审计者自写假依赖；与原29有重叠，不相加冒充66项产品验收 |
| 真实生产运行 | 未执行；runtime_verified=false |

保留非零运行：下载固定对象的容器 DNS 失败 exit1，原冻结目录查看初稿括号语法错误 exit1（修正命令 exit0），独立 AST 发现证据错误 exit2。GitHub artifacts collection URL 和 container.download 路由尝试被工具拒绝，也未当作内容成功获取。随后走 Library 原件并完成 Git tree 验证，没有绕过连接授权。作者历史四次非零仪器尝试仍在原交付 FAILURES.json/日志中，不当成本轮产品失败，也不删除失败记录。

本轮没有重跑原 Stage1 14 项、census 或产品测试套件，不把作者保存的重放结果登记成独立执行。此次受影响检查并不需要再次扫描全仓入口家族。

## 8. 未执行轴、PR 与移交

真实进程/PID/后代/旧writer；真实锁、junction/别名、损坏/迁移与磁盘并发；真实登录/账号/provider/付费与审批；daemon/listener/remote/TLS/RPC重试/gap；真实UI/权限/隐私/clipboard/音频/麦克风；用户Git/PTY/文件/生成器；产品构建/no-Codex平台矩阵/安装更新/registry/readiness/rollback/签名/发布/TestFlight 均未执行。逐项见 UNEXECUTED_RUNTIME_AXES.json。

最后只读查询 PR #6：open、merged=false，HEAD=`55a8443ef98f1cc231f4f45d78c3bc3cad93a9e4`，base=`bc665a852833952b76d9508401193bedd2198436`。PR metadata 报告 changed_files=26；本轮未再次获取 PR changed-file 名单或 patch，故不把 metadata 计数冒充路径级复查。新修订路径范围来自上述固定 Git tree 的独立比较。未查 comments/reviews/checks，未写 PR、未推送、未合并。PR body 随 get_pr_info 返回，但未用于本轮定向判定。

**下一步：Pass C 仅修 C2-F01，冻结新对象后复核受影响 B03 轴。全部指定证据关闭后，再交 Pass D 与 Owner 独立决定代码入口。当前不得因 B01/B02/K16/K17 的证据关闭而开门。**

## 附录：本轮关键源码 Git blob

完整行号、commit、SHA-256、摘录在 SOURCE_EVIDENCE.json。

| 路径 | Git blob |
|---|---|
| `apps/desktop/src-tauri/src/bin/gogoke_daemon/rpc/workspace.rs` | `3f18527f7bd01bd8ace7c9e6c2c82e4a87a36502` |
| `apps/desktop/src-tauri/src/bin/gogoke_daemon/rpc/daemon.rs` | `e491f0372edda68f96b7e55b8c845d406792f693` |
| `packages/seat-runtime/src/close.ts` | `0c2c1b58aa80549fed849fa8fede35dda6d5d5a4` |
| `packages/room/src/server.ts` | `0961b392a5a7d6e64fa72a3a6898f1dba5c39259` |
| `packages/seat-runtime/src/seat-runtime.ts` | `220c036e96f3aee911ced48f2f6ea4e09b5681fc` |
| `packages/seat-runtime/src/claude-seat.ts` | `c77a8d587b56ddcb7a58377d3b0e1bd7ba403d51` |
| `packages/seat-runtime/src/grok-acp-seat.ts` | `198776699f2bec08bd56885c849ad907cd9e608a` |
| `packages/seat-runtime/src/persistence.ts` | `78023a1a169ba63ec6e3296ced315092f6fd9622` |
| `packages/room/src/instances.ts` | `3133e61ed856225595e9c7888eeb9db4a74ccb56` |
| `packages/room/src/proc.ts` | `b7ebc38fbadcd7b0c20d655255c4e8e4ebff1ce6` |
| `apps/desktop/src-tauri/src/shared/prompts_core.rs` | `a03844b6f4e3b1a481c786e6429375b546e9a436` |
| `apps/desktop/src-tauri/src/shared/agents_config_core.rs` | `3f5b8889e442a9e6ba763b28836deabe27137b25` |
| `docs/design/p00-pass-c-b01-b03-r2/reuse-contracts.json` | `d7ff9f8cc9dd8fec28e422090cf25a4c565a9bbe` |

报告生成 UTC：2026-09-19T04:11:12.826525+00:00。报告 SHA-256 置于 REVIEW_RECEIPT.json，不作自引用。
