# gogoke P00 接手摸排 R5：Remote、Tailscale 与 daemon 真实链

**日期：2026-09-18｜源码基线：`2422f32796bc4c9771bf6e92c9fcb39c667b4ffe`｜证据层级：SOURCE_BRANCHES_EXPANDED**

本批承接 R1 的 `RF08` 中 remote/Tailscale/daemon 部分。它区分 app内remote client、受管daemon、standalone daemon、daemonctl、Tailscale CLI 和移动端设置入口。当前产品范围要求远程首版封存保留；源码仍有自动启动、连接、监听和外部CLI入口，因此不能把它标成已经 `preserved_disabled`。

- `P00 acceptance = NOT_ACCEPTED`
- `global orphan_sink_count = null / NOT_COMPUTED`
- 未启动/停止daemon、未连接端口、未运行Tailscale、未读取真实token/settings、未发RPC或网络请求。
- 下表是源码事实与设计输入，不是漏洞复现或runtime验收。

## 1. 后续施工必须接受的约束

1. **remote不是单一路径。** app remote client、app受管daemon、standalone `gogoke_daemon`、`gogoke_daemonctl` 和移动端wizard各有自己的root、认证、状态和启动入口。
2. **断线重试存在“请求可能已执行、响应丢失”的窗口。** allowlist里不只有纯读，还包括连接、runtime args、resume和subscribe；必须逐方法证明可安全重试或改为核对。
3. **事件流会无声丢失。** daemon broadcast lag、未知notification、Tauri emit失败都没有序号/gap recovery。
4. **daemon shutdown不排空工作。** RPC返回后100ms直接`process::exit(0)`；并发请求、磁盘写和子进程可能被截断。
5. **token边界不只settings文件。** app启动受管daemon时把token作为`--token`命令行参数；daemonctl也接受arg/env/settings三种来源。

## 2. App remote client

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R5-001 | Tauri adapters → `remote_backend::call_remote` | backendMode=remote时按需创建TCP client；不是应用启动即必定连接。client缓存在AppState，setting变化可清空。 |
| R5-002 | transport config | host来自settings，空值用default；token可为空。TCP transport直接`TcpStream::connect`，应用协议无TLS。部署预期可能依赖Tailscale网络层，但源码本身不验证对端证书/设备身份。 |
| R5-003 | authentication | TCP连接成功后，如settings有token就先发`auth` RPC；无token时client不发auth。服务端若要求token，后续请求返回unauthorized。 |
| R5-004 | request bookkeeping | request id是每个client从1开始AtomicU64；先插pending，再把JSON行发入容量512的outbound queue。send timeout15秒，response timeout300秒。 |
| R5-005 | ambiguous timeout | send成功后response timeout会从pending删除并返回error；server可能已执行。没有operationId、server receipt或后续状态核对。 |
| R5-006 | disconnect retry | 连接断开时清cached client；allowlist方法会新建连接并自动再发一次。纯写方法如send/start/remove被排除，这是有效保护。 |
| R5-007 | retry allowlist非纯读 | allowlist包含`connect_workspace`、`set_workspace_runtime_codex_args`、`resume_thread`、`thread_live_subscribe/unsubscribe`。其中runtime args可respawn全部shared session；源码需要逐方法幂等/核对证明，不能统一称retry-safe。 |
| R5-008 | reader | 按newline读；空行跳过，非法JSON/未知shape由parser返回None后静默丢弃。response id不在pending时忽略。 |
| R5-009 | notification bridge | 只桥接app-server-event、terminal-output、terminal-exit；未知notification静默忽略。转发到Tauri时emit error被丢弃。 |
| R5-010 | writer/read failure | writer失败或reader结束都mark disconnected，并drain全部pending为同一个disconnected error；无法区分请求未发、部分写、已执行或只丢响应。 |
| R5-011 | WSL path normalization | 仅部分workspace adapter在remote前将`\\wsl$`/`\\wsl.localhost`转换成Linux path并去掉distro段；不是所有path-bearing RPC都有同一转换。 |

