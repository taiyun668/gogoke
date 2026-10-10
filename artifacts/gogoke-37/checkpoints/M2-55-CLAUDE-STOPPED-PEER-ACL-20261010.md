# M2：55 原实例恢复拒绝与正常停止历史 ACL 修正

## 改了什么

55 用云端冻结原字节完成普通 Interactive/Limited 安装；未重新打包、未公开发布。原 Claude 实例读取在新 H 或模型请求之前拒绝。产品正常退出零，正式文件、正式数据、快捷方式、登记及既有记忆保护核对通过；原失败、custody、episode、RPC 和空 StopFact 保留。

直接原库与实际 ACL 读回：Claude recovery journal 为零；复用的实例 HOME 有九个会话 package SID，分别对应两条未确认旧 H 和七条经真实 StopFact 正常停止的历史 H，没有陌生 package SID。原生启动会在复用 HOME 添加当前 SID，普通停止保留其 ACL；恢复却排除正常停止的 H，再把其仍在的 ACL 当作陌生身份拒绝。这是恢复前提与实际生命周期冲突。

修正仅将同实例、精确原 open/stop 请求、claim/episode/custody 三方一致非空 StopFact 的正常已释放历史 SID 列为保留 peer；它们不进入本次清退目标。封存其成员集合与精确来源摘要，在待处理恢复、H 释放事务、完成回执读回时重新资格化并比对封存时的成员，允许后来合法成员增加。陌生 SID、SID 复用、封存成员缺失或来源变化仍拒绝；底层仍只删除已确认消失目标的 ACE，保留其他 ACE 的原字节与顺序。不扩大权限，不造 STOPPED，不读凭据。

下一实验候选号为 0.1.56，含上述真实产品修正。G 目录未改。

## 结果

- SAC 注册表实际读回为 0；已精确读回并沿用 PR #83 合入 main 的本机开发规则。原生编译、测试根、缓存和临时产物直接 D 盘，并行上限八；开发产物不进候选，不作正式证据。
- 同一新边界回归在未修产品模块上实际失败：`Claude disappeared holder: AclWitnessMismatch`，耗时 43.20 秒。修正后同一回归通过，另三项本机诊断通过：陌生 package SID 拒绝、封存后历史 stop 字节漂移拒绝、原 UNKNOWN open 及原记录保全。
- 随后核逻辑发现第一次来源摘要覆盖了全体历史；后来新 H 正常停止新增合法 peer，会误拒旧完成回执。Controller 与独立 Sol 确认前提后取消尚未签名、未安装的在途构建与原生检查，保留原提交和运行；没有把旧局部诊断通过冒充生命周期完整。新增真实后续 open/stop/release 回归在该版本末段实际报 `Claude prior disappeared holder release unverified`，耗时 53.85 秒。按封存成员子集重验后，本机五项诊断全部通过；同一 F 的物理路径/身份前后不变，后续 StopFact 由原生产者产生。
- 历史停止由真实固定 Claude 子进程和产品 stop/release 生产者产生；ACL、进程身份、E/F/H 来自真实生产者。登录 presence 和后续 UNKNOWN/PREPARED 持久形状明确是夹具投影，不是认证、模型超时或已安装原实例恢复成功。
- 独立 Sol 对最终封存子集修正聚焦静态复核未见阻塞；最终受影响正式云测待执行，已取消的先前运行不算通过。正常 APPLIED open 加 StopFact 证明原身份及停止事实，单独不证明握手成功。
- 本次正常退出、安装、构建和本机诊断均不计为新增真机可运行能力；同一次心跳计数仍为二。完整 M2/M3 和稳定链未通过，未自行验收。
- 两项新诊断任务结束后，按确切 action、主体、脚本摘要与实际退出码保全 XML/log，注销并逐项回读不存在。独立组件的真实 Git cwd 已收到 ACK，但空 HOME 结果不能外推原已登录 H。

## 失败原文与仪器

55 原产品：`GOGOKE_DESIGN37_NATIVE_USER_OPERATION_FAILED:ERR V37StoreFailure("Claude disappeared holder: AclWitnessMismatch")`。全部原件保留，不重发失败请求，不重新登录，不复制数据库或实例 HOME。

