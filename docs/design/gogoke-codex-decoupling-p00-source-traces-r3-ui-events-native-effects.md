# gogoke P00 接手摸排 R3：UI 自动入口、事件消费者与浏览器/系统副作用

**日期：2026-09-18｜源码基线：`789cbd2c66642c4e129900614d7d0b0b44a8b5ef`｜证据层级：SOURCE_BRANCHES_EXPANDED**

本批承接 R1 的 `RF03`、`RF04`、`RF05`。它逐项记录没有独立 Tauri command、但会主动触发 IPC、进程、网络、通知、音频、剪贴板或会话状态变化的 React effect、事件 hub、回调和浏览器原生入口。

- `P00 acceptance = NOT_ACCEPTED`
- `global orphan_sink_count = null / NOT_COMPUTED`
- 未运行产品、真实 CLI、通知权限、音频、剪贴板、拖放、远程服务或用户工作区脚本。
- 本文不把 source-present 写成 runtime-verified，也不把前端 state 改变写成后端动作完成。

## 1. 本批对后续公共架构的直接约束

1. **公共 Event 不是只服务聊天 reducer。** 同一原生事件还驱动声音、系统通知、需要回应通知、未读、深链、rate limit/account 刷新和远程 live 状态。
2. **焦点和可见性是执行入口。** 获得焦点、窗口恢复、tab 可见、remote poll timer 都可能刷新 workspace/thread、连接 workspace、resume thread、subscribe/unsubscribe。
3. **前端事件 fanout 无持久送达语义。** Tauri `emit` 错误被丢弃；event hub 在进程内 fanout，listener异常只记 console；没有 sequence、generation、ack或重放。
4. **UI 可见性不等于后端生命周期。** 关闭终端 panel会 dispose xterm UI，但不会自动 close PTY；切 tab和固定 ID脚本 terminal又是另一套状态。
5. **系统通知与剪贴板会携带内容。** 通知 body可包含最后一条 agent文本、审批命令预览、问题或 plan首行；复制 thread会把完整 transcript写系统剪贴板。它们必须纳入隐私域，而不是当装饰层。

## 2. Terminal tab、脚本和 UI 生命周期

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R3-001 | `useTerminalTabs.ensureTerminal` | terminal panel打开且 workspace存在时，若没有 active tab就只在前端创建一个随机 tab ID；此时未必已经有 PTY。 |
| R3-002 | `useTerminalSession` visible+active effect | xterm已经建好后调用 `terminal_open`；成功才把 key放入 `openedSessionsRef`。同 key前端会避免重复 open，但后端也有独立并发去重与补偿分支。 |
| R3-003 | terminal panel隐藏 | dispose xterm、input disposable、fit addon并清 rendered key；**没有调用 `terminal_close`**。PTY/session可继续存在。再显示时前端 `openedSessionsRef`仍可能认为已打开。 |
| R3-004 | terminal output hub | 所有 terminal output先进入每 key内存 buffer，最大约200k字符；active key同时写xterm。buffer只在前端内存，重启丢失，不是 transcript。 |
| R3-005 | terminal exit hub | 收到 `terminal-exit`后删 buffer/opened key并可能关最后一个 tab/panel；但R1已证实该事件可由reader EOF/错误触发，不是child exit receipt。 |
| R3-006 | `useTerminalTabs.closeTerminal` | 先同步删UI tab/切active，再调用 `onCloseTerminal`；callback不await。后端close失败时tab已经消失，仅记debug。 |
| R3-007 | `restartTerminalSession` | 先清前端状态，再请求close；只有“session not found”可忽略，其他错误抛出。单脚本与多脚本会在catch时清pending，不写脚本。 |
| R3-008 | 单 launch script | run保存pending，打开panel并restart；另一个effect只有在 workspace/terminal/readyKey全部匹配才写 `${script}\n`。写入成功不代表shell command启动、退出或成功。 |
| R3-009 | 多 launch scripts | 每条固定 `launch:<entryId>` terminal ID；流程同单脚本。写失败还会产生应用内 error toast；success没有exit receipt。 |
| R3-010 | worktree setup script | R1已固定 restart失败后仍继续open+write、并立即mark-ran。与手工launch是三条不同脚本入口，不能合并成一条“terminal script”。 |
| R3-011 | tab状态范围 | tabs与active IDs按 workspace存于React state；非持久化。关闭/重启应用不会自动恢复UI tab，但后端残留PTY是否存在需运行验证。 |

