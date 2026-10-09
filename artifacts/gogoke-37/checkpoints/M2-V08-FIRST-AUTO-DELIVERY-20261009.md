# V08 首个自动升级投递片段

改了什么：闭库 reader 原要求成功 C stage 的原因字段为空，实际生产者始终保存 `H APPLIED` / `H REPLAYED`。现按生产者严格校验成功状态与说明，不放宽原请求、目标、payload、身份及原 H 回执条件。仪器修订保留原 reader / baseline / journal 引用和哈希，当前 reader 另报实际字节。

结果：已安装旧候选原冻结字节上的首个 REJECT_CAP 自动投递已发生；原 E 原因及 DELIVERED intent、C HOST_RULE 投递、H 已回执 send、原 CLI 完成轮次、三份真实 StopFact 相互对应。三会话真实停止后正常关闭，对相同闭库原件重读，数据测量字节不变；独立只读核对相符。原失败 journal 保留。

边界：仅该 checkpoint 片段通过。`directCaseEvidence=false`、`acceptance=false`；final 尚未核完整 Host case 的原 A typed ACK、H/F 身份、释放及无额外工具效果。完整 V08 未通过，其余排队、路由更改、取消和其他边界仍未执行。旧候选结果不替代新候选及稳定点复测。

下一步：按原 StopFact 释放这三份已停止 claim 并正常关闭，再继续剩余真实场景。系统准入受阻的新候选原字节保留，不重发原投递、不改原记录。V10 前提与 M3 只读 UI 工具并行推进；后者的私有期望文字不能替代原宿主事实，夹具按独立意见修正。

参照：既有 `inbox/host_rule.rs::record_host_stage_result`、`m2-rules.mjs`、`m2-rules-readback.py` 的 final 原仪器引用链、现成 H stop / release 和正常关闭。未加入产品补偿、恢复 API 或新 Owner 操作。