实际安装前的路径、证据目录和可选字段测量失败均在安装器或产品启动之前；原件保留，修正测量后重跑。一次元数据仪器报告 `Readonly DB observer source drift`，未保存比较值，不推定其原因；确认无产品进程、WAL 与 rollback journal 为空后，沿用现有关闭后只读观察法，原库字节与前后元数据核对通过。

恢复源码的文件复制保留了旧时间戳，Cargo 一次复用了负控二进制；尚未将它运行成修正结果。保留该缓存日志，重新写入同一修正字节触发真实编译，确认修正二进制与负控不同后才执行四项诊断。没有把缓存替身当修正代码通过。

后续集合增长回归第一次因独立开发临时路径过长，在真实 Git 建树阶段报 `fatal: '$GIT_DIR' too big`，未测到目标生命周期，不能算负控。原件保留，只改 D 盘开发临时根后，同一负控二进制在目标回执核对处失败；修正后的五项诊断使用同一短临时根。原安装、原实例和原数据目录均没有移动或复制。

独立组件夹具与私有构建目录的递归清理被自动命令审批拒绝，已保留在 D 盘；不换命令绕过。旧候选的确认空钉住根继续保留待审。

## 下一步与参照

先提交正式修正并跑受影响云端检查，再冻结下一实验候选，走原字节验签、实际安装和原实例复现。已并行准备 Grok 的新真实权限回包读回，只拒绝当前新请求，不重放旧 cursor；宿主拒绝写入不能冒充 OS ACL 拒绝，OS ACL 完整轴仍 NOT_RUN。继续主动读取 PR #54 的 Claude 交付。

参照现有 `session_transport/launch.rs` 的会话 SID 派生与绑定树 ACL 添加、`v37_runtime.rs` 的真实 stop/StopFact、`claude_retirement.rs` 的逐对象原字节删除，以及既有双 H/UNKNOWN 恢复组合；沿用普通候选安装、正式五组保护和产品 CDP 硬断言链。历史 gogo-party 的生命周期与 root/home 约定作为已有调研背景，本修法采用当前原生生产者的精确事实，没有新建补偿等待或重试机制。稳定点实测与验收前先提醒 Owner 重新开启 SAC，并实际读回 1。

## 同源云测与换装准备回读

最终修正的组合受影响云检实际通过二十项：Grok 权限六项、Claude holder 十一项、固定 Claude initialize 一项、原 EOF 与退出源各一项。原始运行、归档摘要、成员字节、源码提交及逐项日志已回读；卫生通过。第一次只选 holder 的小检查在四个新增回归建真实 Git 工作树前报 `cloud must bind actual installed Git backend`，原运行和失败产物保留，改用已有的组合检查绑定真实后端，源码不变。全量库与稳定门槛仍 NOT_RUN。

实际普通视图重拍 55 基线：十个正式保护字段与原产品正常关闭后的记录完全相等，原七条 UNKNOWN/null StopFact 逐成员保留；数据库、WAL、SHM 与 journal 与实际 55 安装后读回字节连续一致。独立 Sol 对闭合来源、保护比较和确切候选卸载/安装目标聚焦只读复核无阻塞；未执行卸载或安装，缺冻结与签名输入时会拒绝。

测量脚本两处误读不存在的 provider `recoveryJournal` 字段，错误原文为 `PropertyNotFoundException: The property 'recoveryJournal' cannot be found on this object.`；原任务、原脚本和失败日志保留，改为哈希绑定原库直接元数据的实际 recovery 表计数，未从缺字段推零。失败和已完成的自建任务按确切 action/主体/脚本字节保全后注销并逐项回读不存在。

另一 Sol 机械比对原 53 与已通过空 HOME/Git 组件：二十个启动参数均已带 `--debug-file`，只有会话路径值不同；未据缺失日志断言原因，未重编或重复探针。原实例与空 HOME 的差异尚未解决，不能把组件通过写成原模型通过。

Claude 原实例冷恢复/新一次真实 USER turn，以及 Grok 新一次真实权限回包的独立脚本已并行备好；签名、实际安装和仪器 pin 缺值即拒绝。仍保留所有旧失败、未确认停止和凭据；无新登录、无重发原请求、无补造 StopFact。下一步接最终冻结原字节和签名回读，实验安装后复现原实例。当前仍无新增已安装可运行能力，心跳计数不重置；稳定点开启 SAC 的 Owner 触点保持。

