# V11 准备入口的原生回执核对

- 改了什么：准备入口按真实 K 回执字段核对，去掉不存在的顶层 domainId 回显要求；绑定后的 revision 读外层回执。闭库仍用原请求、回执哈希和原 E/F/ledger 记录核域，不改产品或权限。
- 结果：固定已装源、原 Win11 帧与生产编码器一致；全新 Sol 聚焦只读复核未发现另一处必然失败的字段断言。签名 Node 语法检查通过。准备入口和 V11 真机结果尚未执行，静态复核不是验收。
- 下一步：当前 V10 完成真实 stop/release、正常关闭与保护读回后，沿原 Codex 实例和同项目既有 USER/SINGLE F，显式准备另一份 READ_ONLY USER/SINGLE F；再运行原 V11 文件用例。不新增登录、不重签或换候选。
- 参照：原 `m2-stop-worktree-prepare.mjs`、当前已装候选的原 K-SEAT/K-WORKTREE 帧，以及固定源的 `store/session_transport/wire.rs`、`product_database/v37_seat.rs`、`product_database/v37_session.rs`。复用原准备流程，只修错误的仪器前提，没有新增产品接口。
