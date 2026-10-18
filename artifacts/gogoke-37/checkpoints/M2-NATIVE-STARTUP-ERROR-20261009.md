# 三家宿主续跑的启动失败

结果：已装原 0.1.47 在读取三家当前宿主状态之前失败，原记录为 FAIL、操作数为零。原 WebView 启动警告直接报告 `GOGOKE_PRODUCT_SERVICE_FAILED:1`，内层为 `HOST_EOF: native-host closed stdout before handshake`。不能把此结果归为三家协议失败，也不能由历史登录和版本配置推导当前可运行。

收尾：同一窗口用原 caption 正常关闭，持有原子进程句柄的父进程记录退出码 0。正式版五组、实例记忆与账本保护前后比对通过；五项一次性任务保留原导出和结果后逐项删除并回读不存在。原失败记录不改写为通过，无模型会话，无强杀。

定位：nativeHostClient 的启动路径给 stderr 建了 pipe，却未读取；握手前关闭只返回 HOST_EOF，真实 native 原因被丢掉。工人补有界尾文、spawn / process 错误及已观察退出信息；复核发现第一版检测失败后无界等待 close，修订改成立即返回，未确认的退出与排空明确记为 UNCONFIRMED，不编造退出码。修订聚焦独立复核无阻塞，已集成进 0.1.49，受影响云构建在跑；尚未装到产品，不声称 EOF 根因已修。

续接：0.1.49 受影响轻构建、本机清单验签和冻结字节核对通过。普通视图安装通过，旧候选正常卸载且数据保全；正式版、原生记录和安装字节回读通过。旧候选仅余一个空程序根，独立审计确切对象后非递归删除并回读不存在，没有重复批量清理。

首次只读启动已核到实际资源代和界面，但私有仪器漏建 `launches` 数组，在记录启动时抛出 `TypeError: Cannot read properties of undefined (reading 'push')`。原 FAIL 保留、零操作和零模型调用；原窗口正常关闭，原父进程退出 0，正式版、记忆和账本前后保护通过。修正仪器的新证据目录正在重跑，不把启动一次通过当作旧 HOST_EOF 根因已经修复。

下一步：当前普通只读实例、USER 席位与 F 绑定核对后，再用现成 provider runner 和真实 H capability 续跑。跨项目 V08 工具已集成，真实轴仍 NOT_RUN；V06 席位准备与旧证据保全迁移清单并行。Claude 已交三个面板慢读取修订，全新独立席位聚焦复核中，其代码不由主控修改。完整 M2/M3 与稳定点验收仍未完成。

实际续跑：三家已登录实例、USER 席位和 F 绑定的 12 次原读操作相符；正常关闭退出 0，正式版、记忆和账本保护通过。Claude 真实 H open 随后失败，原文为 `Claude initialize read: PROCESS_PROTOCOL_PIPE_FAILED: persistent frame deadline; PROCESS_STDERR_TAIL: gogoke Claude CreateNamedPipeA failed win32=5 prefix=uv`。未发送模型问题，Grok 在该混合尝试未执行。原 H stop 取得 StopFact、admission-release 为 APPLIED，原父进程确认 caption 关闭退出 0；闭库原操作字节、RELEASED claim、IDLE 席位与正式版/记忆/账本保护通过。原 FAIL 不改写，Grok 拆为新的单家证据续跑；管道根因独立只读分析并行，未改 CLI 或权限。

并行结果：G 慢读取复核发现读失败的旧表单仍可用 Enter 调用真实 configureProfile/create，已在 PR #54 交 Claude 修订，暂不集成。四个旧 39/40 候选 ZIP 经确切对象独立审计后复制至 D 盘并验哈希，仅移除对应 C 盘源文件并逐项回读；历史读回 JSON 与展开内容保留，转移清单保全追溯。未动其他任务工作树。

后续：Claude 的 Enter 补修已通过全新聚焦复核并集成；三个面板慢读取和旧表单操作拒绝的已安装版实测留下一候选，未把浏览器 CI 当真机通过。Grok 单家原 open 拒绝 `Grok private HOME: first ordered root baseline has unknown package SID`，发生在进程工厂前；原未创建 claim 实际释放、USER 回到 IDLE、caption 关闭父进程退出 0，闭库原 RELEASED/无 StopFact 与保护回读通过。辅助收尾脚本首次沿用了错误旧文件名，零操作即 ENOENT；原件保留，新文件核对路径与编码后完成，不重放未知请求。

Grok 根因材料：HOME 的物理身份、孤儿 SID 派生与旧已释放的未创建 H 会话精确相符；该会话没有 F grant/effect。旧代码先 grant root、后验证 auth 子项失败的历史记录与残留吻合，不能据此把当前 ACL 接纳为新 baseline。独立边界意见确认，仅撤确定的失效 ALLOW 不新增权限，属于已授权实现细节；须独立真实 cleanup intent、逐项物理/ACL 前后证据和崩溃续接，保留 unknown-SID 拒绝。已派独占工人实现该限定撤权；另一工人按微软 LOCAL 现成命名方式修 Claude 内部 A 管道两端，云端受影响检查与全新聚焦复核中。主控在旧 49 上继续 V06，不等待这两条产品修复。

参照：原 MainApp readiness 警告、product_entry 的服务尾文、design37_host 的关闭事实及现成 ActualProduct 父进程退出记录；沿用原 Interactive/Limited 任务、caption 正常关闭和保护观察者。仅修直接错误丢失，不延长等待或修改 CLI、隔离、权限和授权范围。
