# R2-05 Owner Win11 实机候选包

这是测试候选，不是已接受成果或 release。产品源码固定为 `985e17d1ffdb19ef05ca5f6bc0a8970a66df3f0d`；PR #27 仍为 Draft。[native/server 运行 36066883566](https://github.com/taiyun668/gogoke/actions/runs/36066883566) 与 [desktop 安装运行 36066910163](https://github.com/taiyun668/gogoke/actions/runs/36066910163) 均实际 checkout 此 SHA。云端是 Windows Server 证据，不代替 Owner Win11。

从 desktop run `36066910163` 下载 `gogoke-windows-unsigned`（artifact ID `10837862139`）。其中有 `gogoke-0.1.2-windows-x64-unsigned-setup.exe`、portable ZIP 和 `SHA256SUMS.windows`。installer SHA-256 为 `5bf0e841a94a6a9127d435d13d20cd516c4d90d85b64500a59964ad61cd33f45`，portable ZIP 为 `b053d0faf9bfe004cf311dcedc52e1a05956e0dc2b5859a81f462d6e809ac4cb`。清单尚未经过 Owner 离线签名，不能据此发布。GitHub 当前显示该 artifact 到期时间为 `2026-12-23T22:20:56Z`。

在 Smart App Control 强制模式的 Owner Win11 上使用正式安装包，记录安装路径、版本，以及安装后的 `gogoke.exe`、`gogoke-native-host.exe`、`gogoke-service/runtime/node.exe`、`gogoke-service/dist/bin.mjs` SHA-256。云端安装后这四项依次为 `333c56ea63da2c1ded3a260a2db546618ebcc38af7393a02b509ac549147ed1d`、`a3854c8146fb8df3a8dbf0bbc6fb2bdec0512f87970f6a45a5c9f09b99178648`、`e3be0545990c90995d7bf3a7af5d64af1f2e0fc1bbd9b79c27f7abc1e9676e50`、`e266112f295e1f764a6af36232ac57e2fe1dee20fb674696915665822d2200fc`。Tauri 在 NSIS 打包时会改主程序的一个 bundle-type token，因此 portable 主程序哈希与安装版不同；此差异已按锁定版本源码和实际字节复算闭合。

产品所在用户上下文里的现有 `gh` 登录应返回 `taiyun668`；不要记录或粘贴 token。Home 依次使用 **Verify local path**、**Run test and save draft**、**Run open fixture driver**，记录 native 准入、测试结果、Outcome/Evaluation/Dream 提案和两个不同的测试 draft commit/path。核 draft 只进入 `s1-r4-ledger-test/r2-02`，并从 GitHub 回读实际字节。重启正式安装的 Gogoke 后重复固定测试动作，检查精确回放且没有新的 Git 写入。逐个记录主程序、native-host、Node runtime 的 SAC 启动结果；若被拦截，记录组件、版本、SHA-256 和 Code Integrity/进程现象，不关闭或绕过 SAC。

测试 branch 不是生产账本，禁止并入 main。Draft、review、已有治理 PR 的接受事实回读和测试成功都不是本次结果的 adoption 或 Goal Acceptance。同一安装 root 丢失协调库后的已接受结果恢复、Claude 异构审查和 Owner 最终裁定仍需各自的精确证据。
