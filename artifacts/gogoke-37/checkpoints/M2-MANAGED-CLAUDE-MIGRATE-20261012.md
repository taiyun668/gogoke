# M2：首次自管 Claude 来源迁移

## 改了什么与结果

PR #88 的 main 授权已精确读回：scope digest 与回执、MANIFEST、三处候选固定值一致。其授权的错误原文接线已交付。

当前实验候选按冻结并验签的原字节正常换装。真实 USER 先停止并读回秘书长 H，再释放其占用。首次自管 Claude 安装完成原包 staging 与版本探针；原 MIGRATE 返回 `GOGOKE_MANAGED_CLI_NATIVE_MIGRATE:ERR AccessDenied`。整体安装失败，原请求没有重发，未调用模型或登录。

原失败窗口通过一次正常关闭退出；原 shell/native 句柄均报告退出码 0。十三项正式保护相等，原失败和未确认业务记录保留。一次性任务逐项导出后清理并回读不存在。候选证据直接保存在 D 盘。

普通视图对同一物理闭库作只读读取，前后数据库及 sidecar 字节不变：CLI 副本为 PROBED，版本与 binary pin 相同，尚无来源关联。迁移首先遇到四条历史 UNKNOWN 的 H open/stop 记录；相应原 open 已有独立 Claude holder release 的 APPLIED journal 与原文完全相符的回执。另有一条 PREPARED stdin 属于另一个已有独立释放回执的原 open。当前迁移资格只接 Codex 的已完成释放证明，未接 Claude。

还有一个旧 Claude failed-resume 子 episode 保持 UNKNOWN、没有 StopFact，且没有 Claude 专用的完成资源释放证明。它与三个已释放原 open 分开判断；进程缺席、缺 init 或原错误中的 `custody: Ok(())` 不能当作停止或撤权完成。

直接读取该子代原登记的实例 HOME、会话 HOME、SINGLE F 根对象安全描述符，三个原 FileID 均匹配，目标 SID 在每个根仍有一个原允许 ACE。因此不能采用“已无目标 SID”的只读资格；必须走现有精确 retirement 的持久原 intent、逐项撤权和读回。没有修改 ACL，也未读取凭据内容。

首次 ACL 读取的仪器错误已保留：把 SINGLE 错认成必有 member 行，inner join 误报缺项。生产 `graph_query` 对已登记且没有 member 行的 F 合成 SINGLE。按同源实现修正测量并独立复核后取得上述原 DACL，未改变产品来适配错误仪器。

## 下一步与决定

现有独立 Claude 完成证明已接到首次同 pin 资源迁移资格，保留 UNKNOWN/PREPARED 原值；对子代按其确切原 tuple 与三棵权限树复用 retirement，不借父代停止证明结算子代。父代 claim 与真实 StopFact 保持原值，子代仅取得独立资源完成依据。未取得原资源释放证明仍拒绝。不重发模型，不补写 StopFact/RECEIPTED，不手工改数据库，不借 Codex 凭据方案。

独立聚焦复核指出：子代完成回执不能代替当前目标 SID 的读回；历史正常停止的 resume peer 还须核完整原父链。两条前提都与生产行为一致，对本次权限资源资格有直接意义，据此补同一 pid/创建时间缺席、逐对象 DACL 读回与同一历史 episode/custody/H 回执链。原 resume raw 的 expected revision 连接父 stop receipt；没有采用会被 UNKNOWN→ACTIVE 恢复更新的 episode.previous_revision，独立复核已确认这个区别。并补原 stop request ID 与 pending stdin expected revision 的关联及拒绝漂移用例。

代码通过本机开发类型编译；两项局部安全开发测试分别为 1 passed、0 failed、0 ignored。它们只验证原记录关联与元数据父边，不是云端或装机证据。独立聚焦修订复核未发现阻止推送的问题；正式云端与实际候选尚未验证。无认证的现有测试只有真实 initialize ACK，不能制造厂商会话来凑子代阳性；子代成功撤权以实际候选与旧原记录的读回为准。

这是同一已授权 F/H 资源与同 DB 的必要产品组合细节，由 Controller 处理；没有改变用户、权限、CLI 或授权范围。并行只读根因席位核旧子代，独立席位复核接线。无须 Owner 操作。实际自管对话、完整 M2/M3 与稳定点完整链仍未完成；SAC 为关闭状态，本次不能作为 SAC 强制模式的通过证据。稳定点安装前仍需 Owner 开启 SAC。

## 参照

查过 gogo-party 的 `packages/room/src/accounts.ts`、`runtime-context.ts`、NaveHQ 的 execution-session 绑定与 authority-effect 分离，以及 LoomOS 目录及源码检索；未找到可直接替代此 Windows LPAC 三树 retirement 的实现。还查了仓库 `reuse-blueprint`、`upstream-reference-map` 与 substrate 源码拆解。历史实现区分当前持有者与历史会话，不能代替本产品的资源完成事实。采用现有 H 原回执、Claude holder recovery/ACL retirement、Job 与 RootLock 生命周期，以及已批准的 holder-disappearance 组合；不新增外围探测框架。与一般 session resume 不同，本次仅判断首次同 pin 自管来源迁移资格，旧 continuation 与需要原 StopFact 的操作继续拒绝。

## 节奏

PR #54 已按时补发进度，本时段新增可运行能力为无；构建、签名、安装与部分阶段不计新能力。公共 reader 的合法重复 init 修正已有受影响语法与卫生云检；全量检查未运行，不计通过。没有等待外部条件，根因接线与独立复核并行推进。
