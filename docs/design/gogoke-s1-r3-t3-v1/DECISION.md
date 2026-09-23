# ADR-S1-R3-001 — 采用 T3-derived 单一主运行服务

## 决定

采用 `pingdotgg/t3code@d6f291303ddc0c9a14f570266a4d9eff6d431593` 作为 S1-R3 的 donor 基线，抽取并改造其 server/provider/orchestration 主体，在 Gogoke 内形成一个 T3-derived 主服务。

这不是移植 T3 UI，也不是把 T3 产品整体嵌入 Gogoke。Gogoke 继续拥有产品语义与界面；T3-derived 服务承担运行事实、provider 生命周期和持久编排机制。

## 选择依据

已核源码显示 T3：
- built-in drivers 同时包含 Codex、Claude、Grok、OpenCode、Antigravity（另有 Cursor）；
- ProviderAdapter 具备 start/send/interrupt/approval/user-input/stop/read/rollback/event stream；
- ProviderInstanceRegistry 以 driver + instance 分离多账号/多配置；
- orchestration event、projection 与 command receipt 同事务提交；
- side effect 由 intent 记录后的 reactor 执行；
- Windows 桌面和 server 有实际产品路径；
- MIT 许可。

这比从零重建多 CLI host/session/event/receipt 更贴近 Gogoke 的目标。

## 不采用为主权威

AionCore、OpenClaw、Herd、Herdr、Orca、AO、Paseo、Multica、Pi、OMP、Goose、OpenHands、Vibe Kanban、ShellX 等保留为机制/测试/适配参考，不与 T3-derived 服务并行拥有同一运行事实。

## 已知必须补齐的缺口

1. provider reactor 的部分 turn-start 去重是内存 cache，不能代替产品耐久 dispatch receipt。
2. provider pending approval/input 回调在 app restart 后可能失去 native callback state；Gogoke 必须保留 continuation / ACCEPTANCE_UNKNOWN / 待核对语义。
3. T3 默认 runtime mode 存在 full-access 默认；Gogoke 不继承为产品默认。
4. Windows provider subtree/custody、writer fence、residual custody 需要真实受控验证和薄 native launcher。
5. Gogoke Owner/private-audit/secretary privacy domain 必须在真实消息、历史、搜索、附件、工具和恢复入口执行，不是 UI 隐藏。
6. 远程/mobile/更新/安装等 T3 能力在 S1-R3 默认封存，不因 donor 存在而开放。

## 推翻条件

只有实际证据表明以下任一成立时才回 GPT：
- donor 无法在不重造主要 provider/session/orchestration 主体的情况下满足合同；
- Windows / 私域 / native 身份的共同要求无法安全实现；
- 许可或依赖闭包不可接受；
- 同条件候选有显著更小的真实补丁与升级负担；
- 新证据要求改变产品权威或能力保证。

普通编译和接线困难不是推翻条件。
