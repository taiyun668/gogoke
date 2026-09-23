
## 11. Pass C R3 — C2-F01 的逐符号与身份回写补充

本节只补 B03；R2 中 B01/B02/K16/K17 的证据关闭结论及原条款保留。
`instances.ts` 实际符号是 `InstanceStore.noteProcess`（325–338），不是 `confirmOwner`；`server.ts` 是 `ensureSeat`（1488）和 `ensureSeatInner`（1498），不是 `startSeat`。
精确声明、源 blob、行号、声明摘要和15条调用边见 `p00-pass-c-c2-f01-r3/symbol-bindings.json`。行为描述与真实声明分开，`run/login/logout` 不再被默认为同名函数。
实际链为 ensureSeat → ensureSeatInner → claim → seat.start → noteProcess → processStartedAt/save，异常链为 ensureSeat.catch → releaseAllFor → save。noteProcess 的 void/查询为空不是所有权确认或已退出证据；内存身份更新先于可失败的回写。claim 保存失败还可能先于 claim.instanceId 赋值。releaseAllFor 不检查进程退出，保存失败会发生于内存清除之后，并可能覆盖原始异常。
上述分支、保留残留身份/旧writer隔离的要求及首验承接见 `p00-pass-c-c2-f01-r3/ownership-contract.json` CJ-O01–04。它细化既有 F24/F25、RT32、W30、Q14/Q15，不把 Room/InstanceStore 收编为新权威；WP01 typed合同、WP02 T39.L/T55.L、WP03 T32.L/T50.L/T51.L/T52.L/T63.L、WP04 T11.L/T49.L 义务保持。
本次仅修证据；真实 CLI/PID/文件锁/磁盘耐久性均未执行，不得宣称原行为已安全修复。