## 3. Focus、visibility、poll和live subscription

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R3-012 | `useWorkspaceRefreshOnFocus` | window focus或document变visible后500ms debounce：先list workspaces，再对connected entries列threads并preserveState。refresh失败被吞；仍可能使用effect闭包中的旧workspace集合。 |
| R3-013 | remote workspace polling | remote mode且document visible时每15秒执行同一refresh cycle；in-flight期间跳过，不排队补偿。 |
| R3-014 | `useRemoteThreadRefreshOnFocus` | remote+active workspace/thread时，DOM与Tauri focus/visibility入口都可触发；若UI认为workspace未连接，会先调用connect，再refresh/resume thread。错误全部吞掉。 |
| R3-015 | remote thread polling | live未挂起、thread不processing、window focused且document visible时每12秒refresh；processing或live状态会停poll。状态判断来自UI，不是服务端租约。 |
| R3-016 | live subscription thread switch | 目标 key变化时best-effort unsubscribe旧key，再视local snapshot决定是否先refreshThread，随后subscribe。unsubscribe失败不阻止subscribe。 |
| R3-017 | live重连既有subscription | target已active时先设置10秒self-detach忽略窗口，best-effort unsubscribe，再subscribe；迟到detach可能被忽略，也可能在窗口外触发重新resume/subscribe。 |
| R3-018 | live blur/hidden | blur或hidden使reconnect sequence失效、清desired key、best-effort unsubscribe并将状态降为polling/disconnected；原生服务端subscription是否实际解除未知。 |
| R3-019 | `codex/connected` event | document visible时会触发 `reconnectLive(..., runResume:false)`；不是仅更新connected badge。 |
| R3-020 | live attached/detached/heartbeat/activity | attached记录key；heartbeat和选中thread activity直接把UI状态设live；detached可触发带resume的重连。activity存在不证明subscription健康。 |
| R3-021 | 通知 deep link | system通知发出前记录一个最多120秒的pending workspace/thread。focus或workspace load后尝试refresh workspace、必要时connect，再调用openThreadLink；connect失败仍继续open link。只保留一个pending link，新通知会覆盖旧值。 |

## 4. Event producer、hub与raw method路由

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R3-022 | Rust `TauriEventSink` | `app.emit(...)`结果被 `_ =` 丢弃；app-server、terminal output、terminal exit都没有后端可见的送达失败。 |
| R3-023 | event payload identity | `AppServerEvent`只有 `workspace_id` + raw JSON message；没有产品Session ID、binding generation、monotonic sequence或cursor。terminal事件只有workspace/terminal IDs。 |
| R3-024 | `createEventHub` | 每个hub维护listener Set和一个native listen；listener异常只console error并继续。首个listen异步失败只通知调用start时传入的那份onError；后加入listener不会自动重试。 |
| R3-025 | late listen resolution | 最后listener在listenPromise完成前移除时，promise resolve后会立刻unlisten；这是有效清理，但没有buffer期间到达的事件。 |
| R3-026 | duplicate semantic hubs | model/access/reasoning/collaboration各存在普通与composer两套hub，绑定相同native event name。若两套都有subscriber，将建立两个native listener，不能按“一个event name一个listener”推断。 |
| R3-027 | `useAppServerEvents` | 每个使用该hook的消费者都订阅同一个appServerHub；hub内再fanout。raw payload先发给`onAppServerEvent`，之后才判method是否合法。debug消费者因此可记录unsupported/stderr等原始内容。 |
| R3-028 | approval/requestUserInput | 只有带request id的approval会形成ApprovalRequest；user input问题会裁剪/归一化。缺ID或空问题可能静默不进入对应UI，但raw debug仍可见。 |
| R3-029 | raw event normalization | thread/turn/item/account/hook/plan/diff/token等字段在前端按camel/snake两套读取；缺thread/item/delta时多数直接忽略。没有统一invalid-event计数或恢复请求。 |
| R3-030 | unsupported event | `isSupportedAppServerMethod`为false后退出；除raw debug/live hook可能读取外，不进入会话状态。公共Event迁移必须保留未知事件诊断而不能授权未知动作。 |
| R3-031 | background thread | `codex/backgroundThread`仅action=`hide`时将thread藏起；其他action忽略。它不是公共席位生命周期，且隐藏可影响后续列表/未读可见性。 |
| R3-032 | thread archive cascade | 根thread archive后会异步逐个archive全部已知subagent descendants；每个请求可部分成功。120秒skip map用于避免事件再触发级联，但不是事务。 |
| R3-033 | account event side effect | `codex/connected`会触发rate limit/account刷新；account updated/login completed也再次刷新。一个连接事件不仅改connected状态，还发后续原生查询。 |

## 5. 未读、计划、通知和隐私副作用

