# 三家宿主续跑的启动失败

结果：已装原 0.1.47 在读取三家当前宿主状态之前失败，原记录为 FAIL、操作数为零。原 WebView 启动警告直接报告 `GOGOKE_PRODUCT_SERVICE_FAILED:1`，内层为 `HOST_EOF: native-host closed stdout before handshake`。不能把此结果归为三家协议失败，也不能由历史登录和版本配置推导当前可运行。

收尾：同一窗口用原 caption 正常关闭，持有原子进程句柄的父进程记录退出码 0。正式版五组、实例记忆与账本保护前后比对通过；五项一次性任务保留原导出和结果后逐项删除并回读不存在。原失败记录不改写为通过，无模型会话，无强杀。

定位：nativeHostClient 的启动路径给 stderr 建了 pipe，却未读取；握手前关闭只返回 HOST_EOF，真实 native 原因被丢掉。工人补有界尾文、spawn / process 错误及已观察退出信息；复核发现第一版检测失败后无界等待 close，修订改成立即返回，未确认的退出与排空明确记为 UNCONFIRMED，不编造退出码。修订聚焦独立复核无阻塞，已集成进 0.1.49，受影响云构建在跑；尚未装到产品，不声称 EOF 根因已修。

续接：0.1.49 受影响轻构建、本机清单验签和冻结字节核对通过。普通视图安装通过，旧候选正常卸载且数据保全；正式版、原生记录和安装字节回读通过。旧候选仅余一个空程序根，独立审计确切对象后非递归删除并回读不存在，没有重复批量清理。

首次只读启动已核到实际资源代和界面，但私有仪器漏建 `launches` 数组，在记录启动时抛出 `TypeError: Cannot read properties of undefined (reading 'push')`。原 FAIL 保留、零操作和零模型调用；原窗口正常关闭，原父进程退出 0，正式版、记忆和账本前后保护通过。修正仪器的新证据目录正在重跑，不把启动一次通过当作旧 HOST_EOF 根因已经修复。

下一步：当前普通只读实例、USER 席位与 F 绑定核对后，再用现成 provider runner 和真实 H capability 续跑。跨项目 V08 工具已集成，真实轴仍 NOT_RUN；V06 席位准备与旧证据保全迁移清单并行。Claude 已交三个面板慢读取修订，全新独立席位聚焦复核中，其代码不由主控修改。完整 M2/M3 与稳定点验收仍未完成。

参照：原 MainApp readiness 警告、product_entry 的服务尾文、design37_host 的关闭事实及现成 ActualProduct 父进程退出记录；沿用原 Interactive/Limited 任务、caption 正常关闭和保护观察者。仅修直接错误丢失，不延长等待或修改 CLI、隔离、权限和授权范围。
