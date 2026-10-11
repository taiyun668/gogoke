# V08 四个 Host 场景实测

改了什么：reader 沿真实生产者校普通 resume 的 process episode、NativeV2 effective seat 的授权代次与 H 进程代次，以及释放后 E 的 IDLE 代次自增；旧 checkpoint 绑定原 reader，当前校正 reader 独立记哈希与变更标记。补原消息正文的精确 echo 参数，保留唯一 started/completed 回显和额外工具拒绝条件。未改产品、原请求或数据库。

结果：原已装旧候选的四个新测试 USER 席，在同一已登录 Codex 实例执行十三项基础规则与自动投递、忙碌排队、路由变更、取消四个 Host 场景；共 197 次操作、六次正常关闭退出 0。两源会话与目标的真实 StopFact、释放及 typed A/H/C/E/F 链由校正 final 读回，`directCaseEvidence=true`、数据库测量字节不变。正式版保护读回通过。原失败 journal 和各次失败 reader 输出均保留，未重放产品请求。独立实测交叉核对相符；通过的 reader 与原加载模块按当时实际 SHA 归档，后续工具提交不冒充该次字节。

原仪器失败原文：`Expected one original row, found 0: SELECT * FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?`；随后旧 binding 表同类零行；`Queued artifact is another subject`。它们分别用了错误写入表和错误仪器哈希关系。普通 resume 的实际 episode、原 NativeV2 selection/effective view、旧 checkpoint 自身字节链仍严格核对。

边界：完整 V08 仍未通过。外项目模型边界、下属直达 Owner、真实 stalled 链、路由变更后的 late ACK、busy-to-idle 自动投递均保持 NOT_RUN。旧候选实测不替代新候选和稳定点复测，不接受 M2/M3。

下一步：独立交叉核原闭库证据；集成工人可选 busy-to-idle 场景并用新独占对象实测。G 持续加载修订已交 Claude，主控继续宿主集成与真实端到端；新候选原字节的 SAC 观察独立保留。

参照：`v37_runtime::dispatch_native_resume`、`session_transport/episodes`、`session_binding::EFFECTIVE_SEAT_SCHEMA`、`admission::release_native`、E 的 `set_dispatch_state_in_transaction`、现有 `verify_recipient`、原 baseline/checkpoint/module 引用链。两独立席位分别核仪器来源和 NativeV2 代次，未用 legacy 合成表当真实产品事实。