## 56 实际回读与新增能力

最终冻结字节的公开资源签名自动验签、产品成员逐字节一致性已通过。55 正常退装与 56 普通视图实际安装均退出零；正式文件、数据、快捷方式、登记与记忆/账本保护读回通过。仅中间实验链，全量原生库、双路构建、稳定安装烟测与验收仍 NOT_RUN。

原 Claude 实例在 56 冷启动后已解除旧持有者阻塞，不需要 Owner 重新登录或复制 HOME。关闭后的原库只读回读以旧 process operation、原 episode open request 精确绑定：消失持有者释放 journal 为 APPLIED、revision 2、原错误为空；数据库及 sidecar 前后字节相同。旧 UNKNOWN 和 null StopFact 保留，没有伪造停止。随后新 H 收到真实 Claude 回复、真实正常 stop/StopFact 与 admission-release，产品正常退出零，正式保护通过。这项“原实例自行恢复并可重新运行”是新增真机可运行能力，不能外推完整 M2。

原一次问答要求照抄测试标记，Claude 回复将其视为注入探测并拒绝，不能写成模型未响应或问答测试通过。原失败记录保留；关闭后读回器还报 WAL 比较变化，但没有保存比较值，不能推定原因。已用现有关闭后、空 WAL/journal 的 immutable 原库读法补核恢复事实；不重放旧请求。新测量改为一次普通算术题，独立核原 H 写入与同会话结果，尚未执行。

Grok 新脚本在产品启动前把真实安装路径与字面量环境变量比较而拒绝，原任务与空证据目录保全，修测量条件后才执行；没有原权限请求重放。独立审计一度把 H generation 与 E 授权 generation 的比较判为误拒，随后核到 admission 会递增 E generation、撤回发现；没有据错误前提改产品。并行席位继续核算术题读回和 Grok 单次权限回包，G 目录未改。

参照沿用已有 `m2-provider-capture-readback.py` 的关闭后原库读法、原生 H 写入/停止生产者及固定 Claude 会话结果；不是凭替身或 CLI user echo 猜成功。源码当前明确追加 `--replay-user-messages`，此前空 HOME 组件参数比较不能推出已安装新会话没有该参数，本次不改 CLI 或其启动参数。

## 普通问答与 Grok 新拒绝原文

56 原 Claude 实例的新一次普通算术问答已完成：题面 `What is 241 + 537?`，实际回答 `241 + 537 = **778**`；原请求最初 UNKNOWN，随后原宿主回执完成，同物理 H 的确认写入、assistant 原文、唯一初始化会话与成功 Result、真实 StopFact/RELEASED 均在关闭后的原库核对。产品正常退出零，数据库及 sidecar 观察前后字节相同，正式/记忆/账本保护通过。两条未处理原帧仍如实保留，单条普通问答不算完整厂商矩阵。实际原帧已导入私有协议样本：入站六、出站二、一个归一化输出，标为 REVIEW_REQUIRED / NOT_ASSESSED，不冒充验收。

将上述已实测的测量修正固化回现有 provider E2E：Claude 普通题与原 H 写入/同会话结果、空 WAL/journal 和正常退出的只读核对；其他两家原判据和旧失败补读保留。独立工人在独占分支交付，Controller 集成；本机签名 Node 语法、Python AST、卫生以及云端 Browser 检查通过。不为测试脚本改动重签、换装或跑全量原生库。

Grok 新请求的原回执为 FAILED，直接厂商终态 `cancelled`，原分类 `PermissionRejected`，工具 `search_replace`，原原因 `User rejected the execution`；工具归一化原状态为 failed。已实际正常 H stop/release、产品退出零并通过正式保护。旧脚本误要求成功 end_turn，原 FAIL 保留；不重发请求。当前 adapter 对严格 Write 才能核准 F 写入，SearchReplace 属未核准形状；不能把这次拒绝写成“已验证 F 写入资格拒绝”或 OS ACL 拒绝。正在关闭后补核原请求、一次 WRITTEN 拒绝回包与目标未写事实，同时准备注册 F 的严格 Write 正向。

补读仪器两项错误在 SQL 之前暴露：漏 import base64，以及把真实 productExits 列表按对象读取；原错误和确切完成任务已保全。已成批核失败路径缺少 hReceipt/toolCandidates、turns 位于 session、写时 E 与释放后 E 字段的区别，不用缺字段补造事实。

