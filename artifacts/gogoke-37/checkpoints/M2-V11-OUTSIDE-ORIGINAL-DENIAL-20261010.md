# V11：原目录外写入请求与只读启动失败

## 改了什么、结果

沿已安装 0.1.56、原 Codex 测试实例和已登记的原 USER/F，只发一次新的目录外写入请求。原 CMD 工具结果为 `Access is denied.`、退出码 1，目标不存在。闭库读回绑定原 H/A/F、请求、工具命令与目标；真实 H stop 取得 StopFact，释放 APPLIED，产品正常关闭、退出码 0。正式安装、数据、登记、快捷方式、记忆和账本的 13 个原保护字段均相等。一次性任务按确切 custody 删除并回读不存在。

这只证明该次原工具调用被拒绝。原 reader 的 `permissionCause=UNATTRIBUTED` 保留，未处理的原通知仍为 PENDING，不计成功。完整 V11、无网络、MIXED 与合并尚未通过。

测量末尾首次比较缺私有输入 seal，原失败与原执行字节全部保留。核对原 config 哈希后，只补齐由既有 preparation seal 派生的输入元数据，重跑原 13 字段比较；没有再次提问、重放工具或改写 H 结果。

此前 MAIN_WRITE 原工具是 CMD 引号语法错误，不能算权限拒绝。普通视图用相同 argv 直接证实引号形式失败、无引号形式成功；工具生成与原文 reader 已同步修正并通过受影响 Browser/卫生检查。此前 READ_ONLY 的真实 USER、SINGLE F、准入和提交成功，固定 CLI 在 initialize 前返回原文 `GetComputerNameExW did not provide buffer size`；实际 stop/release 和正常关闭已完成。该启动失败不能算 READ_ONLY 边界通过。

## 下一步、参照

独立开发环境用原固定 CLI、真实 LPAC 启动器对照：READ_ONLY 仍复现原 panic，NETWORKED_WRITE initialize 成功。开发结果仅用于定位，不代替候选验收；两种配置还有身份/目录权限差异，不能把单一 capability 定为已证根因。按计划无法保证的档位直接拒绝，不增加 capability、不改 CLI；只对已验证的精确 pin/READ_ONLY 组合补 H 准入拒绝和原始原因，旧请求与停止事实不改。

参照既有 V11 原 H/USER/F 入口、immutable reader、普通视图任务 custody 和三组保护观察器；复用原实现，只跑尚未发出的一个请求。独立席位已查自身历史与仓库研究，没有固定 CLI 下保持该只读边界的现成环境修法。并行修 Windows lib-test 缺 v6 manifest 的工作流条件，以及准备当前候选的 OpenCode 原 initialize/stderr 读回。当前没有新增登录或重启触点；SAC 关闭期间的结果不外推强制模式，稳定点前请 Owner 开启并精确读回。