## 3. Server transport、并发与事件

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R5-012 | daemon listener | 默认standalone绑定127.0.0.1:4732；app受管移动access按settings端口绑定`0.0.0.0:<port>`。每个accept spawn独立client task。 |
| R5-013 | auth lifecycle | config无token时连接一开始就authenticated并订阅事件；有token时除auth外请求返回unauthorized，正确auth后开始事件subscription。`--insecure-no-auth`是显式dev入口。 |
| R5-014 | parse/error response | 非法JSON直接continue；没有request id时`build_error_response`返回None，因此caller得不到错误。未知method在dispatcher末端返回error。 |
| R5-015 | concurrency | 每连接最多32个in-flight请求，但每个合法请求独立spawn task；generic transport没有per-session/per-workspace顺序。具体shared core锁只覆盖局部临界区。 |
| R5-016 | server outbound queue | 每连接使用unbounded mpsc写response/events；慢客户端可能累计消息。write失败后task退出，但业务task仍可能继续执行并向已断out_tx发送失败。 |
| R5-017 | broadcast event loss | daemon broadcast容量2048。client `forward_events`收到Lagged时直接continue，没有发送gap、cursor或snapshot需求。 |
| R5-018 | daemon EventSink | broadcast send result被丢弃；没有receiver时也不会反馈producer。terminal/app事件都没有持久队列。 |
| R5-019 | client close | socket EOF后drop sender、abort events task和write task；不取消已经spawn的RPC业务task。 |
| R5-020 | shutdown RPC | `daemon_shutdown`安排100ms后`process::exit(0)`，立即返回`ok`。没有停止接单、等待32并发任务、flush storage、收原生session或发终态事件。 |
| R5-021 | event parity | daemon只传三类notification；menu accelerators在daemon明确no-op，macOS debug通知fallback可能在daemon所在机器执行。remote parity不能按方法名存在来推定UI/OS体验等价。 |

## 4. Daemon state、root和磁盘同步

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R5-022 | state load | `workspaces.json`/`settings.json`读取错误使用`unwrap_or_default()`，可在内存呈现空数据；没有在load时阻止writer或保留corrupt状态。 |
| R5-023 | standalone default root | XDG优先，否则HOME；HOME缺失用`.`，生成cwd相对`.local/share/gogoke_daemon`。违反V3禁止root fallback cwd。 |
| R5-024 | app-managed root | 使用app `settings_path.parent()`作为`--data-dir`；这是正确共享root意图，但实际路径、别名和OS锁仍未验证。 |
| R5-025 | list_workspaces side effect | daemon每次list前重新读storage；成功后替换内存workspaces，并移除不在磁盘的session、调用kill。一个“list”可终止原生进程。 |
| R5-026 | list read failure | read失败只stderr并保留旧内存map；caller继续取得旧workspace列表。与初始load失败变空的语义不同。 |
| R5-027 | stale session pruning | 先从map移除，再逐个kill；kill无result。UI可能看到disconnected，但进程是否仍写入未知。 |
| R5-028 | workspace file read | daemon canonicalize root和candidate、拒绝escape、最多读400k、只接受UTF-8；这是有效保护。listing不follow links并跳固定dirs，最多指定数量。 |