并行推进 V10 新用例准备与秘书长 E.3 生产接缝。秘书长已证实 E 返回任务结果，但 G Routine 只接有日期的 lastRun，共享投影据此拒绝已有无日期结果；已交 Claude 修最小读模型，Root 不改 G。现有 TS coordinator 生产工厂不可用；实际 authority pump 存在，但定时 occurrence 原语尚无生产调用。后续接线必须复用同一 H 准入、同事务准备与原回执，UNKNOWN 不重发，不把 primitive 或静态代码写成已运行。

## Grok 原拒绝链已直接核对

关闭后的原库补读实际通过：同一新 USER/H 原请求、精确 typed permission ID、一次 WRITTEN reject-once 回包和原 raw source 的 NO_EVENT 原因均一致；厂商原终态仍为 FAILED/cancelled/PermissionRejected，原工具 SearchReplace 报用户拒绝，目标前后未出现。真实 StopFact、RELEASED、产品正常退出零和正式保护通过；原 FAIL、旧未回包请求和 UNKNOWN 记录完整保留。此结论仅为未支持形状拒绝、无写入，不是 F 写入资格或 OS ACL 通过。

测量修正来自直接生产者：permission request 的持久状态是 NO_EVENT；归一化事件 cursor 与 raw cursor 是两个序号，以唯一 event 和 `_meta.rawSourceCursor` 联结；permission 摘要绑定原 H USER request，不是其 ACP 编码；写时 E 来自 reserve 后的 BUSY 快照，H claim revision 来自原 open 和 send，不能使用更早的 commit 或释放后的值。所有先前只读失败及自建任务原件保留，完成任务核确切主体/action/脚本后逐项注销并回读。

注册 F 的原生 FileIdInfo、原 E/H 关系及实际候选字节也已只读核对，数据库及 sidecar 前后字节相同。原生 F tier 存储为 `NetworkedWrite`，USER 字段为 `NETWORKED_WRITE`；先前仪器混用拼法造成误拒，未据此改产品。新正向 Write 请求准备沿用同一已注册 F、固定 CLI、模型、权限档位和原实例，不重放原负例；尚待实测，不能计通过。

并行已开始：V06 剩余端到端修正在独立工人分支，先前无注册工具的会话不能要求出现一条被拒工具原帧；缺尝试仍记 NOT_RUN。V10 准备使用两个新测试作用域、真实 USER 与模型派生 LEAD，以及三棵原生已注册 F；incarnation、generation、StopFact 和 F 身份只从宿主回读，不照抄历史标识。秘书长 E.3 的 due 原语仍缺生产调用，当前授权未列其必要的 ProductDatabase 组合路径；仅完成最小范围提案，未越权改产品或回执。G 已有无日期任务结果的读模型问题继续交 Claude。

SAC 本次重新读回为 0；所有新临时与证据直接 D 盘。正式保护保持，完整 M2/M3 和稳定链仍 NOT_RUN；稳定点实测与验收前须先提醒 Owner 开启 SAC，再读回 1。

## 注册 F 的 Grok Write 与新授权读回

