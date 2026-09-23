# S1-R3 架构合同

## 1. 产品边界

公开链路：

Tauri/React → 当前用户私有 IPC → 一个 T3-derived 主服务 → provider drivers/native CLI。

Tauri 不直接写运行数据库、不直接启动 provider。旧 gogoke_daemon 只允许 legacy-only；public-only 路径由新服务承担。二者不得同时控制同一 data root、provider profile 或进程。

## 2. 唯一事实所有者

| 事实 | 唯一权威 |
|---|---|
| Owner grant、Seat、privacy domain、产品 Session | Gogoke policy repository（主服务内） |
| Thread/turn provider-native identity | 对应 native CLI；主服务保存绑定与观察 |
| Execution / action / dispatch / acceptance | 主服务持久事务 |
| Provider session/runtime event | T3-derived ProviderService/Adapter 映射 |
| Delivery / continuation / approval response | 主服务持久 action/continuation |
| Project/task/delegation | Gogoke policy + 主服务持久对象 |
| Process/profile custody | 主服务调用的统一 native custody layer |
| Event cursor / snapshot recovery | 主服务 DB |
| UI | 只做投影与可信请求，不拥有事实 |

禁止建立另一套同义 Session/Execution FSM 与 donor 双写。

## 3. 数据与事务

复用 T3 的 SQL/event/receipt 思路，不继续旧 S1 自定义 journal。

产品扩展表至少覆盖：
- domain / grant / seat / binding
- action / dispatch attempt / acceptance evidence
- continuation / approval / question
- product event cursor / outbox
- custody / writer fence
- capability observation

可信请求先核 principal/domain/generation/grantRevision，再在事务中记录 intent/event/receipt，最后 reactor 执行外部副作用。

同 operationId 只有在 domain、payload hash、target、binding generation 全部一致时才可返回历史结果；同键异内容必须冲突。

## 4. 接受未知

外部 native CLI 的 side effect 不是数据库事务的一部分。

一旦 durable DISPATCHING/attempt 已落盘，而可靠 native acceptance 尚未取得就发生断线/崩溃，恢复为 ACCEPTANCE_UNKNOWN。不得普通重试制造第二个执行。

只有：
- native 协议提供可信幂等键；或
- 可通过只读查询确认原动作；
才允许自动解析。否则要求停止/隔离旧执行并显式创建新 operation。

## 5. Continuation / approval

approval/question 与普通 prompt 分离。continuation 绑定：
requestId、actionId、nativeRequestId、principal、domain、binding generation、expiry、state。

回答与取消在同一串行化点决胜：
- 未承诺外发可撤销；
- 已承诺后取消进入待核/停止；
- 重复、错域、错世代、过期和终结请求拒绝；
- restart 后 native callback 丢失时不得伪装“仍可直接答”，应进入待核/恢复合同。

## 6. Provider / Instance

driver 与 instance 分离。每个 instance 固定：
driver、binary identity/hash/version、profile/auth revision、runtime mode、capability source。

同一 driver 可多个实例，状态不得因品牌相同而合并。账号切换提升 identity/profile revision；旧队列不得静默使用新身份。

五个首要 driver：
codex、claude-code、grok、opencode、antigravity。

descriptor/registry/probe/handshake/真实动作是不同证据级别；缺证据用 unknown，不把 PATH 命中当 ready。

## 7. 权限与私域

Owner、项目主控、项目审计公开委托、Owner-审计私聊、秘书长个人域分别有独立 scope/privacy domain。

policy read failure = deny。

消息、搜索、历史、附件、导出、日志、通知、工具调用、resume、provider launch 都必须核域。撤权后不能声称模型忘记已见内容；必要时封旧 binding，以获准材料冷重建。

模型正文、文件里的 @、owner_approved 字样、角色字符串均无控制权。

## 8. Process / custody

建立薄 native custody layer，所有 donor provider/SDK/helper 子进程最终从同一准入 seam 获取：

launch intent → 精确 binary/profile identity → OS process handle → Windows Job/Unix 可控 group → durable custody → provider initialize → ready。

Windows 专用受管路径：
- CreateProcessW(CREATE_SUSPENDED)
- AssignProcessToJobObject
- 保存精确 handle/start identity/custody
- ResumeThread
- initialize

禁止把 CREATE_SUSPENDED 粗暴加到所有通用短命 probe。

停止分 interrupt_execution / close_binding / shutdown_host。
成功必须有 StopProof：direct child、Job/known descendants、writer fence、durable receipt、errors。terminate 调用成功、exit0、EOF、字段清空都不能单独证明完整释放。

无法确认时保留 residual custody，并拒绝第二 writer/profile owner。

## 9. IPC

Windows：当前用户 SID 限制 DACL 的 named pipe，拒绝 remote client；Unix：0700 目录 UDS + peer uid。

连接有 hostEpoch/version handshake；worker 没有 Owner 管理凭据。跨 await/排队后在副作用前重查授权。

S1-R3 不开放 T3 自带远程/mobile/http 管理面；若 donor 组件默认会启动网络监听，必须在构造前关闭，而非 UI 隐藏。

## 10. Wave A 处置

KEEP：remote/voice 封存意图、公共产品语义、unknown/custody 类型、codec 负例及失败日志。

ADAPT：公共类型到 donor contract 的映射、边界检查、本地桥、测试目标。

REPLACE：旧计划中尚未获得独立验收的 custom journal、重复完整 host/delivery 控制面、强制 TS worker。

任何删除必须逐文件证明等价行为，不做批量“因为换内核就删掉”。

## 11. 非目标

本阶段不做真实用户迁移、真实 provider 登录/费用、remote/Tailscale、语音/model、安装/更新/registry、签名发布、完整 S2/S3。
