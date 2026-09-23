# S1 技术合同与实现决定

本文件定义本次 GPT 制定的方案，不是现有源码已经实现的行为。所有 `gogoke_*` 公共接口及新目录均为拟新增。字段、错误、时序与拒绝条件是合同；内部结构可以等价实现，但不能静默改变此处语义。来源定位与摘要见 SOURCE_MANIFEST.json，原不适用假设不得从函数名推出。

## C01. 组件、依赖与兼容切换

新模块采用 `apps/desktop/src-tauri/src/public_core/`（纯合同/无IO）、`public_store/`（唯一数据写入）、`execution_host/`（本地权威）、`drivers/`（提供方边界）、`delivery/`（持久化投递）、`capabilities/`、`release_policy.rs`。前端是 `src/services/public/`、`src/types/public/` 与 `src/features/s1-harness/` 最小受控客户端。根 crates、Room、Seat runtime 不是新产品依赖根。

既有 `gogoke_daemon` 增加显式 `--local-ipc` 入口并调用同一 execution_host 模块；不新建第二个常驻“Room主控”。桌面通过同一接口访问，不能同时在AppState和daemon各持一份新Session/Delivery事实源。public_core不得导入Tauri、native Codex JSON、CODEX_HOME或spawn。drivers映射原生帧为公共事件，源码侧只允许drivers及明确遗留例外知道原生格式。

引入 `ExecutionRoute=legacy_only|public_only`，由本地受控配置+授权和根记录决定，不信任浏览器参数。legacy_only保持旧UI；public_only用于S1新受控演示。每个逻辑实例和根同一时刻只有一条执行路线。旧原生命令在public_only下转发到已经实现且一一对应的公共操作，无法正确映射者返回 LEGACY_ROUTE_DISABLED，不回退启动旧链；不影响无需模型的本地工具功能。完整旧客户端矩阵仍在WP10b，但S1不能留下访问同一实例的备用spawn旁路。

同一二进制内共享native launch经过一处 LaunchAdmission；跨app/daemon靠root与instance租约同一宿主序列化。未改旧二进制不会理解新锁，不能宣称OS锁能控制它：接管用户旧根另需授权、退出核验与迁移阶段。S1只在临时根构造双宿主/旧writer反例，新根不会自动接管真实Home。

## C02. 公共字段、版本与错误

公共JSON统一camelCase，顶层 `schemaVersion=1`。UUID字符串为产品opaque IDs；时间用UTC RFC3339字符串，超时使用单调时钟，不把系统时间调回当租约延期。sequence、generation、revision采用十进制无符号整数字符串，避免TS number精度损失；零值与unknown分开，未知用显式tag。Rust/TS需同一正负样例，拒绝重复JSON键、非规范数字、无效UTF8、缺必需字段、越界长度或不认识的主版本。

| 对象 | 必需语义字段 |
|---|---|
| Session | sessionId, scopeId, privacyDomainId, title, lifecycle(active/archived), revision；视图或PID不是Session |
| NativeBinding | bindingId, sessionId, driverId, instanceId, profileRevision, authRevision, domainId, generation, nativeSessionId nullable, continuationMode(native/fixed_context/new) |
| Execution | executionId, sessionId, bindingId, generation, state, createdAt, completedAt nullable, resultRef nullable；一个长期进程可多Execution |
| Delivery | operationId, executionId, sessionId, bindingId, generation, requestFingerprint, intentKind, acceptanceState, durableReceiptId nullable, nativeReceipt nullable |
| PublicEvent | schemaVersion, eventId, sessionId, executionId nullable, bindingId nullable, generation, streamEpoch, sequence, kind, payload；路由由服务器真实绑定产生 |
| CapabilitySnapshot | snapshotId, instanceId, executableId(path/hash/version/platform), mode, profileRevision, authRevision, observedAt, expiresAt, evidenceSource, capabilities[name]={state,reason,evidenceRef} |
| ContextPackage | packageId, recipient, scopeId, domainId, taskId, sourceVersion, items[{ref,digest,visibility}], permissions ceiling, expiresAt；不内嵌父对话全文 |
| HumanActionRequest | requestId, executionId, bindingId, generation, continuationId, domainId, expiresAt, allowedAnswers, permissionCeiling, state |
| ProcessIdentity | hostId, instanceId, processHandleRef, pid, startedAtTicks nullable, launchEpoch, imageDigest, containmentId nullable；PID单独不能授权kill |
| OwnershipResult | observation, inMemoryState, persistedState, durabilityAck, custodyState, errors[]；void或busy被清不能代替确认 |