- 已完成：已装 0.1.56 在原 Grok 测试实例、真实注册 F 中完成一次严格 Write。原 H journal、typed allow-once、REGISTERED_F_WRITE 资格、唯一完成调用、目标确切内容、真实 stop/release、正常产品退出及正式保护均直接读回；只读测量保持原数据库字节。真实协议导出并经既有工具生成黄金样本，尚待协议分类审阅。此结论仅覆盖该正向用例，不外推 OS ACL、完整隔离矩阵或 M2 验收。
- 已完成：Owner 合并 PR #84 后，精确读回 main 回执、MANIFEST、可信校验器、摘要与三处固定回执值。摘要 `3a3d6651f8a5ca2b82f712951dc7a1c74ff593bcf8bde95e6e6604ba3f467a78`、回执 blob `5a55255cf5e28593f9df89bc27d65c93080088f7` 一致；合入施工分支无产品差异。秘书长组合路径现已授权。
- 原失败：V10 新绑定任务在任何 USER 操作/模型请求前因 `ERR_MODULE_NOT_FOUND: Cannot find package '@e2e-dev/web'` 失败，产品正常关闭，原日志/快照/输入均保留，精确任务清理并回读不存在。原因是临时目录内复制的 CDP helper 相对加载依赖；改为已有 Root 原 helper，加载检查通过后启动新例，不重放旧请求、不出新候选。
- 并行：Sol 在独立分支 `codex/g37-secretary-occurrence-20261010` 实施 E 到期与 H journal 同事务、单次物理写许可及原 H 回执结算；本机编译仅开发诊断。Root 接既有权威循环与单一 ProductDatabase，G 缺时间戳结果的读模型问题继续由 Claude 处理。
- 下一步：V10 新例直接读回与剩余 M2；秘书长生产接线完成后跑受影响云检。完整稳定链、SAC 强制状态运行和验收未执行。
- 参照：沿用已在真实 Grok/Claude 流程使用的 `tools/e2e/product-cdp.mjs` 与黄金样本工具；沿用原 E routines、H stdin/RPC journal、原单线程权威循环。未另造测量框架、scheduler 或权限；细节由 Controller 决定，因为既有授权覆盖且不跨用户边界。
## 秘书长生产接线与聚焦修复

- 已实施：在授权的 ProductDatabase/main 原权威循环组合 E 到期与原 H，同事务只发一张消费型许可；Codex 沿已有字符串 RPC ID、原 RPC journal/A capture，Claude/ACP 沿原 pending 与完成路径。未造 USER proof、StopFact 或第二调度服务。
- 首次全轴 Astra 核出的五处问题已按直接代码逻辑修：下一次到期不再关联已结算旧回执；H 终态与下一次日程解析解耦，解析错误由 E 单独持久并向认证 USER 原读回组合；厂商错误原文和截断事实保留；单次物理/已关联厂商拒绝保持 UNKNOWN、原原因且不重发，来源/数据库损坏仍拒绝；Codex 原 UNKNOWN 转换先记录 custody。全新 Sol 聚焦复核另发现未准备 H 前退出进程影响整个宿主，现保留实际 dueBlockedReason 并等待原资格，原 E 状态不变，静态复核通过。
- 本机检查只作开发诊断。Root 合入时的 JSON 比较、闭包生命周期、新增字段析构遗漏由编译原错误定位并修；组合 `cargo check` 最后退出零。工人五项同库安全测试本机通过，仅合成持久形状，不等于真实物理进程或正式云检。现有秘书长云检查已增精确 filter，完整稳定库/候选链未跑。
- V10 原新例：测试 profile 首次 USER 配置已成功读回、两域两 USER/LEAD 建立；未启动模型。其 F create 被拒绝是仪器把 wire 小写 `single` 错改成内部枚举 `SINGLE`，Root 接受这一错误前提是本次返工原因；实际 USER 解析器与冻结源一致，只接受小写。原 DENIED、请求、正常关闭和保护原件保留，不改成通过。下一步从已建立原绑定闭库读回后续 F，不重做首次配置、不重放拒绝请求。
- 边界：当前秘书长接线只支持已持有且空闲的原 SECRETARY H；重启后驻留恢复、无 H 时的缺席持久暂停和完整 V14 仍未完成，不能外推验收。G 结果读模型与新诊断展示继续由 Claude 负责。
- 参照：原 USER 席位/实例/F 解析器，原 NativeSession/RPC/StdinJournal/RawCapture 的写入及错误路径，E 已有 UNKNOWN/暂停/删除语义；迁移仅从原 exact E schema 增独立诊断表，不改变原行、权限或持久写方。以原读取和错误类别替代猜测，未添加重试或等待补偿。
## 秘书长定向云检与 V10 续跑

- 云端发布构建及九个过滤器的 24 项原测试全部通过，随后原变异仪器以 `Original USER marker source locator is not unique` 退出。直接源码证实新增历史核对查询与原查询共享尾部条件，泛化定位已不唯一；改为原带时间的完整 SELECT 查询，仅撤其 source_cursor 比较，保留参数、测试和恢复原件。两个源各唯一定位与 PowerShell 语法已核对，正式 mutation 结果待云端重跑。原失败不算通过。
- V10 已闭库逐项核对已建立的两 USER/lead/profile，计划 F 均未创建，原数据库与旁文件字节不变；续跑使用原绑定、新请求和正确小写 wire，不重新配置或建域。读取任务完成后精确删除并保留 XML。模型原输出支持原 contentItems 形状，不以缺失数组误判原 native 回执。
- 并行：工人提取原 E absence 减资格判断，让 no-live/busy 时也能持久暂停；Root 续跑 V10。独立 specialist 核对驻留恢复与长期账本，旧 UNKNOWN 不伪造 STOPPED。
- 参照：原生产 SQL、现有 USER-marker 变异脚本、原 V06 native contentItems 读回与现有 E presence/policy/CAS；仪器修正不改产品字节或授权。

