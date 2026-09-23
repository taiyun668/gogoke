# 第一枪前 API 与界面入口核对

基线：`9131cfe`，2026-09-13。机械清单来自 `rg -n 'url\.pathname' packages/room/src/server.ts`、`rg -n '/api/' packages/room/public/index.html` 和两文件中的 `action ===` / `haltAction(`；原始带行号输出保存在 `%TEMP%\gogo-first-shot-A\{server-route-lines,ui-api-lines,action-lines}.txt`。动态路径按服务器实际分支展开；“无独立入口”不表示接口不存在。

## `/api/halt/action` 的全部动作

| action | 界面触发点 |
| --- | --- |
| `start` | 当前规划目标顶部「开工」→确认框 `confirmStart`；目标和检查声明由 Owner 确认。 |
| `request-reconciliation` | 当前已开工、未验收目标的输入工具条「对账」→`requestReconciliation`；409 写 `senderror`。 |
| `accept` | 对账 CONTROL 卡「验收这一段」→`haltAction`。 |
| `resume-low-progress` | low-progress CONTROL 卡「再开一个有界窗口」→`haltAction`。 |
| `continue-reconciliation` | reconciliation CONTROL 卡「继续（不加配额）」→`haltAction`。 |
| `change-anchor` | reconciliation CONTROL 卡「方向要改」→`haltAction` 的方向句输入。 |
| `add-conversation-budget` | conversation-cost CONTROL 卡「增加本对话预算」→`haltAction`。 |
| `add-daily-budget` | daily-cost CONTROL 卡「增加今日预算」→`haltAction`。 |
| `owner-stop` | 各可操作 CONTROL 卡「停一下/停止」→`haltAction`。 |
| `owner-resume` | owner-stop CONTROL 卡「恢复这条对话」→`haltAction`。 |

后八项由服务器实际 CONTROL `actions[]` 渲染在 `drawFeed`，只在对应状态出现。全房间 `stop/resume` 是另一条 `/api/halt/room`，输入工具条的「■/▶」由 `haltRoom` 触发。

## 其它 `/api` 路由逐项核对

