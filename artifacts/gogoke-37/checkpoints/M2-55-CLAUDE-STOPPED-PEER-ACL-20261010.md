# M2：55 原实例恢复拒绝与正常停止历史 ACL 修正

## 改了什么

55 用云端冻结原字节完成普通 Interactive/Limited 安装；未重新打包、未公开发布。原 Claude 实例读取在新 H 或模型请求之前拒绝。产品正常退出零，正式文件、正式数据、快捷方式、登记及既有记忆保护核对通过；原失败、custody、episode、RPC 和空 StopFact 保留。

直接原库与实际 ACL 读回：Claude recovery journal 为零；复用的实例 HOME 有九个会话 package SID，分别对应两条未确认旧 H 和七条经真实 StopFact 正常停止的历史 H，没有陌生 package SID。原生启动会在复用 HOME 添加当前 SID，普通停止保留其 ACL；恢复却排除正常停止的 H，再把其仍在的 ACL 当作陌生身份拒绝。这是恢复前提与实际生命周期冲突。

修正仅将同实例、精确原 open/stop 请求、claim/episode/custody 三方一致非空 StopFact 的正常已释放历史 SID 列为保留 peer；它们不进入本次清退目标。封存其精确来源摘要，在待处理恢复、H 释放事务、完成回执读回时重新资格化并比对。陌生 SID、SID 复用或封存后来源变化仍拒绝；底层仍只删除已确认消失目标的 ACE，保留其他 ACE 的原字节与顺序。不扩大权限，不造 STOPPED，不读凭据。

下一实验候选号为 0.1.56，含上述真实产品修正。G 目录未改。

## 结果

- SAC 注册表实际读回为 0；已精确读回并沿用 PR #83 合入 main 的本机开发规则。原生编译、测试根、缓存和临时产物直接 D 盘，并行上限八；开发产物不进候选，不作正式证据。
- 同一新边界回归在未修产品模块上实际失败：`Claude disappeared holder: AclWitnessMismatch`，耗时 43.20 秒。修正后同一回归通过，另三项本机诊断通过：陌生 package SID 拒绝、封存后历史 stop 字节漂移拒绝、原 UNKNOWN open 及原记录保全。
- 历史停止由真实固定 Claude 子进程和产品 stop/release 生产者产生；ACL、进程身份、E/F/H 来自真实生产者。登录 presence 和后续 UNKNOWN/PREPARED 持久形状明确是夹具投影，不是认证、模型超时或已安装原实例恢复成功。
- 独立 Sol 聚焦静态复核未见阻塞；受影响正式云测尚未执行。正常 APPLIED open 加 StopFact 证明原身份及停止事实，单独不证明握手成功。
- 本次正常退出、安装、构建和本机诊断均不计为新增真机可运行能力；同一次心跳计数仍为二。完整 M2/M3 和稳定链未通过，未自行验收。
- 两项新诊断任务结束后，按确切 action、主体、脚本摘要与实际退出码保全 XML/log，注销并逐项回读不存在。独立组件的真实 Git cwd 已收到 ACK，但空 HOME 结果不能外推原已登录 H。

## 失败原文与仪器

55 原产品：`GOGOKE_DESIGN37_NATIVE_USER_OPERATION_FAILED:ERR V37StoreFailure("Claude disappeared holder: AclWitnessMismatch")`。全部原件保留，不重发失败请求，不重新登录，不复制数据库或实例 HOME。

实际安装前的路径、证据目录和可选字段测量失败均在安装器或产品启动之前；原件保留，修正测量后重跑。一次元数据仪器报告 `Readonly DB observer source drift`，未保存比较值，不推定其原因；确认无产品进程、WAL 与 rollback journal 为空后，沿用现有关闭后只读观察法，原库字节与前后元数据核对通过。

恢复源码的文件复制保留了旧时间戳，Cargo 一次复用了负控二进制；尚未将它运行成修正结果。保留该缓存日志，重新写入同一修正字节触发真实编译，确认修正二进制与负控不同后才执行四项诊断。没有把缓存替身当修正代码通过。

独立组件夹具与私有构建目录的递归清理被自动命令审批拒绝，已保留在 D 盘；不换命令绕过。旧候选的确认空钉住根继续保留待审。

## 下一步与参照

先提交正式修正并跑受影响云端检查，再冻结下一实验候选，走原字节验签、实际安装和原实例复现。已并行准备 Grok 的新真实权限回包读回，只拒绝当前新请求，不重放旧 cursor；宿主拒绝写入不能冒充 OS ACL 拒绝，OS ACL 完整轴仍 NOT_RUN。继续主动读取 PR #54 的 Claude 交付。

参照现有 `session_transport/launch.rs` 的会话 SID 派生与绑定树 ACL 添加、`v37_runtime.rs` 的真实 stop/StopFact、`claude_retirement.rs` 的逐对象原字节删除，以及既有双 H/UNKNOWN 恢复组合；沿用普通候选安装、正式五组保护和产品 CDP 硬断言链。历史 gogo-party 的生命周期与 root/home 约定作为已有调研背景，本修法采用当前原生生产者的精确事实，没有新建补偿等待或重试机制。稳定点实测与验收前先提醒 Owner 重新开启 SAC，并实际读回 1。
