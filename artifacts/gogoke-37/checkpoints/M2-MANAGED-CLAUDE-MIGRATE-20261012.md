# M2：首次自管 Claude 来源迁移

## 改了什么与结果

PR #88 的 main 授权已精确读回：scope digest 与回执、MANIFEST、三处候选固定值一致。其授权的错误原文接线已交付。

当前实验候选按冻结并验签的原字节正常换装。真实 USER 先停止并读回秘书长 H，再释放其占用。首次自管 Claude 安装完成原包 staging 与版本探针；原 MIGRATE 返回 `GOGOKE_MANAGED_CLI_NATIVE_MIGRATE:ERR AccessDenied`。整体安装失败，原请求没有重发，未调用模型或登录。

原失败窗口通过一次正常关闭退出；原 shell/native 句柄均报告退出码 0。十三项正式保护相等，原失败和未确认业务记录保留。一次性任务逐项导出后清理并回读不存在。候选证据直接保存在 D 盘。

普通视图对同一物理闭库作只读读取，前后数据库及 sidecar 字节不变：CLI 副本为 PROBED，版本与 binary pin 相同，尚无来源关联。迁移首先遇到四条历史 UNKNOWN 的 H open/stop 记录；相应原 open 已有独立 Claude holder release 的 APPLIED journal 与原文完全相符的回执。另有一条 PREPARED stdin 属于另一个已有独立释放回执的原 open。当前迁移资格只接 Codex 的已完成释放证明，未接 Claude。

还有一个旧 Claude failed-resume 子 episode 保持 UNKNOWN、没有 StopFact，且没有 Claude 专用的完成资源释放证明。它与三个已释放原 open 分开判断；进程缺席、缺 init 或原错误中的 `custody: Ok(())` 不能当作停止或撤权完成。

迁移修复前直接读取该子代原登记的实例 HOME、会话 HOME、SINGLE F 根对象安全描述符，三个原 FileID 均匹配，目标 SID 在每个根仍有一个原允许 ACE。因此不能采用“已无目标 SID”的只读资格；必须走现有精确 retirement 的持久原 intent、逐项撤权和读回。该诊断只读，没有修改 ACL，也未读取凭据内容。

首次 ACL 读取的仪器错误已保留：把 SINGLE 错认成必有 member 行，inner join 误报缺项。生产 `graph_query` 对已登记且没有 member 行的 F 合成 SINGLE。按同源实现修正测量并独立复核后取得上述原 DACL，未改变产品来适配错误仪器。

## 下一步与决定

现有独立 Claude 完成证明已接到首次同 pin 资源迁移资格，保留 UNKNOWN/PREPARED 原值；对子代按其确切原 tuple 与三棵权限树复用 retirement，不借父代停止证明结算子代。父代 claim 与真实 StopFact 保持原值，子代仅取得独立资源完成依据。未取得原资源释放证明仍拒绝。不重发模型，不补写 StopFact/RECEIPTED，不手工改数据库，不借 Codex 凭据方案。

独立聚焦复核指出：子代完成回执不能代替当前目标 SID 的读回；历史正常停止的 resume peer 还须核完整原父链。两条前提都与生产行为一致，对本次权限资源资格有直接意义，据此补同一 pid/创建时间缺席、逐对象 DACL 读回与同一历史 episode/custody/H 回执链。原 resume raw 的 expected revision 连接父 stop receipt；没有采用会被 UNKNOWN→ACTIVE 恢复更新的 episode.previous_revision，独立复核已确认这个区别。并补原 stop request ID 与 pending stdin expected revision 的关联及拒绝漂移用例。

代码推送前通过本机开发类型编译；两项局部安全开发测试分别为 1 passed、0 failed、0 ignored。它们只验证原记录关联与元数据父边，不是云端或装机证据。独立聚焦修订复核未发现阻止推送的问题；当时正式云端与实际候选尚未验证。无认证的现有测试只有真实 initialize ACK，不能制造厂商会话来凑子代阳性；子代成功撤权以实际候选与旧原记录的读回为准。

这是同一已授权 F/H 资源与同 DB 的必要产品组合细节，由 Controller 处理；没有改变用户、权限、CLI 或授权范围。并行只读根因席位核旧子代，独立席位复核接线。无须 Owner 操作。实际自管对话、完整 M2/M3 与稳定点完整链仍未完成；SAC 为关闭状态，本次不能作为 SAC 强制模式的通过证据。稳定点安装前仍需 Owner 开启 SAC。

## 参照

查过 gogo-party 的 `packages/room/src/accounts.ts`、`runtime-context.ts`、NaveHQ 的 execution-session 绑定与 authority-effect 分离，以及 LoomOS 目录及源码检索；未找到可直接替代此 Windows LPAC 三树 retirement 的实现。还查了仓库 `reuse-blueprint`、`upstream-reference-map` 与 substrate 源码拆解。历史实现区分当前持有者与历史会话，不能代替本产品的资源完成事实。采用现有 H 原回执、Claude holder recovery/ACL retirement、Job 与 RootLock 生命周期，以及已批准的 holder-disappearance 组合；不新增外围探测框架。与一般 session resume 不同，本次仅判断首次同 pin 自管来源迁移资格，旧 continuation 与需要原 StopFact 的操作继续拒绝。

## 节奏

PR #54 已按时补发进度，本时段新增可运行能力为无；构建、签名、安装与部分阶段不计新能力。公共 reader 的合法重复 init 修正已有受影响语法与卫生云检；全量检查未运行，不计通过。没有等待外部条件，根因接线与独立复核并行推进。

## 受影响云检与普通换装进展

