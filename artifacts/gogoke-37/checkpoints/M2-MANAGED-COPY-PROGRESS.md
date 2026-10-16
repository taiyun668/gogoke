# M2：自管副本准入修正进度

## 已完成

PR #54 第 1 条与其余共享挂载修订已包含在实际安装的 0.1.40；精确冻结与实装证据沿用 `M2-CANDIDATE-40-INSTALLED.md`，没有重包旧版本。

按已决定保留旧 M1、使用新实例的边界，真实 0.1.40 宿主对旧 `codexTestM1` 首次写入显示名与 `enabled=false`，回执 APPLIED，读回 profile revision 1。没有改其旧 H claim、custody、HOME、凭据或实例注册记录。候选正常退出 0；正式五组及全部保留 native 身份、元数据前后相同，无模型调用、无凭据内容读取。

闭库只读原账本确认：M2Ready 的 claim 已释放，两条历史 ACTIVE episode 各有原 APPLIED holder-disappearance 回执及 REVOKED profile；Fresh 无旧 H 或 credential 记录，profile 缺值仍须资格核对，不能被跳过。旧 HOME ACTIVE 本身不是一个存活进程的证明。

共享消费者 `a1a114c6` 已接入宿主 `programSourceError` 到实例页现有 ERROR/raw 字段，并保留登录失败的原原因。签名 Node 类型检查、Browser `37823895165`、卫生 `37823900669` 成功；该新增字段尚未在已装 40 实测。

## 当前检查与下一步

自管副本分支 `7cec2712` 正跑精确来源 library-only `37824666430`。首次准备不改旧绑定或 ACL；同版本同字节迁移先核原 pin、HOME、注册/repin，再允许显式停用实例保留无绑定，其他实例均须资格化并原子绑定。真实版本变化和卸载保留原严格拒绝条件。新增迁移资格包含未决 H stdin；精确无绑定 H 请求返回 CONFLICT；回滚失败同时保留 primary 与 rollback 原因。

已完成首次独立全轴审查和聚焦复核，发现集中修正。先前原云检 E0277、E0624、E0283 的编译失败及取消运行均保留，不记测试通过。聚焦发现的登录观察前置缺失已修夹具，没有改生产逻辑迁就测试。最新测试补 UNKNOWN stdin 和第二个 source INSERT 中止后的整事务回滚；等待真实云结果后再集成、走稳定候选链和原实例实测。

测量修正：最初 profile 请求前遗漏 user-host 初始化，原错误 `USER_HOST_NOT_STARTED` 留存，确认未发写后修测量；元数据投影中两个 revision 列同名，已用显式列名重读，原注册 revision 与 profile revision 分别保留。以上均不是产品或正式数据变动。

## 已并行开工

M3 shadow 原事件生产者 `20775459` 的精确 library 四分片 797 项、LPAC shim 2 项通过。共享消费者最新为 `ac8607a5`，精确 desktop `37824862842` 与卫生 `37824869438` 正在跑：原通知保留 envelope/sourceRef；同代保留成功读取位置；旧 reader 完成不能覆盖新代；原 resume/recover 后接续观测；实际续读成功后才清除旧错误；物理 H stop 不受观察器故障阻断。daemon 的共享 connect 保持原入口，新增安全断言并入既有五个 CI 测试入口。先前 shadow desktop 取消/失败不能外推为最新通过。

这条线尚未集成进已装 40；首次 attachment 与完整 UI 历史交接、真实 USER 对话、M3 和候选接受为 NOT_RUN。不改 Claude 的 G 目录。PR #54 已主动查看并更新进度评论，无新增 Owner 登录、重启或合并步骤。

## 参照

沿用现有 `instance/managed_cli.rs`、`program_source.rs`、原持有者消失回执、原生 instance profile 和普通视图闭库元数据读取。使用既有 holder fixture 验迁移，完整首次下载/探测生命周期留给真实安装链，不建新的测量框架。

事件接线复核了 gogo-party `packages/room/src/server.ts` 的 `handleSeatEvent`、仓库 `docs/research/reuse-blueprint.md` 的 Codex 事件映射依据，以及已有 Tauri `app_server.rs` 原事件出口。采用原线程/轮关联、原错误与既有事件形状；不同处是本产品由 native A 保存原帧、H 拥有物理进程，所以只读消费者使用原来源游标，并独立保留 H StopFact，不在界面层另起 CLI。