| 服务器路由与方法 | 界面入口，或不设独立入口的理由 |
| --- | --- |
| `GET /api/state` | `tick` 定时刷新页面、目标/席位/队列/项目状态；这是数据源，无手动按钮。 |
| `POST /api/send` | 主输入框「↑」，新目标/无显式目标时 `send`。 |
| `POST /api/conversations/:id/send` | 主输入框选席位发送、右侧旁聊「发送旁聊」；`send` / `sendSidechat`。 |
| `POST /api/conversations/:id/transfer` | 旁聊回复上的「投递给主控」「立即插话」；`transferSidechat`。 |
| `POST /api/conversations/:id/read` | 左轨目标切换/看板卡片进入时 `selectGoal` / `enterGoal`，清除未看标记。 |
| `POST /api/deliveries/:id` (`retract`/`interject`) | 原消息排队条「撤回」「改中途插话」；`deliveryAction`。 |
| `POST /api/seat/:id/interject` | 主输入框显式「改为中途插话」后的发送；`send` 绑定 active turn。 |
| `POST /api/halt/room` (`stop`/`resume`) | 输入工具条「■/▶」；`haltRoom`。 |
| `GET /api/m7/evaluate` | 无独立按钮：房间在对账 RESPONSE/CONTROL 自动写 M7，界面显示其裁决；此 GET 供机器独立核对相同裁决，增加手动“重判”按钮会误导其权威来源。 |
| `POST /api/rooms`；`PATCH /api/rooms/:id` | 左轨「新建房间」及房间菜单的重命名/固定/归档；`createRoom` / `patchRoom`。 |
| `POST /api/project/open-directory`；`PATCH /api/project` | 项目菜单「打开项目目录」「重命名项目」；`openProjectDirectory` / `renameProject`。 |
| `GET /api/projects` | 无单独读取按钮：项目列表已随 `GET /api/state` 到达左轨；此 GET 是同一注册表的只读 API。 |
| `POST /api/projects/pick-folder`；`POST /api/projects/open`；`POST /api/projects/create` | 项目菜单「打开本地项目」「新建项目」的文件夹选择和确认；`browseProjectPath` / `submitProjectPath`。 |
| `POST /api/projects/switch`；`PATCH/DELETE /api/projects/:id` | 左轨选择项目、项目菜单重命名/归档/移除；`switchProject` / `renameProject` / `archiveMenuProject` / `removeMenuProject`。 |
| `GET /api/environment` | 右侧「仓库」「运行」页签及环境刷新；`refreshPanelEnvironment` / `openEnvironment`。 |
| `GET /api/accounts` | 建席位表单的账号/本机 CLI 只读数据源；`loadAcct`，不另设入口。 |
| `POST /api/accounts`；`POST /api/accounts/:id/{login,probe,remove}` | 无界面写入口：旧账号存储兼容 API；当前产品登录、检测、移除统一在「本机 → 实例管理」，不恢复会绕开实例池的第二条登录/删除路径。 |
| `GET/POST /api/instances` | 「本机 → 实例管理」列表/新建；`loadInstances` / `instNew`。 |
| `POST /api/instances/:id/{login,login-status,cancel-login,logout,probe,rename,remove}` | 实例卡「登录/查进度/取消/清除登录/检测/重命名/移除」，均由实例设置函数触发；轮询 `login-status` 不需要独立按钮。首次独立审计发现 `instWatchLogin` 曾误发 GET（服务器只接 POST）；本次改为 POST，并以真实页面处理函数测试 URL、method、进度和授权链接。 |
| `POST /api/instances/:id/test-put-session` | 仅 `GOGO_TEST_HOOKS=1` 的测试注入，正式 Room 为 404；无产品入口。 |
| `PATCH /api/human` | 右侧「席位」第一行用户席位编辑/保存；`saveHuman`。 |
| `POST /api/seat-draft/probe` | 新建席位表单「检测这个实例」；`newSeatProbe`。 |
| `POST /api/seat-draft/{login,login-cancel,login-input,login-status}` | 已撤销，服务器固定 410；登录只在实例页，故不加入口。 |
| `POST /api/seats`；`PATCH/DELETE /api/seats/:id` | 「席位」页签/新建抽屉的建立、保存、移出；`saveSeat` / `removeSeat`。 |
| `GET /api/seat/:id`；`GET /api/seat/:id/runtime-context` | 右侧席位详情和终端运行上下文；`openSeat` / `selectTerminalSeat`。 |
| `POST /api/seat/:id/{start,stop,restart}` | 席位列表/详情「启动/停止/重启」；`seatCtl`。 |
| `POST /api/seat/:id/provider`；`POST /api/seat/:id/instance`；`POST /api/seat/:id/bind-instance` | 席位详情切 CLI/运行实例、首次引导绑定实例；`setSeatProvider` / `setInstance` / `bindSeatInstance`。旧 `setProvider` 名与实例列表页签冲突，已改成独立函数名。 |
| `POST /api/seat/:id/probe` | 无独立入口：席位级旧登录状态探针；产品统一用实例卡 `/api/instances/:id/probe` 与新席位 `seat-draft/probe`，避免同一登录状态出现两条按钮。 |
| `POST /api/seat/:id/{login,login-cancel,login-input,login-status}` | 已撤销，服务器固定 410；席位侧不允许操作登录。 |
| `GET /api/env/status`；`POST /api/env/node/{install,recheck,policy}` | 首次引导环境页「检测/安装 Node/我已装好/交给 IT」；`loadEnvStatus` / `installNode` / `claimNodeInstalled` / `markCompanyBlock`。 |
| `POST /api/env/cli/install`；`POST /api/env/cli/custom-path`；`POST /api/env/restart` | 首次引导环境页安装 CLI、填写自定义路径和准备后重启；对应引导按钮函数。 |
| `GET /api/env/cli/install-status` | 无单独按钮：`GET /api/env/status` 已返回同一安装会话进度，环境页从那里展示；保留细粒度只读兼容接口。 |
| `GET /api/onboard/status`；`POST /api/onboard/draft` | 首次引导的当前步骤/草稿；`refreshOnboard` / `saveOnboardDraft`。 |
| `POST /api/onboard/pick-folder`；`POST /api/onboard/workspace` | 首次引导选择并确认项目文件夹；`pickProjectFolder` / `applyWorkspace`。 |
| `POST /api/onboard/create-seat`；`POST /api/onboard/try`；`POST /api/onboard/skip-try`；`POST /api/onboard/verified` | 首次引导建立主控、试一下、先跳过、确认试跑；`createFirstSeat` / `tryFirstSeat` / `skipTry` / `watchTryResult`。本次第一枪明确不点试跑。 |
| `/api/test/{active-turn,pending-fanout,pending-fanout-count,provider-family,reconciliation-latch,barrier,drain-pending}` | 只在 `GOGO_TEST_HOOKS=1` 暴露的测试控制与观测；正式 Room 无界面入口。 |

逐项核对后，唯一缺少的正常 Owner 主路径动作是 `request-reconciliation`，本次已补工具条入口。机械检查还发现页面内两组同名函数：空白态 `quick(i)` 被设置页卡片 `quick(icon,title,sub,page)` 覆盖、席位切 CLI 的 `setProvider(id,provider)` 被实例列表过滤函数 `setProvider(id)` 覆盖；本次把前两者分别改为 `quickSuggestion`、`setSeatProvider` 并保留原可见入口。其它无直接 `fetch` 的路由均由 `/api/state` 聚合读取、现有 CONTROL 动作渲染、测试 hook、撤销的登录旧路或实例/环境兼容读口解释；不另加会误导权威或扩大登录面的按钮。
