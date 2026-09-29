# R2-05：PR #34 合并前的冻结候选与普通 Win11 回执

状态：EVIDENCE_READY_FOR_OWNER_PREMERGE_REVIEW；不是 Controller 验收、Goal Acceptance 或 PR 合并。

## 当前验收对象

旧 985e17d1 候选的实机记录和 PR #34 测试结果只保留历史来源。当前实际运行对象是复合冻结候选：full 0.1.11 源码 c1fc362773f64576c23d17128d9cd86046b5c6b3，加上 resources 0.1.13 源码 acb909d31c02acd1608a6f723d026e1d80ed0681。两份发布清单均使用 Owner 私钥、依 2026-09-25 授权在本机签署并验签；资产直接以冻结字节发布，未重编、重包或重签。

| 对象 | SHA-256 |
|---|---|
| full 0.1.11 setup | f3a5b894abfc9e4351aa41ac79666288e0c6d836b7cd6aa99e1b6e1294966817 |
| 已安装 shell | 3384deda28240834358aee6b6b0bdf14a15fe197de78498209a80674989492b8 |
| 已安装 native host | 9a28408837efd5c05e0c21c5645a3a82d9a7d2a2839f3e2a56ae756e114ad559 |
| 已安装 Node | e3be0545990c90995d7bf3a7af5d64af1f2e0fc1bbd9b79c27f7abc1e9676e50 |
| 活动资源 index / set | 1388d6d5111c0b816b9a94f6361467d2a046885704331686df13798d318b80c0 |
| 活动资源 pack | 3c5035780a6a01152cd260d9032b7b43860dbe5bd9a85a5f8b87077464c8c926 |

精确来源、双构建、签名、云端安装烟测、发布哈希及先前普通 Win11 四种更新场景见 MC-196 至 MC-211。acb909d3 之后的施工分支提交没有改动上述已发布产品字节；有检查点和测试断言修正。独立只读审计对这个复合身份复核了 V01–V08，未发现新增的产品阻塞项；该审计不替代外部异构审查或 Owner 决定。

## 本轮普通 Win11 实测与 G3

同一普通交互式 session 1、未打包进程状态 15700、SAC 强制状态 1 的基线回执 SHA-256 为 14a5e1e15a168ce7463825054659848443e76da6a1f3a3b749bb572f2db894b2。实机当前 HKCU 卸载登记为 OWNER_RELEASE、DisplayVersion 0.1.13，安装位置、实例 ID 和卸载命令均存在。旧 0.1.2 受保护安装树 SHA-256 1eeed52ee69720002af6482c36871d6574c6f31995a2f6841dd9f3dc83e8d4c1；当前 full 树 SHA-256 948448cdd3faa18d812e29170e723659cce70c23ec6852b1d88093bffaf3b417，前后均未变。

G3 的旧命题要区分对象：旧 0.1.2 在普通视图缺卸载登记仍是历史事实；当前运行的注册实例已有有效登记，故“当前产品缺登记”不成立。打包 Codex 视图中的旧 0.1.2 HKCU 键不能代替普通视图登记。旧树保持受保护遗留物，不为 R2-05 补登记或迁移。

当前 0.1.13 真正安装的产品入口在普通视图完成受控任务，native Controller 已准入、native host 可达、结果为 VALIDATED_TEST_RESULT_NOT_ADOPTED；回执 SHA-256 5da83d56d15302142a86942a2da15794fcf96a320f210b46086f302f3d5a8a8b。开放 fixture driver mock_novel_a5e2804b9c6d3180 也完成同一真实入口测试，回执 SHA-256 16bebf8b8790aafd6f2a6238deaeb6e6508233e4de32e9624c11447d38aa4872。它在限定测试账本分支只新增一条 draft：commit 42f077d9651631e81bce42a05bb8af6c3428eb22，单文件 Git blob f83a8028004eff2d26580da49feee43ca2b506d0，文件 SHA-256 54d9b6c7a7b8466c1e20f73e13171d4ecb1963e954ac71e50575ea094238f0ed，executionEvidenceSha 为 acb909d3，testOnly=true，acceptance=TEST_FIXTURE_NOT_ADOPTED。相同请求重放后 Git 分支头仍为 42f077d9，没有第二条写入。此 draft 不并入 main，也不是 PR #34 的旧结果。

