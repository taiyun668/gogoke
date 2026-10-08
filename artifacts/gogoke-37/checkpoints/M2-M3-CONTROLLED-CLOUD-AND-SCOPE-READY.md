# M2/M3：受控云检与最小范围待合并

## 已完成

PR #54 第 1 条的无 Now 数据误提示已由 `e53351a7` 删除，第 2–7 条共享挂载修订为 `9e4094b3`；两者均在实际安装的 0.1.40。真实启动身份与冻结输入相符，初始无 Now 事实的 Home 没有该提示；没有把这一个页面外推为所有真实对话均已实测。

自管副本修正 `7cec2712` 已集成到施工冻结源 `97ab5584`（0.1.41）。原生库 `37824666430` 为 790/790、LPAC 2/2；新 Sol 聚焦和 fresh Astra 全轴源码审计无确认阻塞。仅原 profile 明确停用的旧 M1 保留无绑定，其他合格实例须原子绑定；新原生拒绝字段已接入实例页 ERROR/raw。旧记录、H proof、HOME、凭据不被改成虚构 STOPPED。

普通视图闭库基线 PASS：正式五组、全部保留 native 身份及 custody 一致，未读凭据。两次测量缺辅助文件的原失败保留；补齐既有辅助文件后重跑通过。三项完成的一次性基线任务逐项核对、删除并回读不存在，证据文件保留。

## 外部条件与下一步

PR #77 当前 head `da0c5c35` 已独立只读复核、六项 CI 全绿、转 Ready，结论可合并。它只增三个普通对话 hooks 与一个既有 native effect helper 的精确 INTEGRATOR 范围。Owner 合并一次、精确读回 main 后才施工；没有先改这四个路径。scope digest 为 `b2d36f6d55865d1c40168457dbd04989669aaec9836c1a065bcc58071457fabd`，回执 blob 为 `040be1d40ee317bd6c5960cbf83df7989dfc00b5`，与三处签名链固定常量一致。T00 明确区分 main 中存在的消费者和只在未集成 shadow 存在的 helper。

候选 41 自动 PR desktop `37828992860` 是集成检查，不能被冒充为可签名候选。原签名链明确只接受 workflow_dispatch；已补派同一冻结源的受控完整构建 `37834241027`，源码和版本未改。当前 native full `37829006174` 的 server 658、focused 43、installation/legacy 变异组已有成功原件，其他 formal 与整库尚未齐；process 步骤成功不替代待上传的原 machine evidence。原 790 项库组件仅在 native tree、夹具、锁文件、CLI/Node setup、workflow/tools CI 输入逐项同字节后限定复用，不复用旧 40 的受影响 formal。

验签、同字节装卸、41 实装及原 M2 实例复用仍 NOT_RUN。如授权 main 在候选链完成前变更，重新读回并按现行签名资格安排新来源，不弱化范围或祖先检查。没有公开发布、main/PR #54 合并或自行接受。

## 已并行开工

M3 原事件消费者 `ac8607a5` 的完整 desktop `37824862842` 五项 SUCCESS，native 同树原库为 797/797、LPAC 2/2。最终审计确认“读错误后真停止挡住 resume”的消费者缺陷；共享接线修正为 `ac690fc7`，新 Sol 聚焦静态无新增阻塞，受影响完整云检 `37831916755` 正在运行。仅宿主原 StopFact 允许恢复入口越过旧读错误，H/A 与新代原来源校验成功后才清错；未知送达仍不在同代重播。真实安装 M3、普通 USER 对话及接受仍 NOT_RUN。

## 参照与走法修正

采用已有 USER effect、H/A 原回执、同锁 attachment 快照、原 intent journal、现有物理 StopFact 和旧 reader 隔离。真实 USER entry 已优先 dispatch_visible_effect，后面的只读 UNSUPPORTED 分支不可达；不能据此重复实现 finite 写操作。thread/start 缺新会话身份仍明确未完成。新 effect 的 workspace 当前选择约束只落首次请求，历史 replay/recover 保持原 action/selection 证明。

范围清单曾按工作区 CRLF 计算 SHA，已改为提交 blob 原始字节并独立复核；记录不沿用无效旧结论。候选派发应先读 trusted signer 的 event/来源条件，再开稳定构建；本轮漏做这一步造成一次多余自动检查，未修改被冻结产品迁就仪器。

本记录在独立证据分支提交，施工源保持 `97ab5584`。临时工作树因未合并范围、在飞云证据与可续接状态保留；完成后按确切对象清理。