## 到期恢复、历史原来源与 V10 拒绝核对

- 原定向云检重跑成功：发布构建、24 项测试及两个 USER 来源比较变异均通过；变异各为原测试通过、撤原比较后行为断言失败、恢复后通过。只覆盖该次定向源，不算完整原生库、真机秘书长或稳定链通过。
- E 的到期减资格已独立于活跃 H：无 H 或忙碌时也能用原 presence/policy 判断缺席暂停，不创建 occurrence。E 读取采用原同库快照，借用已有事务或自行打开/结束只读快照；不在外层包住已自行开事务的准入流程。
- A 秘书长历史读取先按同一指定席位/incarnation、NATIVE_V2、原 RESOLVED raw 与原 episode/custody 关联过滤，再分页。独立复核发现初稿漏了原 raw 关联，已补；原历史 episode 不因后来 H generation 更新而消失。仅归一化账本历史，不宣称完整 USER/H 对话或 provider 原帧。
- Root 已接驻留准备：仅沿原指定秘书长、原资源恢复和原 H 准入/打开流程；原 held/busy/unknown 不换进程，不造 USER presence 或 STOPPED。资格不足保留原因，系统/数据库异常仍返回原错误；准备前重新取原时钟。本机检查退出零，仅开发诊断。当前未安装该改动，真机恢复及完整 V14 未运行。
- V10 续例已有两棵真实注册 F、原 H、接管问答和一次原生创建调用；该调用返回 `K-SEAT/create-from-template: DENIED`，子席位未确认创建。真实 H stop/release、正常产品退出零和正式保护通过，原失败不改成通过。直接冻结源码要求已有 policy head；用例没有 policy-initialize，正在关闭后的原库核对，尚不把缺前置条件推定为产品根因。
- 启动仪器已分开保存实际 CDP 连接、ready receipt、原 transport 错误和读取到的页面形状；成功连接后清掉早期连接错误，避免把缺 ready receipt 写成当前端口不通。不增加等待、启动或模型重试。
- 并行：H 工人接原 A 历史到首次 occurrence 的固定请求；V10 仪器工人准备缺策略头的闭库核对；Root 组合与受影响云检继续。G 的无日期结果读模型仍交 Claude，主动查看 PR 后暂无新交付。
- 本机卫生：A 工人的已完成源码工作树保留；其忽略的开发产物清理被工具自动审批拒绝，原对象保留，未改命令绕过。所有新产物与证据在独立开发位置。
- 参照：原 E presence/absence、原 H native admission/recovery/journal、A 原 raw resolution/episode、冻结源 `native_child_create_requires_existing_policy_head` 和已有 CDP readiness；在已授权写域内组合原机制，不新增权限、调度服务或 Owner 触点。下一步核原策略头、完成 H 历史拼接并跑受影响云检，再续真机 M2。

## 历史封存尺寸边界与原策略前置条件

- H 已在原同事务资格之后接 A 历史材料：原 USER 指令单独保留，历史明确标为归一化数据，附原 event/session/source epoch/cursor 和分页事实；旧 occurrence 只按原封存字节结算。全新 Sol 核出首条过大历史可能形成空页无进展，已改为明确尺寸错误、回滚 E/H、无写许可；Root 只将这几种固定尺寸拒绝投影到该任务原因，其他数据库/回滚错误继续传播。原工人八项合成测试本机通过，正式云检待跑。
- V10 原库闭库核对成功：两测试域都没有 policy head，失败 child 与其 F 均不存在；原 source/peer incarnation、两已注册 F、原 NativeV2/H STOPPED/RELEASED 和数据库/旁文件字节一致。E generation/revision 是 stop/release 后的新值，不能拿 pre-H 值要求相等；读取仪器已据真实生命周期改正，旧原件保留。
- 新续例已通过原 USER `policy-initialize`，仅在 source 域创建 OPEN revision 1；不重建 profile、域、USER/lead、两棵 F，不重放 DENIED。新 H 已启动，原生创建及剩余流程仍在跑，尚不能计创建或 V10 通过。
- 原闭库读取任务已按确切主体/action/脚本逐项清理并回读不存在；原 XML、输入、输出和所有失败保留。参照是原 policy head 生产者、原缺头拒绝测试、原同事务 journal 和已验证的关闭后 immutable 元数据读取，不用提示词冒充模型实际参数。

