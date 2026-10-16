# M2 产品就绪错误原文恢复

## 改动与结果

已装 0.1.46 的热态中断和冷恢复覆盖片段通过。随后 V06 原生 USER create 已返回 APPLIED，但测试断言错误地要求回执具有 domainId；实际协议没有该字段。已修测量，域仍由原始帧和域限定读回确定，原父席保留，未重发 create。

续接启动在实际 WebView 控制台取到 `Failed to signal gogoke update readiness` / `GOGOKE_PRODUCT_SERVICE_TIMEOUT`。没有进入 H 或 F 操作。原窗口已点真实关闭按钮，原 driver 捕获进程退出 0；原失败、正常关闭事实和任务 XML 留在私有证据，已逐项读回清理完成任务。未新增登录、未读取凭据。

源码确认服务 owner 已读到 stdout/stderr，timeout 分支却丢弃它们。0.1.47 仅使已确认退出后的失败沿现成 4 KiB 尾文返回，带上原 I/O 与适用的 Windows 原始错误；成功响应字节、20 秒上限、终止/退出确认和 lease 所有权不变。原 0.1.46 被丢掉的尾文无法补造，具体卡在哪一步仍未知。不把超时等同于 SAC、磁盘错误或授权失败。

## 下一步

受影响云端构建后，用新原冻结字节复现就绪检查，直接读尾文。保留原 V06 父席和所有请求身份，继续实际 User/H/C；V10 同域绑定与 V11 固定 Git 原调用工具已准备，尚未运行的轴仍 NOT_RUN。完整重门与完整更新链留稳定点，不公开发布、不接受 M2/M3、不合并 PR #54。

## 参照

采用 product_entry.rs 原非零退出的 with_failure_output 和 design37_host.rs 原 handshake 超时附 stderr 的做法，不新增外围探针或补偿等待。独立只读席位核实实际 Node readiness 是经原 native pipe 认证和 Controller admission 后返回；等待的是进程退出，尾文可区分已经返回 readiness 与尚未返回。本次只恢复已捕获的直接事实。
