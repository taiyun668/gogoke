# P00 Pass D R1 — 施工前技术审查与 Owner 决策移交

## TASK_RESULT / CURRENT_STATE

**Pass D 技术判断：ACCEPTED_FOR_CODE_ENTRY（仅施工前证据就绪）。**
**有效执行门仍关闭：P00 NOT_ACCEPTED；code-entry gate CLOSED_PENDING_GPT_P00_REVIEW；Owner 决定 PENDING；生产授权工作包[]。**

此处不是两个互相抵消的结论：Pass D 判断资料是否足够进入有约束的施工，Owner 决定是否实际放行。技术判断完成不替代 Owner，不授予写生产、合并、发布或真实外部操作权限。legacy gate 字符串继续保持；pending_reason 为 Owner 授权，而不是要求无休止重做 Pass B。

## 1. 固定对象与审查角色

源码 bc665a852833952b76d9508401193bedd2198436；R3 证据 c055e125531e166cb67a45531a3c7a8aa9d70330；被独立复核的 R3 登记 7427b1eaba920e30a188623cea05807df9114ee5。
独立 R3 复核包 SHA-256 b258335fb7281df8c80179bb3aacd3bcf8e53962c0ef82b469fd5d188c963d15。原独立冻结包 SHA-256 685936270e4b4ae45354140c0aebbc05079c63e46abe197944e5b34c58d88507 保留。
本窗口是 Controller 技术综合判断与原件接收，不冒充新的盲审、复核者或生产最终验收。独立关闭来自 received/ 原报告、逐项结果和回执，全部原字节保存。旧 chronology 不可证明的 PASS 继续 NO_VALID_VERDICT；最新有效 B 对原 A 的 CHANGES_REQUIRED 不改写，后续 C 关闭其具体证据缺口。

## 2. G1–G5 裁定

| 条件 | 判断与依据 |
|---|---|
| G1 固定身份和差异 | 复核包外层哈希、140项包内文件、21项直接交付摘要通过；原始源码2806个blob通过。按实际字节重建源码、原snapshot、R2/R3 payload与登记共六棵tree，全部匹配。R3相对源码56个路径均为根交接或docs/design；没有生产/现有测试混入。 |
| G2 有效证据 | 有效R3账本、原观察、能力义务、符号与回写补充保持。数目不是充分性依据；采用已完成的独立逐项/source-join/相邻语义复核。 |
| G3 独立复核 | received/确认C2-F01 CLOSED_AS_EVIDENCE、B03 EVIDENCE_GAP_CLOSED，保全B01/B02/K16/K17。原集合不重做，旧无效PASS不恢复；本轮哈希不冒称平台原始时间线认证。 |
| G4 缺陷闭环 | 指定的证据错误均关闭；没有把固定源码中尚存的坏行为改成已修复。身份、内存、持久化和耐久确认分开；释放busy不等于退出或writer释放。所有实现/运行义务带责任及首验。 |
| G5 D与Owner | D的技术部分满足；Owner尚未接受P00或授权生产包。实际门继续关闭，合并另需授权。 |

## 3. 实际重放与验证限制

本次Controller重放7条入口命令均exit0：作者符号检查两次（基线0错误/14负例）、源码假依赖夹具两次（31项），独立审查者AST脚本（124 checks）、四项负变体组（内部exit2如期拒绝）、preservation脚本（250 checks/40发布对象）。符号、31假件、AST和独立负例结果与原发布或复核结果逐字节相同。
preservation结果除common_files_compared为80而原为27外，其余字段相同：本次投影包含额外未变的继承证据，预期四个变化路径与全部250断言不变。未降低预期、未改审查者脚本。
这些是可重复的原件/源码连接检查，不是第三次独立审计。31个PASS包含危险旧行为被准确观察；没有真实FS/PID/provider，也没有生产mutation。原复核4次非零尝试原封保留；本次不把它们报成产品错误。Stage1、census、旧49场景、产品套件及CI未作为本次新执行或全绿依据。
详情在 INTAKE_VERIFICATION.json；本次命令stdout/stderr与工具脚本在完整D交付包。完整独立复核ZIP留在Library并随D包保留，不声称已将整个ZIP上传GitHub。

## 4. 实现义务与第一批建议

RUNTIME_OBLIGATIONS.json是既有义务的交接索引，不重写测试计划。原计划第9节为逐检查截止权威；某axis的first_wp只表示开始，不能把后续T37/T43等挪到错误包。N1/I/M义务不删除。
建议Owner仅首批授权WP10a与WP01：前者可逆封存外部remote/voice入口，保留本地daemon、模型资产和恢复条件；后者落地公共对象/事件/投递/许可/所有权结果合同及追踪校验。不能只删除Codex，也不能把Room或另一个kernel/store整体导入。
WP02须等P01并另定界；WP03须等P02和P10a并另定界。B02/B03的真实进程、锁、落盘及旧writer隔离必须在相应机制复用前兑现，不允许靠本次证据关闭跳过。

## 5. 权限、保全与下一步

此次只新增D证据与更新独立D分支上的五份控制状态；R3有效payload、R3/R2原分支、PR#6的原head、生产源码和既有测试均保持。新D记录只登记已发生的独立关闭与技术判断，不重新生成A或改写原报告。
若Owner确认P00并明确首批范围，再固定授权记录与任务卡后施工；在此之前不建生产施工分支、不改产品。允许讨论和登记证据不等于允许provider登录/付费、真实Home/数据迁移、daemon/Tailscale/listener、麦克风模型、用户Git/PTY、安装更新registry签名发布。PR#6合并、UI大改、批量删Codex与其余工作包均未授权。
新源码/候选改变后按受影响轴复核，不沿用旧SHA作产品PASS。最终生产集成、高风险全轴、异构审查和Owner产品验收仍按原计划执行；P00技术就绪不取代它们。