所有新公共命令返回 `{ok:true,value,receipt?}` 或 `{ok:false,error:{code,stage,retryable,sideEffectState,correlationId,message}}`。retryable只表示允许客户端提出重新核验，不表示自动重发副作用。sideEffectState=none|possible|confirmed|unknown。敏感内容不放message；完整内部诊断受域约束。

错误码至少：FEATURE_SEALED、UNAUTHORIZED、SCOPE_MISMATCH、STALE_BINDING、UNSUPPORTED、CAPABILITY_UNKNOWN、PROTOCOL_VERSION_MISMATCH、INVALID_FRAME、OPERATION_CONFLICT、ACCEPTANCE_UNKNOWN、ROOT_UNAVAILABLE、ROOT_IN_USE、ROOT_ID_MISMATCH、UNSUPPORTED_ROOT、STORE_CORRUPT、STORE_WRITE_FAILED、DURABILITY_UNKNOWN、PROCESS_IDENTITY_UNKNOWN、STOP_UNCONFIRMED、LEGACY_ROUTE_DISABLED、QUEUE_FULL、RESYNC_REQUIRED、CONFIG_CHANGED、CANCELLED。

原129个Tauri方法的旧ABI不在WP01整体改成上述envelope；外围适配到其既有Result类型并保持显式错误。新增接口使用 `gogoke_public_request`（action enum，非动态任意方法调用）、`gogoke_public_snapshot`、`gogoke_public_subscribe` 和只读 `gogoke_release_capabilities`。schema通过单一合同样例和双语言decoder固定，不复制根旧protocol为第二权威。

## C03. Release policy：封存而不删除

release政策默认 `remoteExternal=false, voice=false`；受客户端状态、旧settings、环境变量、深链或远端RPC影响也不能变true。开发测试仅可替换依赖，不提供运行时“全权限绕过”开关。受管本地宿主不是外部remote，本地模型无关功能不能一并删掉。

检查点必须覆盖：lib.rs setup/exit、AppBootstrap/settings恢复、远程设置/移动向导、Tailscale start/install/status中会spawn的部分、daemonctl和daemon CLI非loopback bind、call_remote/TcpTransport、dictation桌面与mobile stub的下载/删除/权限/start/stop/cancel、Composer热键和hook。不能只隐藏按钮。

外部connect/listener/安装/自动重启/麦克风请求/模型下载加载删除必须在副作用前拒绝。旧remote配置保存原字节供未来迁移，不自动改为Local后对错误主机写文件：只显示封存说明和本地可用状态，需要用户明确选本地工作区才允许工具目标改变。

模型状态读取可以在已授权现存模型目录仅读metadata/保存路径与hash；缺失/不可读分别报告，不凭封存删除资产或伪报模型已加载。stop/cancel只能处理本进程已经拥有且可证明的录音/下载句柄作清理，不能因此初始化音频设备或创建网络。未知旧受管进程保留诊断，不扫描/杀系统Tailscale；本地daemon退出/保活遵守原Owner策略与精确归属，不用外部封存导致本地会话丢失。

legacy TCP若保留，仅明确loopback地址+既有鉴权，拒绝非loopback、通配bind与release的insecure-no-auth；不是把它宣布为新本地公共通道。WP03 public IPC不用TCP。回滚只回策略代码和测试根，真实资产保留；恢复外部功能另需Owner授权。

## C04. Root、写者与持久化

**选型：desktop独立单写者事务日志，不新增数据库框架。** 这是S1实现决定，而非生产运行证明。新根固定在成功取得的Tauri `app_data_dir()/public-v1`；daemon只接收显式绝对路径并独立核canonical/file identity。根解析错误不得回cwd或`./`。S1测试注入绝对临时根，不使用真实Home。拒绝网络根/UNC、未确认卷类型、指向根外的symlink/junction/reparse，以及别名不一致；Windows按打开的目录handle/file ID识别，不用简单lowercase整路径作为身份。

