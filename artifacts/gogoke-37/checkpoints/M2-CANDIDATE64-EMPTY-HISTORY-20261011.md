# M2 / M3：候选64原失败与并行修复

## 改动与结果

- PR #88 已按 main 精确读回。受信签名后的冻结候选64安装成功；实际包内版本仍为 0.1.62，不冒充资源升级、公开发布或稳定完整链通过。
- 原实例列表首次成功读回六个原实例，未新增登录。秘书长随后在空线程冷恢复收到原 typed `-32600 / no rollout found for thread id`；新失败仍 UNKNOWN，旧真实 StopFact 与全部原记录保留，没有模型输入。
- 原 start 返回 paginated、非 ephemeral、空 turns 和预计算路径。首次普通 GetFileAttributesW 返回 Windows 3，但路径超过传统长度限制，该读数单独不能证明缺文件，旧推断已撤回。随后在普通视图对同一原路径使用 verbatim API，仍返回 Windows 3，sessions 祖先也返回 Windows 2；原实例、history 的物理身份与原件相符，四类输入记录均为零。返回路径不等于文件已经落盘；本次补测只认这个原对象。
- 秘书长正常标题栏关闭后，原父进程记录退出零；关闭仪器在保存 shell/native EXIT 原事件前触发断言，二者退出观测继续标未确认，不能补造。关闭前后原件和错误保留。独立十三项正式保护均相等。
- Claude 原多轮用例随后在实例列表被 `holder recovery capture would expand` 拒绝，尚无新 H 或模型请求。原 shell/native 句柄均退出零，原事件逐条保存，十三项保护相等。闭库机械核对确认：选中的旧非 APPLIED 捕获仅含已由精确后继释放完成的旧 g4，当前新 ACTIVE g6 不在其成员内。元数据辅助结论不能替代生产完整的物理和来源核对。

## 已并行开工与下一步

- Codex 官方持久化线：一次真实固定 CLI 开发试验的 `thread/read(includeTurns=true)` 收到原 typed `-32601 / list_turns is not supported yet`，原 H open 保留 UNKNOWN，无模型输入。协议检查通过不抵消该失败；这份产品改动不集成。该独立 D 试验的同一原路径用 verbatim API 确认文件存在，普通路径仍报 Windows 3，再次证伪普通错误码单独推断。继续查官方持久化、索引与当前每代 sqlite_home 的直接关系，不复制历史、不补造 ACK。
- 恢复组线：完整校验已有精确后继释放后，才把完成的 Revoked 旧失败捕获排除出待恢复组选择；不改其历史 phase、members、UNKNOWN、StopFact 或字节。新持有者形成自己的原捕获，未满足完成条件仍拒绝。全新独立静态复核无阻塞；五项受影响云检已启动，正式候选恢复尚未重测。
- 集成席位为以上两个包增加固定的受影响云检选择，不开放任意 filter，不改正式门槛；全库、完整候选链和验收保持 NOT_RUN。
- OpenCode 单次 GIT_DIR 非仓库环境对照仍在三十秒时限报 persistent frame deadline，原 stderr 只有创建实例与 fromDirectory，未发送 session/new 或模型请求。真实 Stop/Job 清零、映射撤销、本轮 SID ACE 撤回与原 ACL 完整相等均有原件；profile 删除并读回不存在。该失败不能定位具体调用，不改固定 CLI、系统根 ACL、特权或网络能力。

## 测量与保全

一次保护读回解析误把 provider 的相对 `file` 字段当成 Secretary 的 `path`，失败原脚本、日志和已经完成的正式快照保留；修正后按原路径与哈希复用该真实快照，仅补未执行的记忆及账本读回，没有再扫描正式安装。一次性任务按真实结果分别导出、删除并回读不存在。自动审批拒绝的额外原响应抽取未绕过；已取得的模式与原记录不据此扩大结论。

## 参照与理由

参照既有 empty-release、completed-successor、原 PID/创建时间消失核对与原 RPC journal；检查过本地历史实现、仓库调研和固定 Codex 官方源码。官方 thread/read 的 persist_thread 之后还要读取 SQLite 索引和 turns，直接试验表明不能只采用前半段源码推导整条请求成功。当前每代 sqlite_home 与持久 history 的关系继续按原启动参数核对；此前旧生命周期处理只在下一次冷启动结算，当前自动可用性仍不合格。

这些修复在已授权的宿主、适配器与云检写域内，由 Controller 决定：同一用户、现有隔离权限不变，无新登录、提权或公开发布。当前没有需要 Owner 操作的外部条件。稳定实测前仍须通知 Owner 开启 SAC 并实际读回；未完成 M2/M3，不自行验收。