探针先后把 readiness 文件名前缀、开放 driver 的 draft 必需字段、以及 novel decision receipt 格式写错；各失败原始回执保留。修正只涉及本机私有测量脚本，未改产品字节。第一次失败后旧安装和当前 full 哈希未变，数据仅有正常窗口几何差异。最终数据库只读回执 SHA-256 2515a283d6498ebc6a0a7ea79d315eb8557a2e4e4b5adb22887b10ec87c0ff11：SQLite integrity_check=ok，原有 6 个 package ID 全部存在，现为 8 个；grant revision 从 8 条增至 9 条。没有旧业务记录丢失或改写的证据。

## 独立审计后的遗留物清理

独立只读审计先核对了对象逻辑与本体：三份文件均为生成的 module-policy 临时路径清单，确切文件名、大小和哈希与普通视图回执一致；没有 Gogoke 进程、重解析点、多硬链接或占用。Owner 已授权按 G4 方法直接清理。Controller 重新核对后只用三个完整 LiteralPath 逐项删除，每项立即回读不存在，剩余两项哈希保持不变，最终匹配数为零。删除前哈希依次为 4228fc6cb34b871dc9b4f1e35180201666f06347179db2f8426437dda0bcd02e、8343b877d8ab9d52bf6ea089f69974ee1a97fe94e0143bec0bff30ea1ffcf6c1、8343b877d8ab9d52bf6ea089f69974ee1a97fe94e0143bec0bff30ea1ffcf6c1；私有逐项回执 SHA-256 d96051f5db96d3c74c658412796144af0b93b62c951a2493961b1e3346271c9c。

清理前后普通视图快照 SHA-256 分别为 47ac3e22a9dcb5f3b89fae07338b7a17694885e8ad05734e0c437059045257b1 和 3b081b910eb83d9d567ef7f2d003abc0ca69aace87f899454f4237481da45730。旧正式树、当前 full 树、SQLite 主文件、HKCU 登记和快捷方式保持相同；三份策略文件消失。快照中另有 SQLite 的空 WAL 和 SHM 临时 sidecar 出现；普通会话文件创建时间为 04:00:37 UTC，对应本轮只读数据库回读，早于 04:02:18 UTC 的清理。sidecar 不在审计删除清单，已保留。Owner 已裁定“快速关闭时残留临时文件”为小型已知问题，暂不修产品。

## V01–V08 与合并前边界

| 检查 | 合并前证据状态 |
|---|---|
| V01 | 授权及 MANIFEST blob 未漂移；精确来源云端检查有非零实际执行。 |
| V02 | 当前正式产品入口、受控任务、Outcome/Evaluation/Dream 的实机与云端证据可复核。 |
| V03 | Git draft 和同请求重放已实测；PR #34 尚未合并，accepted fact 的产品回读仍未执行。 |
| V04 | 云端 native/server 的续发、重放、撤销、范围拒绝及 custody 证据可复核。 |
| V05 | 当前实机开放 fixture driver 已执行；其余开放边界、codec、私域负测以同来源云端证据复核。 |
| V06 | 冻结双构建、签名、完整随包、正式安装和资源动态更新均绑定确切字节。 |
| V07 | SAC 强制普通 Win11 的正式安装、受控产品与更新四场景实测；accepted fact 的重启回读待 PR #34 合并后进行。 |
| V08 | 当前复合候选已有独立只读复核；外部 Claude CLI 只读复核尝试返回 403 Request not allowed，异构回执尚未取得；Owner 最终 R2-05 裁定未作。 |

PR #34 仅增加旧 985e17d1 测试所得的单个 test-only Result。Owner 合并该 PR 是明确的接受事实动作；合并前 Controller 不伪造 merge commit、actor 或 accepted-fact 回读。合并后 Controller 可从 GitHub 读取实际 merge SHA，再在当前正式产品中输入 PR 号、SHA 和固定 Result 坐标，核验 merged_by、blob/hash，并重启同一安装重复回读；这一步无需 Owner 操作产品。Owner 还保留最终 R2-05 裁定。外部 Claude 通道的 403 属于当前工具权限缺口，不能写成异构复核通过。

并行收件箱中的 PR #10 三项为低优先级 notices/README housekeeping，不改变本轮实际产品闭环。此时折入会改变已签的安装包字节并使已完成的云端、Win11 身份链失效，因此本轮记录不折入；依 Owner 批准的收件箱收尾要求，PR #10 已关闭、分支未删除。其余三项可在后续独立范围处理。PR #34 保持 OPEN，未合并。

私有实机原始回执保存在 %LOCALAPPDATA%/Packages/<CodexPackageFamily>/LocalCache/Local/gogoke-agent-tmp/r205-wrapup；公开仓库仅记录经过筛选的事实和哈希，不含本机绝对路径、用户名或密钥。