初始化：先验证可信父目录/当前用户ACL，安全建立或打开根目录，取得下述root.lock独占锁后才create_new root元数据或作为writer解释既有元数据；保持不跟随替换目标的handle。未取得锁不创建新的rootId，也不改root.json。rootId生成一次，schemaVersion=1、ownerIdentity和创建nonce写入；已有未知schema/坏manifest进入readonly诊断，不覆盖默认空池。首次创建/目录持久化失败记DURABILITY_UNKNOWN并禁止激活writer，不伪称`rename`完成即全部耐久。

`root.lock` 用read+write+create（不truncate）的File、`try_lock`独占，句柄持续保留且不传worker；锁文件从不在持锁时删除/替换。WouldBlock=ROOT_IN_USE，其它锁错误不能当“无人用”。排队内只有一个write executor；app/daemon均必须通过同一public_store接口。Rust1.89已有标准File锁API；锁行为不强制所有未合作程序停止写，也不代替身份/custody。[EXT01][EXT02]

目录：root.json、root.lock、domains/<opaque-domain-id>/journal.log、objects/<opaque-object-id>、imports/<import-id>/、custody/与diagnostics/。domain ID不能直接接收路径片段；普通元数据/事件内容按隐私域组织。credentialHome与workspace均不属于产品日志根；manifest只保存敏感引用，不复制秘密。封存语音模型留原位置，S1不迁移。

事务格式：文件头magic+schema+rootId+domainId；每条事务为 u32 big-endian payloadLength + UTF8 JSON payload + SHA256(payload)。payload包含transactionId、previousHash、rootEpoch、单调transactionSeq、完整有序operations与公共事件；length<=16MiB。一个事务同时登记intent/state/对应事件，读者只看到完整校验的事务，不出现对象改了而事件未提交。

每条事务只属于一个privacy domain，不跨domain原子提交。root级hostEpoch/租约元数据放独立control.log，由同一锁持有者串行写入；先sync新rootEpoch，再允许domain日志使用该epoch。跨域委托先在发送域写冻结引用，再在接收域幂等接收，任一阶段失败不授予跨域权限，也不假报双边事务成功。objects文件先用create_new写入、sync并登记digest，再由domain事务引用；崩溃留下未引用对象保留待核，不先删或公开。目录新建/rename采用平台持久化接口，不能取得所需保证则保持DURABILITY_UNKNOWN。

写入步骤：核锁与expected revisions → 序列化和验证整条事务 → append write_all → sync_all并检查返回 → 内存apply → 发布receipt/event。sync错误结果为DURABILITY_UNKNOWN：关闭新派发，保留原字节和内存待核，不能重试覆盖日志或向调用者报已接受。物理设备/文件系统是否承诺掉电持久仍需平台证据，不能由sync API成功推导全硬件保证。[EXT01][EXT03]

S1不做原地压缩；日志硬上限1GiB，预留16MiB给停止/终态/诊断控制记录，普通请求提前QUEUE_FULL/STORE_WRITE_FAILED且未副作用；每项测试可配置较小上限。到极限仍允许尽力停止进程，但若停止收据不能持久化，保留custody和明确错误，不释放writer。后期压缩/大规模迁移是新格式变更而非施工者随手优化。

恢复：持锁后验证header、每条length/hash/prev/sequence；完整事务（即使先前回复丢失）只apply一次，记录operationId用于去重。破损尾部/中间损坏/未知version不得自动截断或空池覆盖；复制坏原件到隔离证据，提供只读有效前缀和明确repair-needed。故障夹具中的恢复工具只能在批准临时副本上显式选择恢复前缀，不能把既有ack丢失当成功。rootEpoch耐久提交后才重新准入。

旧settings/workspaces保留原格式、原位置；只读快照导入到新根，逐条source hash和稳定mappingId防重复。导入与激活分开，坏记录保留/拒绝，绝不调用旧read_settings自动归一化反写当只读导入。真实用户数据切换由WP12；S1不修改原生config/owner。新日志未知则旧exe不得读取新根。

## C05. 唯一执行宿主与传输

