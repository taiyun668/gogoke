# K-POLICY：未设置流程阶段与初次编排的细节变更请求

状态：**Owner 2026-10-10 已认可最小修订；PR #86 经独立复核后由 Controller 合并，main 已精确读回**。请求线 E，集成 G；通知 K-POLICY 的 C/D/G 依赖线。不是里程碑验收。

## 直接前提与结论

原设计把 USER 的编排范围和用户定义的阶段/流转分开，没有规定首次建直属席位必须先设置流程阶段。原建 LEAD 路径只要 head revision 来记录限定 DISPATCH，不读取 stage；stage 只参与关口和流转。但原 policy head 把 stage 定为非空，当前 USER 项目登记没有 stage 输入。因此初次编排被当前数据模型耦合阻塞；不能用测试 OPEN 或虚构阶段填空。

#85 已授权只读原 head、沿原初始化，并明确不改变原政策表。以下方案超出这段已认可细节，先交 Owner 决定，不能以 scope 摘要不变代替裁决。

## 建议的最小方案

- head revision 与流程阶段分开；阶段未设置明确保存为 null，不伪造阶段名称。
- 仅显式 USER 项目/主控创建或设置编排范围时，可对确证不存在的 head 建立空政策元数据；默认 grants/gates/routes 全为空。读取和模型调用都不初始化。
- 已有 head 原样保留。空阶段 head 已存在时，旧具名 `policy-initialize` 仍按原冲突语义拒绝；旧成功请求精确重放只返回原事件回执，不解释为首次设置阶段。旧请求字节和撤销规则保持；已有具名阶段不能被首次设置命令覆盖。
- 用户以后定义阶段时，只允许对“尚未设置阶段”的 head 按原 revision 做一次 CAS 设置；后续流转仍经原关口。阶段缺值时关口及流转拒绝。
- 首次直属 LEAD 的 DISPATCH 仍只由原验证后的 USER scope、层级、实例/模型/权限/并发限制以及 H 物理调用证明导出；不添加宽泛默认授权。
- 旧库用精确 schema 迁移，保留所有原阶段、revision、grants、撤销、事件和回执，不重写旧业务内容；迁移不明或提交不明时拒绝并保留原错误。

## 最少文件与范围

| 对象 | 变更 |
|---|---|
| E 已授权 `store/seat/policy.rs`、`store/seat/mod.rs` | 可空阶段、原 schema 迁移、Owner 元数据/首次阶段命令、缺阶段拒绝 |
| 已列共享 `store/product_database/v37_seat.rs` | 原 head 读取报告 null；仅 USER 入口接上述命令 |
| 已列共享 `src/services/tauri.ts` | 明确读取类型和显式 USER intake 接线 |
| G 已授权 `src/features/seats/`（Claude） | 有需要时显示未设置与 USER 操作，不猜阶段 |

路径均位于 `apps/desktop/native-host/src/` 或 `apps/desktop/` 的既有授权范围内。`v37_policy.rs` 的权限表只有 revision/grants，不返回阶段；模型 wire、H、全局调度和新增写路径均不需要扩展。

按当前 main `verify_plan.py` 的实际范围字段，这种既有路径的 extension/契约细节修订可保持 scope digest 与回执不变；若最终改动触及范围字段，则按规则更新同 PR 回执。无新增登录、重启或验收触点。

## 修订后的验证

受影响云端安全边界检查：未设置阶段无默认 grant；未设置阶段关口/流转拒绝；模型不能初始化；旧具名阶段/原回执与撤销不变；USER 初始化 CAS 冲突只重读；旧 schema 迁移失败不改原记录。真机用已登录测试实例的新隔离项目，在未定义流程阶段时按明确 scope 建直属 LEAD，随后以明确 USER 阶段设置验证关口；正常 stop/release、正式保护。未运行不得写成通过。

参照：设计 37 的 USER 编排与用户定义阶段条款，原 E Owner policy/head revision/关口代码、原 USER ingress、main 范围摘要实现。采用原权限表与 CAS；仅解除数据模型耦合，不提供示例工作流默认值。

独立复核：全新 Sol 只读核对设计、源码、最少文件和范围计算，提案前提成立；要求明确旧初始化冲突与精确重放语义，已按上文补齐。未以静态复核代替 Owner 裁决或运行验收。

## 已认可后的施工记录

- main `820af46f47daa22f21cd2e64ac7f23138d7b1086`：scope digest 仍为 `593b8be6d6b41ba19255636a673441e2d3c7b4f1fd7140806229b6312f1b7490`，回执 blob 仍为 `e1197ca261fb10aa1e173d0d9754be7dbefa1493`；独立 Sol 实际校验 MANIFEST、来源 blob、自测均通过。三处签名链固定回执未改。已合入施工分支，仅计划三个文件差异，产品四域未因合 main 改变。
- E 采用原 Owner issuer、同库事务、原操作指纹与事件回执；新增空元数据和首阶段 CAS，null 阶段拒关口/流转，旧具名初始化保持冲突。五种精确旧 schema 的同一安全用例逐行保全非 1 revision、过期 grant、gate/route、事件、authority 撤销及旧重放；预存迁移中间表时拒绝且原行不变。本机开发诊断通过，正式云检待跑。
- 首次全轴 Astra 静态审计 R1–R9 完成，指出新 TS USER helper 回执校验宽松；已绑定 schema/command/requestId、严格字符串状态与确切 revision。独立 Sol 聚焦复核后无剩余阻塞。24 项安全边界检查加载整份真实 tauri.ts，仅宿主回包为模拟，不能称 native/Win11 通过。
- 通用“新席位”没有主控指定事实，不触发初始化；仅已明确 isLead 的 USER 编排范围设置接初始化，显式项目/主控 intake 可使用导出入口。首阶段 helper 尚未接 UI，未宣称首次项目/首阶段完整端到端接通。没有 G 目录修改。
- 既有事务回滚失败只保留 rollback 原因、可能丢此前操作原因，记为继承诊断限制，不作为新权限风险。E 本机诊断产物的递归清理被自动审批拒绝，原对象保留，不绕过。
- 下一步：受影响原生、Browser/真实 bridge 边界与卫生云检；稳定候选统一安装后，核实际当前旧 schema 的迁移和未设阶段的原生派活。候选、真机、正式门槛与验收 NOT_RUN；V10/V11 仪器和 M2 各线继续，不在检查点停工。
