# S1-R3 Qualification Gates

## G0 — Source / rights / build qualification

必须实际完成：
- donor commit/tree/hash 固定；
- LICENSE 与直接/打包依赖 license 清单；
- 锁文件存在且构建不使用浮动 latest；
- 审查 install/update/postinstall/build scripts，禁止未授权全局安装或 provider 下载；
- 独立工具目录构建；
- T3 server/provider/orchestration 最小闭包 typecheck/test/build；
- fake provider executable/SDK spawn 注入可行；
- 旧 19 task、68/157/33/84 映射生成并校验。

缺 Windows 不阻断纯源码 G0，但 Windows 专属项必须保持 NOT_RUN。

## G1 — Data / authority

- 单 data root / single writer fence
- product migration / downgrade refusal / corruption preserve
- action+event+receipt 同事务
- duplicate op id 正反例
- policy failure deny
- donor 默认远程/update/install/network 控制面在构造前封存

## G2 — Runtime / custody

- 五 driver 均能以 fake executable 或 fake SDK 路径进入受管 seam
- driver + instance 多配置不串状态
- profile revision invalidation
- Windows Job subtree、host crash、fast exit、registration failure、PID reuse
- stop proof / residual custody
- capability supported/unsupported/unknown

没有真实 Windows 环境时 G2 Windows 部分 = BLOCKED_PLATFORM，不得以 Linux/mock 宣称 PASS。

## G3 — Delivery / continuation / privacy

- durable intent before side effect
- crash at pre-send / post-send-pre-ack / post-ack
- ACCEPTANCE_UNKNOWN 无盲重发
- approval/question duplicate/wrong-domain/wrong-generation/expiry/restart
- event gap/snapshot/cursor recovery
- Owner-private vs public review vs project vs secretary isolation
- revoke → deny future material / rebuild when required

## G4 — Product vertical / review

- Tauri client → private IPC → service → fake native → persisted event/result
- duplicate operation executes once
- late/gap events
- bound approval/question
- stop proof and residual custody
- old Wave A useful assets disposition report
- focused tests + full relevant test suites
- fresh Sol review
- Astra latest-candidate high-risk full-axis review
- no merge

## Later native admission

真实 Codex/Claude/Grok/OpenCode/Antigravity 的 login、paid calls、native image/tool/subagent/fork/resume 并不由本次授权自动允许。按原后续 native gate 单独取证；当前 S1-R3 先证明机制与安全边界。
