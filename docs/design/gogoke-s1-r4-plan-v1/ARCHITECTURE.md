# A：单一主服务、持久事实与实际执行

## A1 数据所有权

一个当前用户profile的T3-derived服务拥有产品数据根和SQLite。所有Project/Seat/Grant/Context/Decision/Outcome/Dream记录通过服务内repository进入同一事务域。产品动作复用现有orchestration/event/receipt链；不增加并行派工者。原生thread/工具/原生history仍由原生运行端管理，只保存opaque ID、来源和绑定，不直接改写其文件。

薄Rust native-host负责创建的OS handles、Job、pipe和根锁观察。durable custody、授权和释放决定仍在服务DB。UI只投影和发可信请求；旧gogoke_daemon仅legacy-only，不允许与public服务争用同根/同profile。正常退出UI不终止持续服务或native任务。

## A2 启动、存储与迁移

固定donor源码在third_party/t3code/独立构建；精确字节导入与后续补丁分别提交。沿用其event/projection/receipt与SQL机制，新增gogoke_前缀表和独立迁移账本；不修改已发布上游migration。Node构建使用隔离的24.13.1及pnpm11.10.0，原desktop Rust1.89工具链不联动升级。prepare/postinstall/下载器先审后执行，禁止默认npx latest、全局安装和自动原生安装。

minimal构造先核allowlist，关闭HTTP/WS管理口、remote/mobile/Tailscale、voice、自动更新/安装/telemetry、自动探针、标题生成和账号读取。默认driver disabled，fixture准入后才构造fake实例。不能先启动后台任务再把按钮藏掉。允许当前用户私有IPC和受控测试子进程，不允许live产品daemon；两者不是同一概念。

显式根ID+OS文件身份归一化；拒绝不支持的网络根，锁文件不替换、不继承给worker，根锁持有到服务完成收尾。SQLite外键、WAL、synchronous=FULL在每个连接验证；这是配置合同，不声称硬件断电实测通过。坏库/未知新schema/迁移失败保原件拒绝，不能变空库。attachment先写受控内容地址库并完成所需持久化，再事务引用；孤儿内容隔离，不自动公开。真实迁移留WP12。

## A3 命令与投递

JSON对重复键/无穷/NaN/非法版本/整数溢出拒绝；序号用无损整数字符串跨TS/Rust。durable action记录domain、operationId、payloadHash、target/binding、generation、policyRevision。幂等键(domain,operationId)命中后仍核当前访问权；同键不同内容/目标/世代冲突，不因正文相同误去重。

同事务写intent+event+receipt，reactor独占claim，先耐久写DISPATCHING/attemptId再外发。崩溃或失联于外发前后且无可靠native回执时保ACCEPTANCE_UNKNOWN；通过可信native幂等或无副作用查询才可化解，否则隔离旧绑定后显式新operation，不盲重发。DB commit报错也可能未知，先读回，不重复外部动作。

continuation绑定nativeRequest/principal/domain/action/binding generation/expiry。回答和取消有同一串行化承诺点；承诺前可撤销，承诺后进入待核/停止。原生callback丢失不能假装还可答，不能以新prompt替原批准。EOF、exit0、Promise完成不是任务接受证明。

## A4 进程两阶段与失败保管

所有direct CLI、ACP、SDK内部可执行文件、helper、PTY均列入spawn清单。统一机制：持久launch intent→native-host.prepare(ticket)→受管句柄/Job身份→主服务持久custody→native-host.activate(ticket)→initialize→ready。票据绑定binary摘要/profile/domain/generation，一次消费，不能经模型参数授予。

Windows受管创建用CREATE_SUSPENDED及受控句柄继承列表，保留实际初始线程句柄，AssignProcessToJobObject成功并保存custody后ResumeThread；失败不得继续裸跑。专用launcher可为SDK提供被监管的可执行入口和透明字节转发，不能假设SDK必有custom-spawn API。SDK要求无法满足则对应通路BLOCKED，不偷偷绕行。不要给所有短命probe强加挂起。

Job禁止breakaway，检查父Job/嵌套限制；KILL_ON_JOB_CLOSE只是机制，不是退出证据。服务异常后helper尽力关闭自己拥有的Job，DB保留未决launch；恢复拒绝第二writer直到核对完成。helper无独立产品store/策略；失联的清理结果不能从租约到期推断成功。

停止分interrupt/closeBinding/shutdownHost。默认工程预算grace10s、termination5s、observe5s，host总30s容纳身份/收据；不是按8s强退父进程。StopProof包含精确身份、直接子进程退出、后代/Job活动证据、writer fence、耐久收据与errors。身份未知、缺句柄、save失败、二次stop、原始错误被清理错误覆盖等均保residual custody。Unix group不保证防setsid逃离，不宣称等同Windows。

## A5 IPC、优先级与恢复

Windows named pipe限制当前SID、拒绝remote、首实例保护与server身份核对；Unix0700目录UDS+peer uid。Owner控制/worker工具/只读诊断分别principal，secret不放命令行或模型env。frame最多4MiB；control队列64、session普通队列128；输出1024事件或8MiB先到为准。溢出明确gap/backpressure，不吞终态/批准。

每域有序event+snapshot cursor；广播只加速，可从同版本快照恢复，不能拿另域序号暴露活动。sourceEpoch与binding generation严格匹配，旧帧不追认给新执行。首连singleflight；32并发只一次connect/auth/subscription，失败方关闭；清理用epoch compare-and-clear。

权限在入队与副作用前重查，停止/撤权不等待Jev。原生工具还需实际OS/工具隔离，不能用pipe鉴权、目录名或同用户ACL假称对恶意同用户进程强隔离。保护模式未准入即拒绝该模式，不降成full-access。
