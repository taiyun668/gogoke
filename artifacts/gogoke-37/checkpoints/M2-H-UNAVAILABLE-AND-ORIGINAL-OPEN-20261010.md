# M2：H 已知不可用组合、当前 OpenCode 原启动与更新库测试

## 改了什么、结果

H 对实际已验证的固定 Codex 0.160.0 程序 pin 与 READ_ONLY 的新准入返回 DENIED，并保留原 gethostname panic 和未取得 Windows 原错误码的界限。没有增加 capability 或修改 CLI。首次独立风险复核发现权限字段提前求值会影响旧回放；已改为匹配新请求及精确 pin 后才读取权限。USER、Host、Lead 三入口的原 pin/上限检查保留。原请求、其他 pin 和档位保持原行为；预先创建的 home/F binding 不在“无新 H claim/进程/StopFact”的保证内，Lead 外层仍沿原 StoreFailure 传递原因。

修正源的受影响云检查六个过滤器、11 个测试通过；原日志包含实际旧无权限字段记录的回放、精确 pin 矩阵及真实 SQLite 事务回滚。独立首次风险审查及已知缺陷复核无剩余阻塞。测试不等于三个完整产品入口实测，当前已装候选尚未包含此改动。

沿原已装 0.1.56，语法修正后的一个新 MAIN_WRITE 原工具返回 `Access is denied.`、退出码 1、目标未出现；实际 H stop/release、正常关闭 0、13 项正式保护字段相等，任务逐项删除并回读不存在。READ_ONLY 与目录外请求没有重放；原 reader 的 cause-unattributed、PENDING 及未运行项保留。完整 V11 不记通过。

当前 OpenCode 固定程序、原 USER/NW/F 单次启动确实复现 `EPERM: operation not permitted, lstat` 系统盘根对象，原 H 返回 ProtocolEvidence 的 stderr_tail 与管道错误 109。没有发送模型问题。真实 stop/release、正常关闭 0、13 项前后保护字段相等，原执行字节和错误保留，任务已删除并回读。

普通宿主对该根对象的只读访问检查：READ_CONTROL 成功，WRITE_DAC 返回 Windows 错误 5，ACL 前后相同。H 使用按会话代次派生的 SID，给旧 SID 的静态许可不会成为下一次会话的许可。权限模型：当前用户宿主不能修改该根 DACL，模型仍在 LPAC；满足 Owner 限定的自行授予/撤销需要当前未授权的权限机制。未改 ACL、未提权，OpenCode 这一条线保留阻塞并交 Owner 裁决，其他已授权线继续。

Windows 更新库测试已在修正源实际执行 10 个原测试通过；直接提取同一个 Cargo 测试 EXE 的 RT_MANIFEST #1，确认 Common Controls v6 六项属性。冲突模式负控明确失败且未启动 Cargo，取消条件修正通过独立复核。只有测试步骤追加 manifest flags，正式程序编译/签名逻辑未改，诊断结果不当作完整门槛。

## 下一步、参照

参照原 H verified pin、同事务准入和旧 journal；复用原 USER/F、真实停止及普通视图保护观察器；OpenCode 先核自身历史、仓库调研及固定 OpenCode/Bun 源序，未找到当前 H 下免改 ACL 的配置做法。采用 Microsoft/Cargo/Tauri 现有测试目标 manifest 做法，修的是实际加载器，非 CLI 或替身。

更新库修法集成到施工分支；为可信 main 准备单独 CI PR，避免实验候选工作流字节不匹配。Root 新增的 USER 桥 24 项边界检查移到原 npm test 入口，原云检查保留，main 无该产品实现时不会引用施工分支脚本。继续受影响 Browser/卫生及真实 M2；阶段内不出中间试用版。稳定完整链前请 Owner 开启 SAC 并读回，M2/M3 与验收尚未完成。