## 5. App受管daemon与Tailscale CLI

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R5-029 | app setup自动入口 | desktop setup读取settings；provider=tcp时，remote mode直接尝试start；local mode若status显示running也start以强制版本一致。错误被忽略。远程尚未封存。 |
| R5-030 | setting update自动入口 | 保存settings后如transport字段变化清remote client；backendMode=remote时调用daemon start，错误被忽略，然后返回settings成功。保存成功不证明daemon成功，且会主动启动进程。 |
| R5-031 | app exit | keep-daemon=false时prevent exit，调用tailscale daemon stop后无论结果如何app exit。不能从app退出推定daemon/子进程停止。 |
| R5-032 | command preview | 解析daemon binary、data dir和token configured，只返回placeholder command，不泄token；此入口本身不spawn。 |
| R5-033 | start preflight | 必须有token，计算0.0.0.0 listen，解析binary/data root；先refresh内存child，再probe端口。 |
| R5-034 | existingdaemon restart | 若identity/version/mode不符但auth/identity可核，会先RPC shutdown，必要时Unix按PID TERM/KILL；ownership不足则拒绝force kill。该保护有效。 |
| R5-035 | PID边界 | info PID或lsof listener PID，没有创建时间/executable digest绑定。Unix kill(0)/TERM/KILL可受PID reuse竞态；非Unix不能force-stop external PID。 |
| R5-036 | child spawn | 直接启动daemon binary，参数含`--listen 0.0.0.0`, `--data-dir`, **`--token <secret>`**，stdio null。token可能暴露给可读取进程命令行的本机主体。 |
| R5-037 | start success | spawn成功立即把runtime标Running；不等待daemon bind/auth/ping。随后status/probe才可能发现startup failure。 |
| R5-038 | stop managed child | 若AppState持有child，调用process-tree kill和wait；kill结果不返回，之后再probe决定最终status。 |
| R5-039 | stop external daemon | 先RPC shutdown+wait；仅auth ok且name匹配才允许force kill。stop最终会再次probe并报告still running/non-daemon。 |
| R5-040 | Tailscale status | 遍历PATH和标准安装路径执行`talescale version`，再`status --json`。macOS先`launchctl asuser`，失败再直接执行，单次status可能启动两次CLI。 |
| R5-041 | Tailscale status内容 | 可读取DNS name、hostname、tailnet/IP并返回suggested remote host；属于本机网络身份信息，不应进入普通日志/分享包。 |

## 6. Standalone daemonctl与移动端wizard

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R5-042 | daemonctl root | Windows APPDATA优先，USERPROFILE缺失用`.`；macOS/Linux HOME缺失也用`.`。同样存在cwd fallback。 |
| R5-043 | token resolution | 显式arg优先，其次`CODEX_MONITOR_DAEMON_TOKEN` env，再settings token。start command可把token放命令行。command-preview使用placeholder。 |
| R5-044 | status/stop/start | status会连接/probe；stop可能RPC和Unix signal；start可shutdown/restart旧daemon、bind preflight并spawn。名称“ctl”不表示只读。 |
| R5-045 | mobile wizard auto check | mobile settings加载后，只要host/token已配置，effect自动调用`listWorkspaces`进行连接测试，并best-effort refresh。 |
| R5-046 | mobile connect/test | 先把backendMode=remote、host/token写settings，再连接；成功后第二次写settings更新remoteBackends和lastConnectedAt。连接失败时第一份remote配置已持久化。 |
| R5-047 | token UI state | token进入React state及AppSettings/remoteBackends结构；后续迁移/日志/export必须单独脱敏，不能将整个settings对象当普通公共配置复制。 |

## 7. 当前闭合状态

- `RF08 remote/daemon`：主要生产路径已展开；仍需所有daemon RPC handler逐方法读写分类、Tailscale `core.rs` JSON解析和各settings UI按钮/菜单/深链调用收口。
- 远程当前不能标`preserved_disabled`：注册、自动start、移动wizard和remote adapters均active/source-present。
- 进入代码施工前，WP10a必须先形成“入口全部封住但本地daemon保留”的具体变更清单；WP02/WP03必须先固定root/token/owner/retry/event-gap合同。
- `CF01–CF05`保持OPEN；runtime未验。

## 8. 已读源码索引

固定提交均为`2422f32796bc4c9771bf6e92c9fcb39c667b4ffe`：

- `src-tauri/src/remote_backend/{mod,transport,tcp_transport,protocol}.rs`
- `src-tauri/src/bin/gogoke_daemon/{transport,rpc, rpc/dispatcher, rpc/daemon}.rs`
- `src-tauri/src/bin/gogoke_daemon.rs`
- `src-tauri/src/bin/gogoke_daemonctl.rs`（root/args/preview片段及既有P00精读）
- `src-tauri/src/tailscale/{mod,daemon_commands,rpc_client}.rs`
- `src-tauri/src/settings/mod.rs`, `src-tauri/src/lib.rs`
- `src/features/mobile/hooks/useMobileServerSetup.ts`

后续整合不得把app remote client、受管daemon、standalone daemon和daemonctl合并成一个“remote service”。
