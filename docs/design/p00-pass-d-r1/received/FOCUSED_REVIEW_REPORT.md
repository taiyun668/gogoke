# P00 Pass C R3 — C2-F01 独立定向复核报告

## 结论

**C2-F01：证据缺口可关闭。B03：证据缺口可关闭。**

```text
P00: NOT_ACCEPTED
code-entry gate: CLOSED_PENDING_GPT_P00_REVIEW
runtime_verified: false
Pass D completed: false
Owner code-entry authorization: false
```

本轮保留原独立冻结集合，不重做整仓 Pass B；B01、B02、K16、K17 的 `CLOSED_AS_EVIDENCE` 结论按原字节和语义字段保全。没有修改生产代码、现有测试或 PR。这里关闭的是证据源连接缺口，不是生产实现缺陷，也不是 runtime 通过。

## 固定对象

| 对象 | 固定身份 |
|---|---|
| 生产源码 | `bc665a852833952b76d9508401193bedd2198436` / tree `746024abeb2e6d35ac88fd1b2e315fceb0e9f54f` |
| R2 payload / 登记 | `5d15a11ecd33763df31172dbdd3426f9f740ab0c` / `3ed20ef3b84e8b26ac2985337da6c87aae3e7ed2` |
| R3 payload | `c055e125531e166cb67a45531a3c7a8aa9d70330` / tree `bfc7908f28a850594d74beb537f49be9d614ccdc` |
| R3 登记 | `7427b1eaba920e30a188623cea05807df9114ee5` / tree `8582a032c7bb7ef98d2f1dc8a0045e0cf6615b68` |
| 原独立冻结包 | SHA-256 `685936270e4b4ae45354140c0aebbc05079c63e46abe197944e5b34c58d88507`，未重写 |

R3 交付 ZIP SHA-256 为 `58f15f570d50c3d4b0e210c279b2dd0ac89091ec2490ab3eec769723af6fa4a9`；包内 SHA256SUMS 全部通过。固定源码 ZIP SHA-256 为 `cbbe3204755c6066fc0f3a07d9bcab1ef7283c56def81f1e22f2023e844f5f0e`。源码 HEAD/tree/parent、`git fsck --full`、无 remote 及 2,806 个 tracked blob 原始字节身份均通过。

## C2-F01 逐项判断

### 1. 两处符号映射

- `confirmOwner` 已更正为 `InstanceStore.noteProcess`，固定在 `packages/room/src/instances.ts:325-338`。
- `startSeat` 已更正为 `ensureSeat` 与 `ensureSeatInner`，固定在 `packages/room/src/server.ts:1488-1496` 与 `1498-1701`。

R3 将 14 条 selection row 拆分为 34 个具体源码符号 token、9 个显式语义标签，并登记 15 条 caller edge。独立 AST 复核逐一核对 path/kind/line/declaration digest/Git blob/文件 SHA；恢复两个旧名称、把 `noteProcess` 伪装成语义标签、或删除 `ensureSeatInner → noteProcess` 调用边均被独立反证拒绝。

### 2. `noteProcess` 身份未知与保存失败

固定源码在缺记录、缺 busy、seat 不匹配、缺 PID 或 `processStartedAt` 返回空时直接 `void` 返回。`processStartedAt` 把无输出和命令异常/超时都折叠为 `null`，因此不能将空结果解释成“进程已退出”。对 managed/host 两个分支，身份先写内存，随后调用可失败的 `save()`；`save()` 仅执行 mkdir、固定 `.tmp` 写入和 rename，没有在该方法中提供 fsync、锁或回滚。

R3 的 CJ-O02/CJ-O03 已将 `UNKNOWN/NOT_RECORDED`、内存身份、持久化记录与耐久确认分开，并把残留 custody、root/writer fence 继续交给 WP02/WP03。证据足以关闭原 source-join 缺口；生产行为没有改变。

### 3. `ensureSeat` 异常释放与残留 writer

`ensureSeatInner` 先调用 `claim`，后设置 `claim.instanceId`。若 claim 内部先改内存而保存失败，外层 marker 仍为空，catch 不会调用 `releaseAllFor`。在 `seat.start()` 成功后，`noteProcess` 保存或后续 home probe 失败会进入外层 catch；`releaseAllFor` 只清匹配且非 orphan 的 busy 记录并保存，不检查、关闭或终止子进程。回写失败还会发生在内存清除之后，并可能覆盖原始异常。

