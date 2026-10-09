# 三家宿主续跑的启动失败

结果：已装原 0.1.47 在读取三家当前宿主状态之前失败，原记录为 FAIL、操作数为零。原 WebView 启动警告直接报告 `GOGOKE_PRODUCT_SERVICE_FAILED:1`，内层为 `HOST_EOF: native-host closed stdout before handshake`。不能把此结果归为三家协议失败，也不能由历史登录和版本配置推导当前可运行。

收尾：同一窗口用原 caption 正常关闭，持有原子进程句柄的父进程记录退出码 0。正式版五组、实例记忆与账本保护前后比对通过；五项一次性任务保留原导出和结果后逐项删除并回读不存在。原失败记录不改写为通过，无模型会话，无强杀。

定位：nativeHostClient 的启动路径给 stderr 建了 pipe，却未读取；握手前关闭只返回 HOST_EOF，真实 native 原因被丢掉。已在独立施工分支补有界尾文、spawn 错误及实际关闭信息，聚焦独立复核中；尚未云构建或装到产品，不声称 EOF 根因已修。

下一步：复核后纳入含真实产品改动的新候选，仅跑受影响构建、安装和启动复现。取得原生错误后按根因修，再用现成 provider runner、原测试实例及真实 H capability 续跑。跨项目 V08 独立准备线已开工；G 实例页持续读取问题仍交 Claude，其代码不由主控修改。完整 M2/M3 与稳定点验收仍未完成。

参照：原 MainApp readiness 警告、product_entry 的服务尾文、design37_host 的关闭事实及现成 ActualProduct 父进程退出记录；沿用原 Interactive/Limited 任务、caption 正常关闭和保护观察者。仅修直接错误丢失，不延长等待或修改 CLI、隔离、权限和授权范围。