| ID | 实际入口与已读链 | 实际效果、条件与失败边界 |
|---|---|---|
| R3-034 | agent delta | 确保thread、mark processing并append；**不会立刻mark unread，也不调用全局message activity**。完成事件才更新时间/activity并可能mark unread。 |
| R3-035 | agent message completed | 更新时间、最后消息、localStorage activity并触发message activity；仅当 `threadId !== activeThreadId` 时 `markUnread=true`。判断基于全局当前active thread ID，不读取window focus或visibility。 |
| R3-036 | turn error unread | 非retry error插入assistant error message；若thread非active则mark unread。旧turn error在activeTurnId不匹配时被忽略。 |
| R3-037 | thread started/activity | thread start会记录activity/timestamp并可能命名，但不自动mark unread。subagent visibility/parent mapping影响是否出现在列表，不应代替未读事实。 |
| R3-038 | plan状态 | `turn/plan/updated`写结构化plan，`item/plan/delta`追加展示；turn完成可能按helper清plan。另一个独立通知hook只在item type=plan且status字符串看起来complete时发“Plan ready”。模型声明/原生item状态/通知不是同一事实。 |
| R3-039 | completion system notification | 只有运行时长达到默认60秒、窗口不focused、enabled且通过1.5秒去重才发。标题来自workspace名，body可为最后agent message前200字符。敏感回复会跨出应用进入OS通知。 |
| R3-040 | completion double-source抑制 | agent message completed会先consume duration并可能通知；随后turn completed通常拿不到duration，从而避免第二次。若事件缺失/顺序不同，行为可能不同，需fixture和native验证。 |
| R3-041 | sound notification | 独立hook维护另一套duration map，条件类似系统通知但不看subagent开关。它通过Web Audio加载打包URL；system notification成功与声音成功彼此独立。 |
| R3-042 | response-required approval | approval数组变化后选最新未通知项；body可能是命令preview。key仅workspace+requestId，移出active数组后会从notified Set删除；同ID未来重现可再次通知。 |
| R3-043 | response-required question | extra携workspace/request/thread/turn/item IDs，body为第一问题header/question。通知spacing导致retry timer；只存最新未通知遍历结果。 |
| R3-044 | plan notification queue | complete plan在spacing不足时进入内存Map，之后逐个发；unmount只清retry timeout，没有清plan map（hook实例销毁后由GC）；没有持久恢复。 |
| R3-045 | subagent通知过滤 | system completion和response-required可按`isSubagentThread`关闭；sound hook没有该过滤。subagent detection来自当前内存映射，尚未hydrate时可能分类不同。 |
| R3-046 | `sendNotification`权限/回退 | 每次先invoke检查macOS debug；debug直接尝试osascript并无论fallback成功与否返回。正式分支动态import plugin，可能请求OS权限；拒绝/异常后fallback，而非抛出给caller。非macOS-debug fallback本身返回Err但最终`sendNotification`仍resolve。 |
| R3-047 | notification call success语义 | caller await resolve不证明可见通知；plugin或fallback失败多数只console warn。`onThreadNotificationSent`在调用notify之前执行，因此deep link可被记录，即使通知未显示。 |
| R3-048 | Web Audio | 首次播放创建全局AudioContext；suspended时调用resume但不await；随后fetch→decode→gain→destination→start。并发通知可重复fetch/decode同资源；失败只debug。 |
| R3-049 | copy thread | 显式菜单/按钮可将完整可见transcript写 `navigator.clipboard`；失败只debug。系统剪贴板是独立隐私sink，不能只盘文件export。 |
| R3-050 | drag/drop event service | Tauri环境下首subscriber安装window drag/drop listener并fanout；listener异常隔离。payload的paths直接来自OS事件，后续消费者的路径验证仍需RF03 leaf对账。 |

## 6. 仍需继续读取的前沿

- `RF03`：还需 modal/layout/menu/shortcut 与拖放消费者逐项绑定到上述回调；特别是外部open、文件选择、附件、workspace dialogs。
- `RF04`：还需 daemon transport的event producer/subscription、app-server stdout reader、thread reducer clear-unread入口与tray/menu producer对账。
- `RF05`：还需全部11+1 clipboard caller、9个openUrl、12个reveal、dialog/notification/opener消费点及post-update notice renderer。
- `CF01–CF05`：保持OPEN；`orphan_sink_count`仍为null。

## 7. 已读源码索引

固定提交均为 `789cbd2c66642c4e129900614d7d0b0b44a8b5ef`：

- `src/features/app/hooks/useWorkspaceLaunchScripts.ts`, `useWorkspaceLaunchScript.ts`, `useWorktreeSetupScript.ts`
- `src/features/terminal/hooks/useTerminalTabs.ts`, `useTerminalSession.ts`, `useTerminalController.ts`
- `src/features/workspaces/hooks/useWorkspaceRefreshOnFocus.ts`, `useWorkspaceRestore.ts`
- `src/features/app/hooks/useRemoteThreadRefreshOnFocus.ts`, `useRemoteThreadLiveConnection.ts`, `useSystemNotificationThreadLinks.ts`
- `src/services/events.ts`, `dragDrop.ts`, `tauri.ts` notification section
- `src/features/app/hooks/useAppServerEvents.ts`
- `src/features/threads/hooks/useThreads.ts`, `useThreadEventHandlers.ts`, `useThreadItemEvents.ts`, `useThreadTurnEvents.ts`, `useThreadStorage.ts`
- `src/features/threads/utils/threadStorage.ts`
- `src/features/notifications/hooks/useAgentSystemNotifications.ts`, `useAgentSoundNotifications.ts`, `useAgentResponseRequiredNotifications.ts`
- `src/utils/notificationSounds.ts`, `src/features/threads/hooks/useCopyThread.ts`
- `src-tauri/src/event_sink.rs`, `src-tauri/src/backend/events.rs`, `src-tauri/src/notifications.rs`

后续合并时必须保留“触发条件/失败语义/内容外泄面”，不能缩回“通知、事件、终端已盘”。
