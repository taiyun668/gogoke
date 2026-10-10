# M2：53 原启动失败与恢复施工

## 改动

普通视图已安装 0.1.53，冻结字节与安装身份一致，正式版及实例保护读回通过。旧 Claude 席位的 BUSY preflight 拒绝发生在新 H 启动前；原记录保留。既有已登录实例在 B 域的新 USER/SINGLE 工作树创建、登记、正常关闭及保护通过，不代表旧席位已恢复。

新席位第一次实际 initialize 仍失败。原错误是 `persistent frame deadline`，child stderr 为空。原 H stop 返回 Job 成员零、退出码 137，但 parent 未确认退出、stdout EOF 未确认及三条停止错误；未产生 StopFact 或释放信用。产品和原 custody 继续持有，未关闭、强杀或重发。一次脚本路径转义失败发生在产品启动前，原件保留，修正后才产生上述唯一真实请求。

服务原 journal 已有更完整的启动错误，界面只返回“已记录”；本次将已有 bounded 原文直接带入错误结果，停止失败也返回已有错误原文，保持原拒绝、权限和超时。独立只读 live 数据库测量返回 `unable to open database file`，保留为仪器失败，不改锁、不复制活库或忽略 WAL。

## 结果

Grok 原权限回包的六项受影响云端原生检查和卫生通过，产物及成员哈希已独立回读，代码集成到施工树。它尚未安装，不记为 53 的真机能力。完整 M2/M3、稳定链及最终验收未执行或未完成。

53 的受影响云检仅证明 EOF/reader/shim 检查；找到的固定 Claude 云端启动只运行 `--version`，不证明真实 initialize 路径。空 hook 日志不能区分未加载、未调用或日志投递失败，未据此改 CLI、权限或等待参数。

## 下一步

Sol 独立分支实现普通 Claude 原两 HOME 和可证明原 SINGLE 工作树的精确 SID 退休及 H/E CAS，主控负责共享接线。封存原值/目标值及各子对象身份，保留全部内容、其他 SID、UNKNOWN、RPC 和 episode，不伪造 STOPPED。公开 CLI/DLL 的原 RX ACE 保留为既有残余，不声明全部 ACL 已清。

main 计划要求真实 StopFact 后才能关闭候选；当前 stop 失败阻止换装。已一次请求 Owner 裁决是否允许这次正常关闭并保全，再以原生持有者消失路径恢复。答复前继续云端实现，不自行关闭或请求登录、Windows 重启。PR #54 已续报，同一次心跳不重复计数。

C 盘专项已完成 389 项逐项保全至 D，22 项因真实引用留原位；当前约 43.35 GiB 可用。所有新 scratch 与候选证据直接写 D，正式版未改。保全清单仍在既有 D 归档。

## 参照

采用既有 Codex 有限对象退休、原生 exact PID/创建时间、Job kill-on-close、同物理根排他协调及原 H/E 生产者；历史 gogo-party 进程身份收尾用于核对。普通 Claude 不具备 Codex 的凭据登记，不伪造登记以复用恢复。独立风险复核确认旧 session claim 不可重新 reserve、新 session/generation 的 profile 不同，因此不新增清除全部公开运行资产 ACE 的释放前提。当前缺少同路径云端 initialize 成功原件，明确保留该证据欠项。
