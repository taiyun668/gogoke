# 0.1.46 中间复现候选

- 产品改动：热态历史投影改为核原事件来源覆盖，关闭/卸载仍使旧读取失效；版本三处更新。不为换哈希空打包。
- 结果：热态共享 hook 独立复核通过，受影响浏览器云检通过；V06 原来源工具接线已集成。原生产品源与 0.1.45 相同，不冒充新的全量原生结果。
- 候选链：PR #81 默认正式路径五项、实验路径三项及卫生通过，Controller 按既有细节授权合入。main 回执和 scope digest 精确读回相同；施工采用与受信 main 完全相同的 desktop workflow。中间模式不做独立重编比对、原生信任测试或 smoke，全部记 NOT_RUN；正式默认门槛未修改。施工旧 workflow 独有的 Design37 安全/登录控制保留在 Git 原件，稳定点须单独补齐，不以本次轻构建替代。
- 下一步：新源原冻结构建、受信资源签名/验签、普通视图安装、只复现原热态问题；V06/V10/V11 继续独立接线。不是中间试用发布，不合并 PR #54，不接受 M2/M3。
- V11：原固定 CLI 调用没有抄入提示的 JSON 转义；CMD 的参数编码碰到带空格的绝对路径，返回 not recognized/exit 1。该命令原 started/completed 已解析，其他尾帧 PENDING 独立保留；既不能认作 Git 可达，也不能认作隔离拒绝。不改固定 CLI/ACL、不增加 Owner 登录。
- 参照：已通过 PR #81 的 main 工作流和范围回执、原 Win11 H/A 工具帧与同一 fixed Codex rollout、Codex rust-v0.160.0 unified_exec/pipe/item_builders、Rust Windows CommandExt 与 Microsoft CMD 原规则。
