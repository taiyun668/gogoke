# V06 共享接线

Owner 已决定完整推进 M2/M3，不采用中间试用版。集成工人显式 cap 解析与 V06 驱动；共享宿主新增计划已列出的 USER set-orchestration-bounds 操作，单独回执，保持原 tune 事务、完整请求指纹、CAS 和层级检查。页面报告范围自身 cap；设置范围不再改写项目并行上限。创建与准入在原事务核对未回收直接子席位数，缺 cap 拒绝，不编默认。

聚焦源复核发现新操作由当前 generation 构造的指纹不能回放；已改为从 E 的原操作结果恢复原写入前 generation，仍由原指纹与当前目标一致性判定。新增实际安全控制涵盖同原请求回放、改变原请求拒绝、后续配置后旧回执拒绝、项目 cap 保全及第二子席位拒绝。四字段模板可存储和复制为未完成配置，无法授权子席位；只有显式 USER 范围操作提供新的五字段授权值。

受影响原生云检新增枚举选择 seat-scope / ledger-scope；每个原测试过滤器至少完成一项，否则失败。其余原生门槛标 NOT_RUN，默认完整运行条件不变。签名 Node 类型检查及 YAML/作业条件检查通过；首次 Node 语法命令工作目录错误，纠正路径后通过。原生行为、真机及全量仍 NOT_RUN。

首次受影响云编译失败原文：`E0599: BTreeMap<JsonString, Json> ... trait bounds were not satisfied`、两处 `E0425: cannot find function config in this scope`。Json 不实现 Clone，已沿用 canonical parse 复制经过验证的 payload 值；新安全控制补上现成局部配置调用。保留原失败，重跑同一受影响选择，不扩大到全量。

第二次云编译通过，范围安全控制实际 2 过 / 1 失败；原文 `NonCanonicalJson("JSON integer exceeds JS safe range")`。夹具错把 canonical parser 先拒绝的大整数交给 scope parser 的 unwrap；改为分别断言两个真实入口的拒绝，不改生产限制。受影响选择继续收集已成功编译后的互不依赖测试，集中报告失败；编译失败则保留后续依赖项未跑，不追加完整门槛。

下一步：读回受影响云端原件，集成空目录卸载修复、V10/V11 入口，再集中真机。当前冻结候选公开密钥验签与产品成员一致已通过；尚无新的安装或产品能力结论。

参照：main PLAN 的 K-SEAT 五字段范围操作、E.2、V06；原 seat::tune 的请求指纹和 authorize_replay；工人 orchestration 与原 E/H/F 驱动。沿用已验证存储原件与事务，不用项目 cap 替代席位 cap。
