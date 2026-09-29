# R2-06a Owner 机器测试遗留物：待独立审计的清单

**状态：2026-09-27 预清理盘点；尚未删除。** 范围只含本轮 R2-06a 测试遗留物。所有路径是当前用户路径占位，不含本机用户名。`<CodexPackageFamily>` 是 Codex 的已安装包族名。Codex 派生进程的逻辑 `%LOCALAPPDATA%` / `%APPDATA%` 与普通视图同名目录不是同一个物理对象；删除必须使用清单指定的视图。正式安装 `%LOCALAPPDATA%\Programs\gogoke`、正式数据 `%APPDATA%\app.gogoke.desktop`、正式快捷方式和任何正式登记均不在清理范围。

## 已核身份与字节

| 对象 | 当前精确身份 | 证据用途与清理视图 |
| --- | --- | --- |
| 打包视图候选根，逻辑 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-install-b025`，物理 `%LOCALAPPDATA%\Packages\<CodexPackageFamily>\LocalCache\Local\gogoke-agent-tmp\r206a-win11-install-b025` | 14,226 文件、1,087 目录、469,860,460 字节，无重解析点。与旧普通视图 `ordinary-after-install.json` 的 14,226 个文件逐相对路径、长度、SHA-256 全相等，缺失/额外/变化均 0；参考树 SHA-256 `5950cdd1490c09ba0648c20be4e580d9b8f2e1621562e7ad29c45a5184d3c72f`，本地全树清单 SHA-256 `b7022b44a1b841d25d8b08a583e80ec22dffea41c9a7a416ea02e1129b83b43f`。 | MC-138–145 引用；清理前重核同一包视图的候选登记、实例 ID、根路径、全树哈希。只能在该包视图中删精确候选根，不能跨视图调用旧卸载器或触及正式根。 |
| 打包视图 `HKCU\...\Uninstall\gogoke-candidate` | 候选用途 `CI_CANDIDATE_RESOURCE`、旧 b025 实例 ID 的 SHA-256 `827b6b4749ade20acfdbd367346804179f9c0ba4dc26663d69451eeb7bf7fd02`；登记仍存在。 | MC-138–145 引用；在同一包视图核精确 `InstallLocation`、`UninstallString`、`InstallInstanceId` 后只删该候选键。正式 `gogoke` 键保留。 |
| 打包视图候选数据，物理 `%LOCALAPPDATA%\Packages\<CodexPackageFamily>\LocalCache\Roaming\app.gogoke.desktop.candidate` | 2 文件、1 目录、548,962 字节；`product-authority/.gogoke-state.sqlite.custody-v1` SHA-256 `17874899367622f8872bd69e029eed6bf0d21b2d6c014c86800bb67d0821c6e4`；`product-authority/state.sqlite` SHA-256 `d7ba659901f70f020122222794d4f7f9174cd0ee1f4ea74af4e46f725119457c`。 | `candidate-data-final-packaged.json` 与 MC-145 引用；只在包视图按同一物理身份删候选数据，不碰正式数据。 |
| 打包视图候选登记锁及生命周期锁 | `%LOCALAPPDATA%\gogoke-registration-CI_CANDIDATE_RESOURCE.lock` 0 字节、SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`；包视图 `%LOCALAPPDATA%\gogoke-agent-tmp\gogoke-install-lifecycle.lock` 392 字节、SHA-256 `a3639af5e23464fc391dae127fd5493f02c8178068a35866d9c7fdd35dcd623f`。 | 旧候选锁域；核无 Gogoke 进程、普通文件、可独占打开后按包视图分别删除。 |
| 普通视图登记锁、生命周期锁、卸载回执 | 登记锁 0 字节、SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`；生命周期锁 647 字节、SHA-256 `2f6614fa28dd0bc3a880913e92e23d784d656cfa89b6e692f6d598f9d435d82f`；回执位于普通视图 `%LOCALAPPDATA%\gogoke-agent-tmp\gogoke-uninstall-<old-instance-tag>-<nonce>.json`，198 字节。 | `ordinary-residue.json` 和 MC-144/145 引用；旧回执必要内容在检查点与本单保全。普通视图独占核身份后逐文件删，不用递归删除整个任务临时父目录。 |
| 普通视图候选数据 `%APPDATA%\app.gogoke.desktop.candidate` | 2 文件、1 目录、548,962 字节；custody 文件 SHA-256 `d5e3bb0f1c203979c57576068f718282f31e2794be025034e0e9193fade03b08`，SQLite SHA-256 `f3a446ae27218ce5b52319d8475b57e9c7c57d2bd6b3d7d772becc590090ba56`；旧树 SHA-256 `498070a77bddc1d137cd0d5c10e30fe99cf05168709dcab0045aaf710ba4466d`。 | `ordinary-after-product.json` / `ordinary-after-uninstall.json` 引用；独立审计若确认旧证据已保全，可在新字节 Win11 重测**前**按精确旧哈希清理，以建立全新数据基线。审计未过则保留。 |
| 普通视图旧候选根 `%LOCALAPPDATA%\gogoke-agent-tmp\r206a-win11-real-install-b025` | 0 文件、1,087 个空目录、0 字节，候选 HKCU 登记缺席。 | MC-144/145 引用；新测试使用另一个精确目标，确认仍全空、无重解析点后在普通视图删该根。 |
| MC-122 两文件副本 `%TEMP%\gogoke-r206a-evidence-a3b6-36293571078` | 2 文件、2 目录、3,912 字节；两个原始回执 SHA-256 分别为 `47962e4378b076b0088506b857cd8047a15fe6e4eaca0bfe25d92d019472df99`、`cb74c05fd784552d43879d2be3a2fcd00f5757d6cce6e1264f2b5e5d02d86163`。 | MC-122 及后续检查点引用；必要云端状态已在 MC-122 保全。独立审计通过后才按精确 `%TEMP%` 根删除。 |
| Codex 包 `LocalCache\Local\gogoke-agent-tmp\r206a-win11-b025` 原始实测证据 | 61 文件、1 目录、76,502,822 字节，本地全树清单 SHA-256 `bed769dfcd8d6cfb3fe9c7dd0174fa453b3ee489d3bac5e21c58f8ed221d03ab`。 | MC-137–145、G3 报告和旧准备单引用；先把必要结果、原始回执 SHA-256 和脱敏摘要推送，再经独立审计清理物理 `LocalCache` 精确目录。 |

打包视图旧候选目录的首次 Python 清单把 547 个超过传统 Win32 路径长度的文件误记为重解析点。扩展长度路径重算后，14,226 文件逐项匹配，重解析点 0；**清理只能用修正后的比较结果**。两份本地比较回执分别为 `packaged-candidate-compare.json` 和 `packaged-candidate-compare-v2.json`，位于本任务 `r206a-g4-readonly` 临时目录。

## 本任务的其他临时目录

这些目录位于 Codex 包视图的 `%LOCALAPPDATA%\gogoke-agent-tmp`（普通进程见其物理 `LocalCache\Local` 路径）。下表清单 SHA-256 是按每个相对路径的类型、长度和逐文件 SHA-256 排序后形成的本地清单摘要；不是把目录本身当作一个文件来哈希。每项无重解析点。它们承载旧云端下载、审计和测试工作副本，关键 run/artifact/hash 已写在 MC-122–145 与准备单；需复核引用后才能逐项清理。

| 相对目录 | 文件 / 目录 | 字节 | 清单 SHA-256 |
| --- | ---: | ---: | --- |
| `r206a-3bd-audit-evidence` | 24 / 2 | 4,009,835 | `a814f83525511e48fa12ce68d34377690405e235eb8245b6ce06346e2a5c7270` |
| `r206a-3bd-build-evidence` | 2 / 0 | 2,157 | `2cf95247c0cd44db6362c372f02f8db7179578ca5f59ec9c3efcc6bdc7de04e8` |
| `r206a-b025-audit-evidence` | 25 / 2 | 300,909,299 | `351813d52472ab54a101e5875f15d0e16259946f86de361662bf4551c02dfe62` |
| `r206a-b025-build-evidence` | 2 / 1 | 2,159 | `39630c01bc953f299a1e6e51237314c223442596eba38d8da630899883467f20` |
| `r206a-b025-signed-candidate` | 10 / 1 | 345,694,856 | `a649fd136d5c897ffe3823d7bb04f6736d59c959e611f82d0fd56f39d4c3a108` |
| `r206a-b025-smoke-evidence` | 4 / 2 | 6,977 | `9dfc88a558deaa74561e8adbc7cdf9f31406806022d7261c3ab6494e82f0d186` |
| `r206a-final-audit` | 36 / 3 | 417,948,611 | `df76c7d5e46486a0e818ad9d4bd86d9942097ce359b7ca41e2c056c531a25874` |
| `r206a-win11-baseline` | 1 / 0 | 1,606 | `281f5a2a6ef058ef615fa712eedcc0fef11f6a532501e8b9bbf9eba1a6f9ea8e` |
| `r206a-win11-prep` | 9 / 0 | 232,524,211 | `775e4273d418579803c8e06cb9e8edf0ec3edb00288ff794619a3fe4a9056b27` |
| `r206a-win11-prep-3bd` | 10 / 1 | 345,669,986 | `aa4acc0ee1729e02080c8ddf90fddd9369d3686ea41696133ea8067cec8b2006` |
| `r206a-g4-readonly` | 尚在使用，待最终盘点 | 尚在使用 | 尚在使用 |

`r206a-win11-b025` 与 `r206a-win11-install-b025` 已在上表单列，不重复计。`mc031-runner`、`mc032-controller` 不属于本轮 R2-06a，排除。最终新字节 Win11 实测的临时目录、回执、哈希会追加到本单；审计未通过或自动审批拒绝的项原样保留。

## 已提取、可在删除原始目录后使用的证据

下表只保存旧 b025 实测关键原始文件的哈希；其必要结论见 MC-140–145、旧准备单、G3 报告。原始 JSON 内含本机绝对路径，不能原样提交到公开仓库。

| 原始相对文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `formal-before.json` | 3,334,116 | `bc261d03cbcc12f38cc3a01fef31e4425ea2a2c5c5a1a617a8783b5d272db0e3` |
| `ordinary-before.json` | 5,881,913 | `9fdc4798295e01ab3242a6d76d2d7226b071c3fda6f14b6b49e0bb5d80f251c1` |
| `ordinary-after-install.json` | 12,648,187 | `93d22919e0b2e7559c97b35f62853d634ce2e9b87c7db0022bb78c0ae87ee9c1` |
| `ordinary/product-corrected.json` | 3,817 | `a13bb9ee58194361c8fa669e1df37e66662df0727226358cd1101d48c88b9e9a` |
| `ordinary-after-product.json` | 12,649,666 | `35db1574062609d03e839460b4cfe47a45358df6361ea4ff0c12c284bb5afda1` |
| `ordinary-uninstall-corrected.json` | 938 | `f6b0dc17d8587b0a2465d9f889eb9344a880f7369ea5f32c37297cec0f77cc3e` |
| `ordinary-after-uninstall.json` | 6,164,570 | `1e9975db0a5b0fcc01bc6e4d8c7ea1d9401b7f9e386e5bc1536ba496cfef6ac8` |
| `ordinary-formal-shortcut-once.json` | 875 | `95d7154d7a7739a5eae5a6b13a5f8739bb0b90323d59d66afbff1406ec1970a6` |
| `ordinary-after-formal-launch.json` | 6,164,570 | `47956f7e731ee411f8a7d244f203b466a9424c6131f06cb4855fbb2e0295f5b0` |
| `ordinary-residue.json` | 777 | `7a48bd499c73aae4e11d7f5a200233cfd36664f159ece13ac6a5fd4293f86edf` |
| `candidate-data-final-packaged.json` | 800 | `aab29facbe80d95f41cd0ff97e42182c5566b68cc7fcbea839ee9bf337cfa1a5` |
| `uninstall-completion.json` | 761 | `931a67c9681a7489053aeb333553dfe5fe5c1fc2b54303b741032f14a70d240b` |
| `candidate-corrected-home.png` | 160,341 | `b6f1aeea747195d7635923528d7062b878e211f890f94cd062a8c9bb9689d815` |

旧普通视图正式安装树 12,989 文件的 SHA-256 在安装前、安装后、产品后和卸载后均为 `12718c0f75088df2e58bdda03e1bd552ce3450893c26be559bacd12b50a21515`；正式数据树在正式快捷方式复启**之前**均为 `a6cf031c11494970dc3d22d7ec61f2696027cd1ff7e60b4b98e97c6fac4c71a8`。快捷方式复启后仅 `.window-state.json` 内容改变，不能把复启后的正式数据当作旧基线。普通视图 SAC 状态 1，相关 Code Integrity 基线 RecordId 7097，产品与卸载后的该日志未见新事件。

## 删除顺序与停点

1. 完成 G1/G2 新字节云端和普通 Win11 实测、冻结正式版前后快照；列入新测试残留并更新本单。
2. 推送已脱敏的必要证据与哈希；新独立审计席位只读核每一项的归属、引用、当前身份、视图、精确删除方法以及正式版排除边界。
3. 仅逐项删除审计通过且当场重核哈希/无重解析点/无 Gogoke 进程的对象。对递归目标先核解析后的绝对路径仍在本单精确目标父目录；打包候选先重核完整文件清单和包视图候选登记/实例。删除后逐项按同一视图回读缺席。
4. 任一项不匹配、审计未通过或工具自动审批拒绝即保留该项，写入检查点，不另走清理路径。正式版任一字节/数据/快捷方式/登记变化即停止全部施工、保全现场。
