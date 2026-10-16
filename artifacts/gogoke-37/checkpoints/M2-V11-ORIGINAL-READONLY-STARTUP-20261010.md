# V11 原启动与测量错误

改了什么：保留原 MAIN_WRITE 与 READ_ONLY 两个请求，不改固定 Codex。主树测试命令去掉 CMD 重定向内层引号，只允许无空白/扩展符路径；闭库 reader 同步核对精确原 argv。F 祖先改为同物理对象核对，原请求/目标/权限约束不变。

结果：原 MAIN_WRITE 工具报 `The filename, directory name, or volume label syntax is incorrect.`，不是权限拒绝。普通视图同 argv 对照证实带引号时退出 1、不创建文件，无内层引号时退出 0且只写私有空目录的新文件。原错误与原工具字节保留，未补写通过。

READ_ONLY 的真实 H open 报 `GetComputerNameExW did not provide buffer size`，固定 CLI 在 gethostname 断言退出；管道 109 是后续症状。已用实际 USER stop 与 release 取得真实回执，正常关闭退出零，13项正式/记忆/账本保护字段相等。没有伪造 StopFact、强杀或新登录。

下一步：仅续跑尚未发过的外部空目录边界，原两项不重放。READ_ONLY 原失败明确未通过；继续查不改变其网络/文件/LPAC边界的做法，不能直接增加 internetClient。Windows lib-test manifest 修法在独立云端线执行，秘书长接线 Browser/卫生已通过。

参照：现有原 H/A/F reader、系统 CMD 的实际 argv 对照、固定 gethostname 源与微软 API 文档。两档均已有 registryRead/lpacIdentityServices，是否缺网络能力导致 hostname 失败仍是推测，不据此扩权。PLAN 的无法保证档位须拒绝/不可用规则仍适用，完整 V11/M2 未通过，SAC关闭期间不称强制模式通过。
