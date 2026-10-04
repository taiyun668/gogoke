# M2 原测试实例的旧隔离状态恢复：待 Owner 裁决

状态：PROPOSAL / NOT_AUTHORIZED。产品施工停在需要新恢复证据契约的边界；不修改当前计划、凭据、ACL 或旧 custody。

## 已取得的事实

- 已安装候选 0.1.23，冻结产品源 `563a0ef0135413cddd425c6d7af6909034e8118e`。签名、同字节云端装机冒烟、实际 Win11 安装和正式版五项保护回读通过。
- 修正后的云端运行 `37210416627` 三个 job 全部通过：库测试 662、聚焦测试 33、同源路径 shim 2，均无失败或跳过；四项编译后安全变异步骤全部通过。失败后的普通视图正式版保护快照再次确认五项未变。
- 原 `codexTestM1`、固定 CLI 0.160.0、物理实例 home 和已登录状态保留。未改 CLI，未读取、复制或哈希凭据内容。
- 真实 M2 首次 `K-SESSION.open` 原文为 `legacy account scope requires original stopped observer custody`。尚无模型 send/turn；失败请求和 COMMITTED 预留已保全，产品经原窗口关闭，退出码 0。
- 旧 global/instance/generation 2 的 owner-login custody 为 UNKNOWN，停止证明为空。历史源码和创建时点证明该登录使用旧 LPAC 身份，不能按名称、旧版本或 PID 不在排除。
- 普通视图只读 ACL 证据：HOME 有 20 条旧 AppContainer 可继承授权；auth.json 和 installation_id 均为未保护的继承 DACL。此记录仅为安全描述符元数据。

威胁模型：旧隔离进程所持身份可能沿用实例目录的历史授权；涉及同一 OS 用户内的实例/项目隔离，不涉及提权。当前模型启动在授予新权限前被拒绝。没有执行攻击复现，也没有证据证明凭据泄露。

## 已排除的修法

不清空 UNKNOWN，不伪造 STOPPED，不凭进程缺席推造原 Job/writer 停止证明，不跳过迁移或 quiescence guard，不把本次产品退出当成旧操作停止。不修改固定 CLI、不复制或换绑原 home。

只读复用已有 protected baseline 的方案在当前对象上不成立：基线本身尚未保护。仅增加 observer SID 不能消除旧继承授权。上述结论已由独立只读席位按实际 ACL 与源码复核。

需要裁决的不是常规产品 bug 的修复权限：当前授权计划的共享停止契约明确复用既有 stop proof，不重做；若用另一个恢复事实允许旧权限迁移，会改变此处允许写入的证据前提。现有 UNKNOWN 保留规则不能被进程缺席替代。此次只交这一个边界决定；普通测量修正和已启动的云端检查已自行完成。

## 推荐裁决：补充独立的系统重启栅栏事实

批准设计并审计一个独立的恢复事实契约：产品在重启前，绑定当前系统启动身份、原物理数据库、root、home 和旧 custody 的确切身份，持久保存恢复栅栏；Owner 重启 Windows 后，产品从受信 OS 来源读回不同的启动身份，才允许针对这些确切旧身份进行权限迁移。

这个事实不转换成 NativeStopProof，旧 UNKNOWN 及缺失的停止证明保持原样。当前启动期的 UNKNOWN、未纳入原栅栏的对象、来源或身份变化均继续拒绝；固定 CLI、登录 home、模型 LPAC 和 capability 集合保持不变。不能把普通界面重启、时间差、PID 缺席或调用者提供的 boot 值当成系统重启事实。

Owner 同意后先补实现、云端边界检查和独立审计，准备可回读的候选，再一次通知 Owner 重启 Windows。随后回读栅栏和 ACL，继续原 `codexTestM1` 的真实 M2；此时不新增一次 Codex 登录。

替代裁决：明确授权一个全新的测试实例并由 Owner 登录，保留原实例及其旧状态，旧数据兼容另行处理。此替代改变当前指定的测试绑定，也不证明旧实例迁移通过，因此须 Owner 明确决定。

## 参照与依据

沿用当前 `AGENTS.md` 的角色/契约边界、先查逻辑硬伤、直接证据和阶段内轻规则；对照现有 `process_custody.rs` 原 ticket/nonce/Job/writer 停止契约，`v37_login.rs` 旧作用域迁移及 source quiescence，`credential_binding.rs` 进程内 ACL 完成状态，`credential_launch.rs` 冷 holder 拒绝逻辑，`private_history.rs` 的目录继承行为。沿用本轮原始普通视图 ACL 回执、历史登录源码和捕获 custody；完整本机路径、身份及私有回执留在私有证据区。

本文件不是施工授权、产品验收或新恢复事实的执行回执。

## 走法反思

本轮云端使用的新根通过，并不能推出保留旧 custody 的 Win11 根也能迁移。实际首次 open 原文、旧启动源码和当前 ACL 元数据已经足以定位；继续讨论假设或增加外围探针没有收益。下一步只实现获选的恢复边界，云端并行验证后走一次真实候选更新，不重复重建诊断框架。