R3 的 CJ-O01/CJ-O04 已明确：claim attempt、durable claim、startup owner、cleanup intent、busy release、process exit 与 writer release 是不同事实；未知 PID/后代必须保留 residual custody，未满足 WP03 条件前不得授权第二 writer。该责任连接满足 C2-F01 的修订要求，也没有把 Room/InstanceStore 整体内化为产品权威。

## 重放和独立复核

| 检查 | 实际结果 | 证据层次 |
|---|---:|---|
| `verify_symbols.mjs` 两次 | exit 0；14 个负例全部拒绝；两次输出及发布结果一致 | 作者脚本的被动 AST/元数据检查 |
| `targeted_fixtures.mjs` 两次 | exit 0；31/31；两次输出及发布结果一致 | 固定源码方法体 + 假依赖；无真实 FS/PID/CLI |
| 独立 AST/source-join 检查 | 最终 exit 0；124 checks；0 errors | 不导入作者 checker；逐声明、调用边和语义分支复核 |
| 独立负例 | 总体 exit 0；4 个变体内部均 exit 2 被拒绝 | 恢复旧名称、语义伪装、删 caller edge |
| 独立 preservation/object 检查 | exit 0；250 checks；40 个发布对象 | R2/R3 字节、Git blob、对象摘要、已关闭项保全 |
| 原 R2 `verify_repair.py` 辅助重放 | exit 0；发布结果逐字节一致 | 只证明既有 tuple/结构检查，不单独证明 C2-F01 |
| 固定源码 tracked-byte 检查 | exit 0；2,806/2,806 | 原始 Git blob 身份 |

31 个假依赖场景观察到的“通过”包括旧行为按预期出现，例如 unknown query 后仍继续、claim-save 在 marker 前失败、release-save 覆盖原异常。这些结果证明证据描述与固定源码一致，不证明行为已被修复。

## 保全与对象摘要

独立保全检查比较全部 R2/R3 公共文件，只有以下四个公共证据文件发生预期变化：

1. `gogoke-codex-decoupling-p00-gpt-pass-a-v3.json`
2. `gogoke-codex-decoupling-p00-gpt-pass-a-v3.md`
3. `gogoke-codex-decoupling-p00-gpt-pass-a-validation-v3.json`
4. `p00-pass-c-b01-b03-r2/reuse-contracts.json`

`reuse-contracts.json` 仅 selection rows 9/12 和两个 R3 指针变化；其 imports/writers/lifecycle/root/permission/remaining-runtime 字段不变。R2 收到的独立报告、发现、逐项结果和 receipt 与先前交付逐字节相同。40 个发布对象的 SHA-256 与 Git blob、payload changed-blob map、5 个登记控制对象均完成核验。

## 非零尝试

命令账本保留 4 个非零尝试：Git safe-directory 拒绝、一次 wrapper 参数错误、独立检查器三项分支混淆假阳性、preservation checker 调用接口错误。它们分别在 C011R、C039R/C040、C041R 得到可解释修正；没有删除失败日志或降低候选要求。详情见 `INSTRUMENT_NOTES.md` 和 `ACTUAL_COMMANDS.*`。

## 未执行 runtime 轴

真实 CLI/账号/登录/付费、PID/后代/强杀/旧 writer、真实锁与磁盘耐久/崩溃迁移、daemon/remote/RPC、UI/权限/隐私/音频/麦克风、用户 Git/PTY/Files、产品构建、安装更新、rollback、签名发布及平台矩阵均未执行。Stage1、census、旧 49 场景和产品测试套件也未作为 R3 新执行。

## 最终 GitHub 状态与后续

R3 分支 HEAD 为登记 `7427b1...`；R2 分支仍停在 `3ed20e...`。PR #6 仍 open、未合并，HEAD 仍是原审计 snapshot `55a8443...`，base 仍是生产基线 `bc665a8...`。本轮没有移动或合并 PR。

C2-F01/B03 可交 Pass D 与 Owner 做独立后续判断。该证据关闭本身不授权任何生产工作包，P00 与代码门继续关闭。