HostRegistry按rootId+OS-user记录一个当前hostEpoch；持root锁者才可改变Session/Binding/Delivery/custody。Tauri客户端连接失败只查询本地host状态；不能因此直接spawn provider。启动singleflight按canonical root及固定配置键；leader取消不取消其它等待者共享初始化，所有人取消则清理拥有的句柄，清理未知保留custody；失败结果不永久缓存，重新尝试要用新尝试ID。

Windows命名管道使用当前用户SID限制DACL、拒绝远程客户端、首次实例标志与host进程身份核对；Unix使用私有0700目录中的socket和peer uid核对。pipe路径由用户+root身份摘要生成，不暴露Home。首次启动的随机challenge经继承的专属控制通道传递，不能使用旧远程token或将秘密放普通settings/log；重连需要同一有效产品会话和hostEpoch握手。不把同一OS用户当作任意项目/模型都可授权的主体。named pipe默认DACL可能过宽，因此必须显式安全描述符。[EXT05]

本地wire使用u32长度+JSON，最多4MiB一帧，超长、截断、重复键、未知major立即关闭连接并保留redacted错误。handshake只交换schema/hostEpoch/能力摘要和身份校验，不执行命令；版本不匹配不fallback旧链。控制请求队列64、普通请求每Session128、输出缓存最多1024条且8MiB，取先达到者；队列满在dispatch前显式拒绝。stop/cancel/原continuation有独立队列但不越授权。

所有连接/认证/一次订阅建立与缓存写回包含在singleflight阶段，不能只在最后写槽去重。晚到的旧连接错误按connectionId+epoch compare-and-clear，不能清掉新连接。N=32并发首连只产生一个connect/auth/subscription；竞败的任何已建立句柄必须显式关闭并等待可核结果。该新验收要求直接归S1宿主，不依赖曾被挂起的旧PF07报告权威。

本地权限边界依赖产品principal、scope和可信Tauri操作上下文；worker不持公共管理token，模型文本不能成为principal。OS同用户恶意程序仍可能有较大权限：本计划不宣称仅靠nonce/ACL即可构成同用户强沙箱，真实原生准入须验证可用隔离并拒绝无法执行的权限。

## C06. Launch、实例复用与受限 TS worker

LaunchSpec包含driverId、instanceId、绝对executable路径+hash+version、args数组、explicit cwd、credentialHomeRef、profileRevision/authRevision/domainId/generation、env allowlist、权限上限、binary/prototype digest、containment policy。参数不是shell拼接；Windows .cmd shim只能由已审quote机制在显式允许路径处理，不执行用户工作区同名程序。

环境从白名单重建：OS必需的SystemRoot/ComSpec/SystemDrive/temp/locale及经确认的binary search路径；HOME/USERPROFILE/APPDATA/LOCALAPPDATA、provider Home分别绑定受控实例目录，禁止继承未登记的token、NODE_OPTIONS、提供方配置或工作区可执行路径。不会依赖“父进程已经过滤”。只读模式由宿主策略与实际原生/OS限制分别取证；CLI不支持约束则unsupported，不能改成full-access。

实例复用键含driver/binary digest/mode/instance/profile/auth/domain/scope/generation。更改revision使新请求重新选择，不改在途目标；同账号可有多个隔离实例，不把同账号强制当一个席位。active进程/未知残留或旧writer尚未封口时，不因租约超时/新hostEpoch重新使用同Home。

启动事务：durable claim → 创建受管容器/记录launch-intent → spawn受控进程 → 捕获句柄+真实start identity → durable owner（observed/inMemory/persisted/ack分别记录）→ 原生initialize → 能力验证 → publish ready。任何阶段失败必须保留startup owner，执行受管清理并合并原始/cleanup错误；保存失败不能仅release busy或清child后return0。

R3 C2-F01对应的noteProcess与ensureSeat/ensureSeatInner仅作为错误路径来源，不导入为新host实现；实例记录由public_store写，worker不能调用Room save/releaseAllFor。public接口不能把IdentityUnknown等同AlreadyExited。

