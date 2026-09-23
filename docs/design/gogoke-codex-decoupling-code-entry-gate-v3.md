# gogoke P00 / S1 — Owner 明确授权后的代码入口门

```text
P00: ACCEPTED_FOR_CODE_ENTRY
P00_SCOPE: PRECONSTRUCTION_EVIDENCE_ONLY
CODE_ENTRY_GATE: OPEN_FOR_AUTHORIZED_S1_ONLY
OWNER_CODE_ENTRY_DECISION: AUTHORIZED_FULL_S1
AUTHORIZED_PRODUCTION_WORK_PACKAGES: [WP10a, WP01, WP02, WP03, WP09a, WP04]
RUNTIME_VERIFIED: false
PRODUCT_ACCEPTANCE: false
MERGE_AUTHORIZED: false
```

## 授权来源和固定对象

Owner 原文“ok，确认授权，直接推进”，是在明确提出接受 P00 并一次授权完整 S1 的上一轮提议之后。逐项许可/禁止见 `docs/design/gogoke-s1-authorization-r1/OWNER_AUTHORIZATION.json`。这不是将历史“继续写方案”推定为开工；本条登记的是新的明确批准。

固定计划 `34b7f891e4e17715b08491570740516d6cf7f49f`；生产输入 `bc665a852833952b76d9508401193bedd2198436`；R3证据 c055e125531e166cb67a45531a3c7a8aa9d70330；D证据 9ec7ca56566b667271013d75d867f2008aeeceda。已关闭B01/B02/B03/C2-F01/K16/K17和D技术判断保留，不改旧报告或旧冻结。

## 开门只及 S1 范围

G1 固定身份和范围继续要求：实际dirty/生产漂移核验，任务允许路径以固定TASKS.json的write_scopes及Controller共享列表为准；read_source不是写授权。
G2 原303观察、84行为、133方法与具体新合同继续保全；不得把错误行为的证据关闭当成已修复。
G3 施工前独立B/C已完成；新实现仍需fresh审查和高风险全轴，不由Owner批准替代实际测试。
G4 六个P依赖、33个S1截止及全157检查义务保持；阶段内先满足前置再集成，失败/skip/缺平台不写PASS。
G5 D技术认可加本次Owner授权允许有边界施工；不代表允许合并、真实业务接入或产品验收。

允许Codex在一个S1集成线编写范围内生产实现、同域测试、运行受控临时目录/假进程/测试宿主/私有本地IPC/构建测试，Controller提交推送。临时OS测试与真实用户daemon/数据/凭据严格分开；所有权限仍受实际sandbox和角色限制。

不允许：PR merge/auto-merge，重写原分支历史或丢弃用户改动，真实账号/付费/秘密/用户迁移，外部listener和live daemon/Tailscale，麦克风与模型操作，用户工作区Git/PTY副作用，产品安装更新/registry/签名发布/TestFlight，S2/S3或未列依赖升级。worker依然不得commit/push；任何新研究或改变contract/root/writer/权限/迁移/验收仍回GPT。

## 执行状态不与许可混淆

本提交只登记授权与启动指令，初始Codex接单状态尚未确认；没有生产变更/测试通过/运行中任务的凭空声明。实际Codex必须先写接包和真实任务/branch/HEAD，再施工。本门不得因提交授权文件被自动改成S1已完成。

当前规则见 `docs/design/gogoke-s1-authorization-r1/CODEX_START.md`。原固定计划PENDING字段是历史，不通过改计划或降低校验消除；只有后继授权改变许可。S1完成后的研究/方案回GPT，范围外施工另需授权。
