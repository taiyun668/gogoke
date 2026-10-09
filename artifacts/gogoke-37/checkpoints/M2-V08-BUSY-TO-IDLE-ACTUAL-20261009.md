# V08 忙转闲自动送达实测

改了什么：集成可选第五场景，使用四个新独占 USER 测试席及原已登录实例。原问题 OPEN 后才记事件边界，原 completion 与正向 idle 出现后核同一会话的 Host 送达；不要求读取时仍为 idle，避免新通知已经 active 时误拒。默认旧四场景不变，选择第五场景时旧四明确 NOT_RUN。

结果：已安装旧候选本体完成 111 次操作、三次正常关闭退出 0。原 C 通知 PENDING，唯一原问题回答取得精确 native 写入回执；同一 H 代次的原 CLI idle、busy turn 完成均在 Host typed ACK 前，随后有唯一 Host send 和独立通知轮完成。接收者原 stop/release、StopFact、闭库测量和原仪器字节相符；`directCaseEvidence=true`，未改仪器重读。独立实测交叉复核通过，原工具字节已按结果 SHA 归档。

时序结论：本轮 idle 通知先于 busy completed；正确事实是二者均先于 Host ACK，不能改述成 completed 必须先于 idle。Host active 在 ACK 后，本轮符合严格 reader；未拿假协议或宽限补偿。

边界：仅第五场景实测成立，旧四在另一份原运行证据中已核。完整 V08 仍缺跨项目模型边界、下属直达 Owner、真实 stalled 链、路由变更后的 late ACK；稳定候选复测及 M2/M3 接受未执行。

下一步：当前普通视图只读核三家 provider 的原实例、USER 席和 F 绑定，历史 pin 仅作 expected，后续实际 H capability 核真实版本和摘要。G 加载修订已交 Claude；新候选原字节的 SAC 观察保留。

参照：原 `v37_host_rule`、`v37_host_idle`、QCard answer、H output、现成 checkpoint/stop/release/final reader。独立复核先查发现逻辑再修确定时序误判，未增加新权限、登录、模型替身或安装包。
