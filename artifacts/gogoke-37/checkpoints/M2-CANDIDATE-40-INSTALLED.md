# M2：0.1.40 实装与自管 CLI 原始拒绝

## 改动与结果

0.1.40 冻结源为 `0571c47b966102f0858ab2edeaa83ca31a2e6158`。它保留受验的产品及 G 集成，并将自管 CLI 的 native `ERR` 原文和失败阶段带回界面，在原拒绝后停止后续下载、探测与迁移。

PR #54 第 1 条的无 Now 数据占位已于 `e53351a7` 删除；第 2–7 条共享挂载修订为 `9e4094b3`，两者均已进入 0.1.39 和本候选。真实 0.1.40 启动自报版本、源、资源集合与冻结输入一致；本次初始 Home 未显示“正在进行的数据还没接上”。未把初始 Home 的观察写成每个真实对话已实测。

| 证据 | 精确对象与结果 |
|---|---|
| 完整 desktop | `37801979909`，五项 SUCCESS |
| 卫生 | `37801986459`，SUCCESS；公开源扫描 0 泄漏 |
| 冻结产物 | `11563228494`，ZIP SHA256 `16b2526d845def0df56e50f65a10810097bd63082ce04de79fe564b13c47ca6a` |
| 受信 main 候选资源签名 | `37808818660`；产物 `11563827770`，ZIP SHA256 `d148185ee8a507353c864b8948943f089ab98a179547a53fb05ec23e3934744a` |
| 同字节云端装卸 | `37809255910`，SUCCESS；原正例 `11565533324`、独立负例 `11565488449` 均 PASS/complete，绑定同一冻结源、产物、安装包及签名运行 |
| 当前原生完整库 | `37802307694`，四分片 `12+32+6+738=788`，无遗漏、重叠、失败、忽略或跳过；实际 LPAC shim 2 项通过 |
| 限定原生资格 | 独立资格 SHA256 `bcaf91d19d66e5556b0d974d7f4e50ba7f2d0c1011aa7f08b982ba3b4b302a58`；formal 复用原 `d2267b28` / `37778197099` 的 security 22、focused 43、process 3、server 658；输入树未变。原 whole run 超时取消保留，当前 library-only 的 formal 项为 NOT_RUN |
| 冻结安装包 | SHA256 `9be88540ac5b6b8de33c631545d7c400851618b9211379674786488ed410df8f` |
| 实装壳 | SHA256 `7e223e7eb5d703e56bdbbb58d721e6f84187a16dc22d3efeb30f1b8143f1cbb8` |
| 实装 native / Node | SHA256 分别为 `4a7f0bdd3e28d8e27e77125dcc415b603edfef61126ccd4cb608fdd716d36ae6` / `e3be0545990c90995d7bf3a7af5d64af1f2e0fc1bbd9b79c27f7abc1e9676e50`，均与 0.1.39 相同 |

普通视图的安装路径、非提升用户和 package status 15700 已核对。独立复核过渡脚本后，真实 0.1.39 原卸载退出 0、产生唯一新 `DELETED` 回执。随后独立逐项审核并清理 1117 个空目录，0 文件、0 重解析，每项回读不存在；inventory SHA256 `a9ec7c60f57bae7ab3bc847c9ceeb2ca099437d986032648c827969efaa773e2`，审核 SHA256 `f466a2cfb2c539c3598fad028e320fbcf670af2425352267703b2386fb811575`。0.1.40 原包安装退出 0，八成员、候选登记与新安装身份符合原冻结输入。正式五组、原 native root/HOME 和保留 custody 前后相同；安装期间无 3077/3033。

## 实际未通过项与原文

真实产品、原桥接、原请求的 Codex 自管安装在 BEGIN 阶段拒绝：

```text
GOGOKE_MANAGED_CLI_NATIVE_BEGIN:ERR\tV37StoreFailure("managed CLI global login/observer custody: Busy")
```

不再是丢掉 native 原因后的 JSON 解码错误。此例未启动模型，四家自管副本原状态均 NOT_INSTALLED、无 STAGED 探测副本。保全后从原界面关闭，正式五组及保留 native 事实再次相符；没有重放安装请求、伪造 StopFact、清记录或读取凭据内容。闭库直接查询原生产 SQL，发现 H claim、owner binding、process episode、instance HOME 四组存在匹配；正在核对确切持有者与原独立消失回执。

测量记录：运行中外部 SQLite 只读打开失败，未据此判断产品或绕过文件保护；关闭产品后按已验证做法在普通视图读取 checkpointed、immutable 元数据。原失败输出留在私有阶段。

## 下一步与并行线

按确切 Busy 记录及既有 holder-disappearance 原结论修正准入一致性；修复期间只跑受影响云检。Claude/Grok 的真实后续模型流程仍待自管副本就绪，不记通过、不新增登录触点。

并行 M3：`347caf9a` 的完整 desktop 与原生 library-only 已通过；新的原事件读取分支 `20775459` 正在云测，原 E0382 失败和新源码分别保留。共享 physical-stop 原 intent 接线已独立全轴静态核对，移除了对已知不支持的 Native 删除操作的前置停止；这些新字节尚未集成进 40，真实 USER 通知及 M3 为 NOT_RUN。

## 参照与边界

沿用真实 39 候选的冻结、验签、普通视图过渡、确切空目录审核和原 CDP/tester-army 流程；`E2E_TELEMETRY_DISABLED=1`，未使用 agent.act。Busy 依据原 `v37_managed_cli.rs`、`instance/managed_cli.rs` 和 `v37_holder_disappearance.rs` 的 SQL 与既有独立持有者消失结论，未用进程缺席代替 StopFact。M3 原事件依据 A 已保存的原 H 帧及既有 authority pump；历史、gogo-party/NaveHQ/LoomOS 和仓库调研的查阅结论保存在并行交接中。

当前 main、授权回执 blob `470e27951143b83d5230bfa3602fa6e21148a918` 与范围摘要 `7abd79da58def8623ca9afa6b1ee6984ef49d5056fef64a9b67ef6aeeb3ef8ff` 已精确读回。无 main/PR #54 合并、公开发布、代码签名、系统安全设置修改、自行验收或 Owner 新操作。