### Owner 合并 #85 后：原策略头只读与 V10 直接证据

- main 上 Owner 合并身份、v2 回执 `e1197ca2`、范围摘要 `593b8be6…`、MANIFEST 及三处固定回执逐项精确读回一致，才施工新增共享路径。原策略头 API 与 USER/TS 只读入口已接：同一已验证数据库的显式事务、当前 Owner；缺行如实返回 ABSENT，其他异常保留原错误，已有 revision/stage 不修改。全新 Sol 聚焦静态复核无阻塞；本机开发编译通过，受影响云检待执行，尚未接通首次项目配置或装入候选。
- 初始阶段尚无生产输入：原设计要求阶段由用户定义，已有 USER 项目登记/席位创建不含 stage。测试案例 OPEN 是案例明确输入，不能变成产品默认；本包不自动初始化、不给默认授权。下一步接真实 USER 初始阶段输入，已有策略保持原样。
- 秘书长原定向云端发布构建、32 项测试和两项真实来源比较变异通过；全量原生库、稳定候选链及真机秘书长仍 NOT_RUN。
- 已装候选 56 的原策略续例实际创建 LEAD、登记原 F，真实 stop/release、正常退出零、闭库 E/F/H 及正式保护读回通过。完整 V10 新例实际运行四个 H 会话并取得原 stopFact，正常退出零；原回读在 PENDING 分支被历史 source 限定拒绝，失败保留，不能写 V10 通过。正在普通视图补正式保护及原帧方法/item/RPC 关联元数据，以直接事实判断测量或产品缺陷，不扩大 PENDING 成功判据。
- 参照：采用原 E Owner 校验/事务、原 USER pipe 与配置入口；现有 policy-initialize 和原设计的用户定义阶段保持。V10 取原 H/A raw source 与原 RPC step，不造新成功回执、不复制凭据、不重放模型。所有新临时与证据直接写 D；没有为仪器问题重签或换装。秘书长接线、E 只读接口和 V10 真机诊断并行；G 仍由 Claude 负责。

### 本批受影响云检与原 V10 补读

- 原策略头 USER/TS 接线受影响云端原生检查、Browser build/tests、卫生全通过；未运行的完整稳定门槛仍 NOT_RUN。原生只读静态独立复核无阻塞，生产首次阶段输入仍未接通，不默认为测试阶段。
- V10 原失败 reader 的逐帧补读成立：四原 H 的独立 native 身份、原 stdin/RPC、原 A 归一化来源、两项目输入及旁聊之后新正式会话关系均有直接事实；51 条原帧继续 PENDING，不计成功，未关联 RPC，旧失败与数据库原字节保留。正式版、正式数据/登记/快捷方式及原记忆观察器全部等值。原 vendor 返回的四个 jsonl 路径不存在，vendor 历史对象、后续 peer/same-domain 实读及完整 V10 尚未完成；先取原 warning/错误和保存行为原因，不猜路径、不改固定 CLI。
- 最小修正只移除测量脚本的历史提交号/案例号代判，保留原 custody、逐帧已知 Unhandled 形状、typed RPC 排除、旧失败/仪器原字节锁和无活跃 SQLite sidecar；四项测量负对照拒绝。全新只读复核指出摘要不能证明 item 显式线程字段，实际 reader 直接解析原帧后通过该严格条件，未放宽。
- V06 独立案例已开：同一已装 56、原已登录实例、明确测试阶段值，实际 USER 来源先核设置与上限；不换候选、不重登、不重放失败。Root 在 MSIX 视图先查普通 HKCU 键不可见，该读数不用于安装结论；实际普通视图沿原候选身份校验，不能把视图缺值当产品登记丢失。