修复源的五项受影响原生测试均为实际通过、无失败或忽略；生产库与主程序编译、Browser、卫生通过。实验冻结产物与受信签名已逐文件回读，使用原签字节；全量原生库、双构建比对和同字节装机烟测未运行，不能借本次局部结果记为通过。

普通视图新基线确认十三项正式保护、原闭库字节、物理身份和全部业务记录连续。旧候选通过原正常卸载入口退出零、原 finalizer 回读 DELETED；卸载后十三项保护与完整候选连续性仍相等。原空安装根保留，没有额外清理。新候选按冻结原字节正常安装，安装回执、真实字节、登记与十三项保护和完整候选连续性均通过。

并行复核发现下一次执行脚本错误地把旧秘书长 H 代次作为新代次依据：原 E 席位空闲代次与旧 H 已释放代次不同。按照现有宿主启动代码，以同一闭库 designation/E 身份、无未释放 H、E 代次加一和生产 resident 身份公式绑定新会话；线程标识取第一次非历史的真实产品读回，停止与释放后继续核同一 E/H 归属和真 StopFact。原脚本字节保留，修订聚焦复核及语法检查通过。这是执行前的测量前提修正，不是产品启动失败，没有重登。

## 实际安装恢复与原记录读回

新候选中的单次新 USER 请求通过原产品入口恢复同一 PROBED 阶段，原回复为 resume/READY、migrated=1；后读仍为 READY，原实例 install-state 为 APPLIED/installed=true，版本与登录观察一致。没有重放原失败请求，没有重新下载原包。秘书长能力、真实停止、释放及逐次读回完成，原 shell/native 正常关闭退出零；十三项正式保护相等。

同一物理闭库后置只读确认前后数据库字节相等。原子代 episode/custody 仍 UNKNOWN、没有 StopFact、没有成功代次映射，父 claim 的原修订号和真实停止证明保持不变；独立失败子代资源 journal/receipt 已 APPLIED，原 capture 与回执原文、父子原请求及 revision 关系完全对应。三个原根 FileID 不变，原目标 SID 直接读回为零，当前 DACL/control 与原精确 after-image 相同。此处独立测量只有三个根，不能声称另一次全后代测量；产品的原持久 retirement 记录与此分开保留。

自管 SOURCE 为 MANAGED，与 READY 的原 stage、原实例 HOME、固定版本、CLI 原字节及原包一致。上述 USER 没有模型或登录调用；实际新能力仅为首次自管安装恢复，不是自管模型会话或完整 M2/M3 验收。原始失败、UNKNOWN/PREPARED 及父停止记录保全。闭库仪器中局部函数遮蔽和缺 receipt revision 关联在执行前经独立复核修正，原字节保留，未改产品迎合仪器。

下一步继续绑定自管来源的新 E/F 与模型用例，真实两轮及同 H 冷恢复尚未运行。用例与只读准备已并行完成，Root 继续集成，无需 Owner 操作。心跳已在 PR #54 记录实际新增安装恢复能力；没有以构建、审计或准备结果重置停滞计数。

## 后续实际自管模型与同 H 冷恢复

已装修复源 `78ba520f` 上，真实 USER 新建 Claude 模板、直属席位与私有 SINGLE F 并逐项读回。实例页为 LOGGED_IN；原闭库确认 MANAGED 来源与 READY 副本、固定版本及 HOME 一致，新域没有 H。秘书长真实停止、读回，正常关闭退出零，十三项正式保护相等。

后置仪器曾错误要求凭据元数据全表相等。本次真实秘书长合法增加历史、代次、别名和已撤销 profile，各行与同一 H/custody/StopFact 及原源身份对应；旧行完整保全。源 FileID 未变，硬链接数的增量与唯一新别名对应。依原事实修正测量并独立复核，不改产品或旧记录。随后两次只读执行发现 `str`/`Path` 调用形状与 Win32 扩展盘符路径比较错误；保留原脚本、输入、日志和任务结果，仅修测量后读回同一物理 F 成功，未重发 USER 或模型。

沿用原 `m2-provider-win11.mjs` 的一次性 Claude 上下文用例，无旧 registry fallback。实际模型两轮分别回答 778、432；真实停止并正常关闭后，同一 H 冷恢复到新代，第三问未泄露前文答案，仍回答 778。三个原 USER 对应原 CLI result、厂商会话、物理源游标与规范化答案；两代真实 StopFact 不同，最终停止与释放完成。两次 shell/native 正常退出均为零，十三项正式保护相等。

原端到端退出零，闭库直接协议读回确认两代均 STOPPED，观察器读前读后数据库字节相等。协议黄金样本已生成，仍为 REVIEW_REQUIRED/NOT_ASSESSED；五条未知原帧保留，不计为成功事件。补读原 custody 对应两次实际 PID/创建时间、操作和原 StopFact，映像均来自同一自管 stage 的固定 Claude 原文件，不借全局 CLI 的成功。未读取凭据内容、复制数据库或补写停止事实。所有本次一次性任务已导出并逐项清理，原失败证据保留在 D。

这确认自管 Claude 两轮对话及同 H 冷恢复上下文，不证明同轮插话、运行中打断、其余 CLI 或完整 M2/M3。实验期间 SAC 为关闭，稳定点前仍需开启并补完整链实测。下一步继续当前候选的 OpenCode 无模型生产 H/别名生命周期、V08 与其他已授权项，无需 Owner 操作；不发布、不自行验收。

参照：采用既有原候选 Claude 同 H 上下文用例、现有闭库协议 reader、黄金样本导入器和宿主真实停止/正常关闭路径。不同之处仅为实际 MANAGED 来源、新 E/F 与当前输入绑定；不使用旧全局来源回退。OpenCode 并行准备采用已经修正的 init-only selected-profile 入口，不采用会发模型的同名脚本，不重复已通过且字节未变的云检。