TS方案已定为“受限协议worker样本”：新路径 `apps/desktop/worker-s1/`，只复用通过symbol/provenance登记的纯解析与输入校验，注入transport，stdin/stdout帧通信；不import persistence.ts、Persistent*Seat、Room accounts/instances/server或根store。worker不得选择账号、创建产品Session、持根锁、写transcript/owner/state或spawn provider。Rust驱动启动native并控制其管道，worker仅解码/校验分配给它的协议字节，输出公共候选事件仍经host再验证。

worker以批准工作区现成Node24（记录绝对路径/完整版本/hash）和对应已锁定编译器构建固定bundle；执行时manifest校验Node+bundle+schema摘要。S1只做受控原型，不从用户workspace加载、运行时下载latest或默认把Node塞进发布包。Node/bundle实际hash在该构建产生后固定，不伪造为计划时已知。未通过样本/来源边界，不切换Rust生产Codex路径；保留原生Rust能力。worker布局不能改变原提供方功能承诺，真实非Codex验证在WP11a/11b。

## C07. Process custody 与停止

Windows使用独立Job Object管理本实例进程，禁止breakaway；CreateProcess在挂起状态创建，成功AssignProcessToJobObject并记identity之后才resume。assign失败则销毁自己创建的挂起进程并报告失败，不能继续无containment执行。使用同版本windows-sys直接封装，不导入根kernel。KILL_ON_JOB_CLOSE只是清理机制，不是确认后代退出的证据；必须查询active processes/等候已拥有的进程句柄，不能只相信完成端口消息。某些外部创建途径可绕过Job默认继承，真实native准入验证该模式或明确不支持。[EXT04]

Unix进程组仅用于合作子进程控制，不宣称挡住setsid逃逸；保存handle/pid/start identity、已知后代与group归属，真实容器/后代可证的环境才给完整stop proof。无法证明的分支返回descendantsUnknown+custody retained，不对裸PID/group猜测kill，也不把POSIX支持写成Windows全覆盖。

停止结果含requestedAction(interrupt_execution/close_binding/shutdown_host)、graceful、terminationAttempt、processExited、descendantsState、writerFence、durableReceipt、residualIdentities、errors[]。默认worker/native graceful10秒、termination操作预算5秒、观察5秒；由host单调deadline管理，总预算30秒包含身份/命令/存储收尾，禁止沿用8秒宿主先退出。在强制退出/预算耗尽时先记录未完成custody；记录失败则显式durabilityUnknown而不是释放。第二次停止复用原stopId和未完成状态，不因child引用被清返回0。

只有精确identity退出+所有已归属后代已处置/或明确被另一个已授权custodian接管+旧writer已封口+耐久收据，才可释放Home。无句柄但有旧记录、124/125式旧结果、helper异常、保存异常、host终止、null查询都不满足。用户明确强退不等于cleanup成功；重启继续隔离未确认实例，不无限自动重启。

## C08. 能力探测与五家适配

五个driverId固定为codex、claude-code、grok、opencode、antigravity；这是产品适配对象，不是施工模型角色。每家P09a有独立manifest/schema/fixtures和探测接口，即使机器未安装也返回unknown及理由。S1不声称某版本必然支持fork/steer/tools/native delegation。

Observation分三级：L0文件/path/hash/注册manifest检查，不执行native；L1明确批准的--version/--help或协议宣告，有超时/输出上限且禁止登录下载；L2运行/权限/付费能力只在该家准入授权时做。默认只执行L0和假二进制的L1。缺真实证据state=unknown，肯定拒绝的已验证命令才unsupported，不用不存在当unsupported-all。

快照键为executable摘要+version+platform+mode+instance+profileRevision+authRevision+domain，缓存TTL5分钟作为S1保守默认；任意键变化立即失效，读取缓存时再比对身份，不只靠TTL。cache过期不删除在途固定binding，但禁止借旧快照新派发。authRevision变化通过可信观察/设置动作，不能自行打开凭据推断登录。账号/额度不由本地JWT或历史用量冒充官方信息。

## C09. Delivery：去重、未知接受和控制优先

一次用户意图产生operationId，正常同文本再次发送是新ID。指纹由host按固定规范计算：UTF8规范JSON（键按UTF8字节排序、array保序、只允许整数/十进制字符串、Unicode不另归一化）包含schema/action/session/binding/generation/domain/内容/附件digest/权限/continuation；不含传输requestId、重试时间或UI焦点。同ID同指纹返回已知receipt/status，异指纹拒绝OPERATION_CONFLICT，不能复用ID换目标。

