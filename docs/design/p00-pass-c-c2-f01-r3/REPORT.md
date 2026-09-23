# P00 Pass C R3 — C2-F01 窄范围修订

状态：AUTHOR_REPAIR_READY_PENDING_C2_F01_REVIEW。P00 NOT_ACCEPTED；code-entry gate CLOSED_PENDING_GPT_P00_REVIEW；runtime_verified=false；生产工作包[]。

## 1. 固定输入与审查角色

源码：bc665a852833952b76d9508401193bedd2198436；tree 746024abeb2e6d35ac88fd1b2e315fceb0e9f54f。
父登记：3ed20ef3b84e8b26ac2985337da6c87aae3e7ed2；父payload：5d15a11ecd33763df31172dbdd3426f9f740ab0c。原PR#6仍以55a8443ef98f1cc231f4f45d78c3bc3cad93a9e4为历史审计head，不移动/合并。
收到的R2独立定向复核ZIP SHA256：03204138c9b8a931a07038f7157bc32d8a006b9415882c3a1844adc6b1588a46。原发现、逐项结果、报告及receipt原字节放received/；完整ZIP留在Library和交付包来源记录，不冒称已整体上传GitHub。
这是作者侧C3修订，不是新盲审或独立关闭。收到的B01/B02/K16/K17 CLOSED_AS_EVIDENCE保留；B03只剩C2-F01，新对象关闭仍待审查者确认。原冻结集合未读取改写或重新认证。

## 2. 有效修订

reuse-contracts.selection_matrix 的 confirmOwner 更正为 noteProcess；startSeat 更正为 ensureSeat/ensureSeatInner。原R2合同其余字段保留，新增两条显式索引，连接本目录的symbol-bindings与ownership-contract。
14条选择分别列 operation_label 与真实 source_symbols；34个具体声明token绑定源码文件/行号/blob/声明SHA256，9个描述性标签明确为行为而非同名函数，另1条Rust目录范围不冒充TS声明。15条实际调用边绑定调用表达式与声明；外部Node API与动态Seat调用明确区分，不声称完成全程序调用图。

## 3. 实际语义与责任

- noteProcess 缺记录、缺busy、seat不匹配、缺PID或processStartedAt为空时返回void；查询异常也可变成null，不能据此认定进程已退出或owner已确认。
- managed/host身份先改内存再save；save使用固定.tmp写后rename，无本方法级fsync/锁/回滚。保存失败时内存与磁盘可不同；rename失败可留临时文件。
- ensureSeatInner在claim返回后才设置claim.instanceId；claim自身先改busy后保存，保存失败可能尚不满足外层回滚条件。
- seat.start之后，noteProcess保存或后续home探测失败会走ensureSeat.catch/releaseAllFor；该方法清匹配的非orphan busy并保存，不验证子进程退出。释放保存异常发生在内存清除之后，且可替代原异常。不能凭“释放了busy”重新授权第二writer。

ownership-contract CJ-O01–04把这些分支接回F24/F25/RT32/W30/Q14/Q15及WP01 typed合同、WP02持久化、WP03唯一ExecutionHost/残留custody/旧writer隔离。Room总服务、InstanceStore和另一Rust kernel仍不整体采用；本次不改生产。

## 4. 执行与证据层次

verify_symbols.mjs：正确基线0错误；14负例全部被拒绝，包括恢复两个旧名称、伪造行为标签、不存在类/符号、错路径/行/blob、删除绑定或实际调用边。保留40/37计数仍能检错。
targeted_fixtures.mjs：31个精确源方法体假依赖场景，31通过，0未来合同模型；PASS可表示原错误行为如预期被观察。输出重复一致。没有原生CLI、真实PID、OS强杀、真实Home/文件写入、网络或产品启动。
源对象身份重新核验2806个tracked文件，Git fsck 0，无remote；R2 candidate tree按实际字节重建匹配c3e602ffa43049306ea4943b80372a6399e849a4。这是身份检查，不是重做全仓审计。
R2原verify_repair.py在新对象清单上重放，输出另存R3；Stage1/census/原49场景未作为本轮新执行，旧结果保留。当前逐符号重放入口见本目录REPLAY.md。原R2重放说明中的25对象/49场景仅属R2；新对象数量以当前index为准。具体命令/退出码和失败记录见execution.json与本轮交付logs。

## 5. 保全、冻结与下一步

surfaces、六份303 traceability、84能力行为、K16/K17纠正、R2源身份和原夹具/结果保持原字节。reuse-contracts的imports/writers/lifecycle/root/permission/remaining_runtime字段逐项相同；原权威表与R2重放说明逐字节保留；本项新增AUTHORITY_ADDENDUM.md与REPLAY.md，通过当前索引和报告明确承接，不重写已关闭章节。细目见preservation.json。
发布采用payload及后继控制登记两提交，返回的完整SHA在控制文件/回执，不伪造自引用。本轮独立结论仍PENDING，不把原CHANGES_REQUIRED改成PASS。
下一步仅对C2-F01实际符号、身份回写/释放语义、缺符号负例与新对象摘要定向复核；已关闭B01/B02/K16/K17不重开。之后才由Pass D和Owner决定代码入口。

真实CLI/账号/付费、PID/后代/旧writer、锁/alias/junction/崩溃迁移、daemon/remote/RPC、UI/权限/音频/麦克风、Git/PTY、构建/no-Codex/安装更新/签名发布均未执行。