Delivery状态：recorded → queued → dispatching → accepted|acceptance_unknown|rejected；Execution独立为created/running/completed/failed/cancelled。recorded必须先journal sync；dispatching也先sync才写native。响应丢失、EOF、写入部分后错误、重启遇dispatching都标unknown，不盲重发。只有明确确认未产生副作用的拒绝可以重新排队；重发stateful原生请求需要原生operation receipt/dedup实证，不按read前缀或旧36重试列表推定幂等。

取消queued项可耐久cancel；取消dispatching/accepted是新control intent关联原execution，不能把收到cancel请求等同真实停止。控制队列优先于普通消息，避免输出洪水吞stop/原批准回答；拒绝/入队/接受/执行完成/通知显示分别取证。terminal事件与状态同事务持久，不因队列满丢终态。

## C10. Event cursor、重连和隐私

一个domain事件流以root/streamEpoch+sequence定位；eventId和(native绑定内来源键)去重。旧generation事件进入受限诊断而不写当前会话；同sequence不同内容是协议冲突，触发resync，不忽略。native无连续序号时host只能给自己接收到事件的序号，不能据此证明native未丢事件，需标sourceContinuity=unknown。

消费者按cursor请求，保留窗口之外返回RESYNC_REQUIRED+授权snapshot边界，snapshot和nextCursor由同一事务视图产生。输出背压上限到达先暂停可暂停native读取；不能暂停时断开该订阅并要求snapshot，不默丢事件/未知notification。可忽略的无副作用未知minor事件明确记录版本分类；涉及state/permission未知帧fail-closed。S1本地故障复现不替代WP10b的全部旧协议兼容矩阵。

所有列表、历史、事件订阅、诊断、导出引用、草稿与context package都以principal+scope+privacy domain服务端过滤；跨域Session ID不能读出存在性或内容。每个派工包只复制用户/Controller显式选中且允许的材料引用与digest。使用高熵marker正反测试真实payload/stdin/规则/日志/缓存/恢复快照，不只检查函数返回字段。各worker结果回请求Controller；正文@、日志中的owner-approved、原生subagent通知均不创建新公共权限/席位。

## C11. HumanAction 与最小委托

HumanAction必须包含真实request/continuation及binding/generation/domain/expiry，答案在allowedAnswers内且不扩大permissionCeiling。重复同答案返回同receipt；重复冲突、过期、错Session/错caller/终态后回答拒绝。不把回答转换为新一轮普通prompt或插话，不自动记住全局授权。

委托是结构化Delegate请求，不解析模型文本执行；接收者固定、材料最小化、预算/工具/写根为上限交集。child scope只能缩小，调用者不能自称Owner升级。S1用两个受控假driver及干净review binding验证最小接缝，不恢复Room的组织、排班和广播权威。正式独立审查必须fresh材料，UI或模型建议的“继续上下文”不得绕过。

## C12. 常量、验证范围与回到 GPT 的条件

上述timeout/容量是方案配置默认值，不是测量结果；允许测试下调以加速，但必须同时验证发布默认值已实际加载。提高或降低会改变权限/耐久/送达语义的上限不得以调参跳过失败。所有native/OS/存储副作用仅在Owner批准的临时根、假binary、受管handle和私有本地端点内测试。

返回GPT的实质条件是必须改变组件权威、公共schema、权限/身份、迁移、停止证明或采用边界；缺库、普通类型错误、分支实现bug和已定合同内算法调整由Codex处理。Node样本实测不满足此合同，则维持主Rust路径、报告具体阻断，不暗自采用Room或删原义务。

## 外部技术依据（仅API边界，不代替固定源码或平台实验）

EXT01 Rust std::fs::File，lock/try_lock文档标1.89.0，sync_all含义和Drop错误限制。来源：https://doc.rust-lang.org/std/fs/struct.File.html 。本计划要求desktop固定1.89兼容，不使用页面较新nightly API。
EXT02 Linux man-pages维护者的flock(2)，说明advisory语义、描述符继承与网络文件系统差异。来源：https://man7.org/linux/man-pages/man2/flock.2.html 。S1据此拒绝未核网络根并禁止向worker泄露锁句柄。
EXT03 Microsoft FlushFileBuffers，写入缓存、失败报告与具体handle语义。来源：https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers 。它不替代本项目崩溃/掉电与目录持久化测试。
EXT04 Microsoft Job Objects / AssignProcessToJobObject，进程关联、breakaway、继承例外与通知限制。来源：https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects ；https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject 。
EXT05 Microsoft Named Pipe Security and Access Rights，显式DACL与默认访问范围。来源：https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights 。

这些网页在本轮读取用于确定方案；没有执行真实平台实验。原包/源码引用仍以SOURCE_MANIFEST的固定Git身份为准。

## 附录 A. 公共请求和事件的初始闭合集合

`gogoke_public_request` 只接受以下action与命名payload。外层包含requestId、clientInstanceId；principal、许可和scope capability由可信本地会话解析，不接受payload内自称Owner。每个有副作用的请求还必须operationId，客户端不得指定host生成的receipt、execution结果或durabilityAck。

| action | 输入必需项（除外层字段） | 输出 |
|---|---|---|
| session.create | scopeId, domainId, title | 新Session + durable receipt |
| binding.select | sessionId, expectedSessionRevision, driverId, instanceId, profileRevision, authRevision, mode | 未启动/待准入Binding；nativeSessionId只能由驱动登记 |
| delivery.send | sessionId, bindingId, expectedGeneration, operationId, contentItems, contextPackageId nullable | Delivery receipt；executionId由host关联 |
| execution.interject | executionId, bindingId, expectedGeneration, expectedNativeTurn nullable, contentItems | steer支持证据不足则拒绝；不能改为queue |
| execution.cancel | executionId, bindingId, expectedGeneration, operationId | cancellation intent + stop状态，非“已经停止” |
| binding.close | bindingId, expectedGeneration, operationId | typed Stop/OwnershipResult，unknown不释放 |
| human_action.answer | requestId, continuationId, bindingId, expectedGeneration, answer, operationId | 原请求回答receipt，禁止新prompt |
| context.create | recipient, taskId, scopeId, domainId, sourceVersion, items(ref/digest/visibility), expiresAt | 冻结ContextPackage，权限取交集 |
| delegate | recipient, contextPackageId, parentExecutionId, permissionCeiling, operationId | 子任务执行引用，结果返回主控 |
| receipt.read | operationId | 已登记Delivery/Execution快照，不能因查询不存在自动重建 |

`gogoke_public_snapshot`接受scope/session及expectedStreamEpoch，返回同一事务视图的状态和cursor；`gogoke_public_subscribe`从cursor开始并服务端过滤domain。取消订阅只是关闭消费者，不停止Execution。

contentItems区分text、attachmentRef、toolResultRef，引用须冻结digest及授权；S1只用受控text/合成附件对象，不因为接口能表示路径就允许任意文件读取。参数长度：标题最多256 UTF8字节，文本最多1MiB，单ContextPackage最多128个引用、合计帧仍不超过4MiB；不允许隐式截断后继续执行。

事件kind初始集合：session.updated、binding.updated、delivery.updated、execution.started、execution.output_delta、execution.completed、execution.failed、execution.cancelled、human_action.requested、human_action.resolved、context.created、delegation.result、stream.gap、diagnostic。每种payload只含本kind字段；state由已验证事件/事务决定，diagnostic/模型文字不改变状态。native thread/item/turn原文只能由driver转换或进入受限诊断，不直接暴露成公共许可控件。

recorded/queued可转cancelled且未派发；dispatching不能直接被客户端改成rejected，跨崩溃恢复为acceptance_unknown；accepted的Execution终态由可归属的native终态或明确host失败产生；未知接受可经原生receipt核对为accepted/rejected，但不能因超时变成可安全重发。terminal后迟到output保留为受限附加证据，不复活Execution。

Session/Binding变更使用expected revision进行乐观并发检查，并仍由单写者串行提交；revision冲突返回STALE_BINDING，不覆盖当前scope/profile。未来增加action/kind需minor兼容分类；增加授权或改变旧字段语义必须回GPT版本决策。
