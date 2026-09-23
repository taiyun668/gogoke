# 2026-09-22 Claude 独立复核记录

本文件记录 Claude 在 gogoke 迁入公开仓库当天所做的独立复核：范围、方法、证据和结论。施工检查点（`artifacts/s1-r4/checkpoints/`）里写的 "Claude 裁定"，依据都在这里。

通用约定：
- 验收只绑定内容（blob、树哈希），不绑定提交 SHA。
- 本机原生编译结果不作为证据（见 `docs/governance/gogoke-build-and-release.md`）。
- 任何一次复核都不自动上调 G1–G5 或到期检查。

---

## 1. 推送前复核（公开迁移第一阶段）

**对象**
- 本地导出 `main@c8f3a5a3`，原远端 `main` 为 `ed5e9711`。
- WIP 分支 `codex/gogoke-s1-r4-native-close-r1@43b2e1a3`、`codex/gogoke-s1-r4-lineage-r1@7a2f8d58`。
- 迁移报告。

**结论：可推送，无阻断项。**

| 检查 | 方法 | 结果 |
| --- | --- | --- |
| 推送方式 | `merge-base --is-ancestor` | `ed5e9711` 是本地 main 的祖先，推送为快进，无需强推 |
| 治理文件 | 比对 blob | 公开副本与来源 `e98d17c8` 同为 `f6eca013`，逐字节相同 |
| 泄漏扫描 | 独立规则（用户名、本机盘符目录、用户目录、邮箱、令牌、私钥块），扫三条引用的已提交 blob | 0 条真实泄漏。命中项均为上游 T3 测试中的虚构路径、AWS 官方示例密钥，以及仅作字符串出现的 PEM 标记 |
| 扫描器自身 | 在临时副本植入含本机用户目录的文件后提交 | 扫描器退出码 1，正确拒绝 |
| 更新链信任根 | 比对 blob | 公钥 `gogoke-release-public-key.txt` 与私有来源同为 `68bee586`；私钥未入库 |
| WIP 分支 | 逐文件比对 blob | native-close 5/5、lineage 2/2 与私有原件相同；两条分支都基于 `06abd420` |
| 随迁的 12 个验收范围 | 按各自提交逐文件比对 | 全部相同 |
| 流水线 | 阅读三条 workflow | 每个 job 都设了 `timeout-minutes`，同分支 `cancel-in-progress`，`contents: read`，无 secret，无 `pull_request_target` |
| 计划校验 | `verify_plan.py --self-test` | 1965 项结构检查通过，21/21 负例被拒 |

**非阻断发现**
- `verify_plan.py` 读文件时未指定编码，在非 UTF-8 区域设置下报 `FAIL_PLAN`（charmap 解码错误）。已由施工方改为显式 UTF-8。
- 原生两条进程轴当时为 0 执行，需推送后在云端实际执行。

## 2. 推送后核实

- 云端原生运行 [35799638957](https://github.com/taiyun668/gogoke/actions/runs/35799638957)：`b07_two_process`、`host_typed_ops` 各发现 1、执行 1、通过 1、跳过 0。已下载 machine-result 核对，绑定 `c8f3a5a3`。
- 桌面 CI [35800598752](https://github.com/taiyun668/gogoke/actions/runs/35800598752)（`0ba04988`）：两个 job 均成功，包括未签名安装包构建和"更新、就绪、回滚"冒烟测试。
- 移动端远程设置 hook（`useMobileServerSetup.ts`，blob `0df84e2d`）与私有 `660dece9`（WP10A 收口）逐字节相同，来源属实。`SettingsView.test.tsx` 是新改写的，不是逐字节随迁。

## 3. PR #1：WP10A 旧编解码器收口

**结论：ACCEPT，已合入（`c77e68ed`）。**
- 删除的 `apps/desktop/src/services/public/codec.ts` 及其测试，与私有 `660dece9` 删除的内容一致。
- 候选内没有残留引用；替代实现 `public-r4/codec/strictJson.ts` 已在用。
- 绑定 `5702d703` 的运行 [35802709546](https://github.com/taiyun668/gogoke/actions/runs/35802709546)：两个 job 都成功，无跳过。
- 合并后 main 的桌面 CI [35803871976](https://github.com/taiyun668/gogoke/actions/runs/35803871976) 通过。合并提交与候选的 `apps/desktop` 树哈希相同（`603bdecb`）。

## 4. N1：发布镜像测试钩子检查器

**结论：关闭。**
- 代码层面：`build.rs` 的 `assert_release_object_omits_test_hooks` 已做到三点：读失败即报错；断言镜像非空；以 `sqlite3_gogoke_bind_main_handle` 作为必须存在的正向符号。
- 这段检查只在 `PROFILE=release` 时执行，最初云端从未跑过这条路径。
- main `9422573a` 的运行 [35805375774](https://github.com/taiyun668/gogoke/actions/runs/35805375774)：`release_build` 记录 executed=true、exit 0、finished_release_profile=true。
- 反例运行 [35805099386](https://github.com/taiyun668/gogoke/actions/runs/35805099386)：反例提交 `b62fc32a` 不在 main 历史中。该次运行 exit 101，日志报 "release SQLite image still contains test hook sqlite3_gogoke_test_enable_full_pathname_reentry"。

## 5. Route-B 核心（私有候选 `6fc8c634`）

**结论：代码层面 ACCEPT，记为 `ACCEPT_CODE_PENDING_CLOUD_TESTS`。**

范围是 native-host 的 28 个文件，其中 10 个有变化：
- Cargo.toml、Cargo.lock、build.rs
- src/main.rs
- store/atomic.rs、store/mod.rs、store/protocol.rs、store/same_open.rs、store/session.rs
- tests/host_typed_ops.rs

**依据**
- **服务认证**
  - 口令：`BCryptGenRandom`（系统首选 RNG）生成 32 字节，只经 stdout 交给父进程。
  - 首帧：必须是只含 `operation` 与 `capability` 的 `AuthenticateService`。
  - 比对：先做格式校验，再做常数时间比对。
  - 未认证通道：只允许 `Shutdown`。
- **权限判定**：移除了客户端自声明的 `accessAdmitted`，改由服务端判定。
- **协议**
  - 新增操作都在封闭白名单内。
  - `decode_flat_string_object` 拒绝重复键、禁用字段、嵌套覆盖和数值强转。
  - `RecordActionOutcome`、`AppendOwnerOutcome` 不作为服务协议操作。
- **atomic.rs 重构后仍保留的保护**
  - 事件计数连续性校验。
  - 流头按预期值 CAS（`changes()==1`，否则 `CounterConflict`）。
  - `BEGIN IMMEDIATE`。
  - 提交失败回滚，并报 `CommitUnknown`。
- **既往发现**
  - F4（关闭账本测试）已修正：单次 close、不重试、UNKNOWN 不记为成功。
  - F3（`extension_admitted` 硬编码）相关代码已不存在。
- **证据绑定**：原生两轴运行时的 `c8f3a5a3` 与当前 main 的 `apps/desktop/native-host` 树哈希相同（`c56dc341`）。

**非阻断**
- m1：`serve_*` 判断 `Shutdown` 用的是在原始文本中查找字符串。若嵌套内容里含 `"operation":"Shutdown"`，服务会在处理完当前请求后退出。能做到这一点的只有已认证方，所以无安全影响，但应改用 `decoded.name`。
- m2：新依赖 `ryu-js =1.0.3`（Apache-2.0 OR BSL-1.0）未登记。Rust 依赖整体都缺许可清单。

## 6. 其余 17 个待重审范围（合并复核）

**方法**
- 每个范围先取候选旁支相对集成线的全部改动文件，再挑最近一次已复核的版本，与公开 main 逐文件比对。
- 格式化造成的差异，用忽略空白的 token 级比对排除。
- 同时检查每个候选自身新增的行是否还在公开版中。

### A. 代码层面 ACCEPT，但缺测试执行证据（`ACCEPT_CODE_PENDING_CLOUD_TESTS`）

| 候选 | 范围 | 说明 |
| --- | --- | --- |
| `33c4251d` | 物理根 | 新增的 poisoned-root 登记、sidecar 检查、线程亲和标记均为收紧 |
| `b7389d3c` | FilePin | 内部 NUL 拒绝完整保留 |
| `f298187e` | 原子 SQLite | Node preflight 不再打开产品 SQLite，是 `3aa0ad59` 的有意演进 |
| `d286e6fc` | Route-B 供应 | SQLite 源哈希校验、`SQLITE_TRUSTED_SCHEMA=0` 均在 |
| `32ece0cc` | 进程接线 | 889/889 行保留 |
| `57f65da9` | 上下文组装 | 必需约束序列校验在，形式已调整 |
| `d51d4e88` | TS 委托 | 完整保留 |
| `7b157164` | 原生委托 | `foreign_key_check` 测试在 |
| `8483bf6c` | 解析器 | 完整保留 |
| `b1a3da29` | ATP | 封存逻辑在，后续加了 material refs；surrogate 处理改为 UTF-16 保真，是 ExecutionRecipe 的有意演进 |
| `dd2807e9` | 持久化血缘 | 完整保留 |
| `8a534197` | ExecutionRecipe | 完整保留 |
| `69b85100` | Action | 完整保留 |

另外，测试文件逐一核对了用例数和断言数，没有发现删测试来凑通过；唯一的例外见 B1。

### B. 需要返工：旁支上已复核的加固从未进入集成线

- **B1 `4be803e6` G1 基础**
  - 以下两个文件在解析和 `canonicalJson` 两处都缺少负零拒绝，对应测试 "rejects negative zero before canonical encoding can change the validated fact" 也不在了：
    - `apps/desktop/src/services/public-r4/codec/strictJson.ts`
    - `third_party/t3code/apps/server/src/gogoke/contracts/strictJson.ts`
  - `public-r4/codec/model.ts` 与 `gogoke/releasePolicy.ts` 的 `Object.freeze` 缺失，常量表在运行时可被改写。
- **B2 `5014096f` 最小服务封存**：`gogoke/bootstrap/minimalService.ts` 的输入快照校验整段缺失，对应测试也不在。这段校验包括：
  - 只接受非 Proxy 的纯对象；
  - 拒绝 symbol 键；
  - 字段白名单和必填检查；
  - 按 descriptor 复制。

这与 WP10A 只随迁一半是同一类问题。施工方应排查其余已 ACCEPT 的旁支修补是否也没进入集成线。

### C. 受阻：WP01（`7ff6f3fb`）、S1-01-V（`eca956db`）

`tools/gogoke-s1-r4/` 没有导出到公开仓库，但计划中的 `RUNBOOK.md`、`CHECKS.json`、`EXECUTION_PLAN.json` 仍引用它。需要先清掉其中的本机路径再导出，或者修订计划。

### D. 云端测试缺口

- D1：`third_party/t3code/apps/server/src/gogoke` 下有 44 个测试文件，没有任何流水线运行它们。桌面 CI 只测 `apps/desktop`。
- D2：`gogoke-native-host.yml` 只运行两条集成测试轴，native-host 的 lib 单元测试（含 `store/authority/*`）从未在云端执行。

A 类要转为验收证据，必须先补上 D1、D2，按执行数记账（executed=0 记 `FAIL_INSTRUMENT`）。Windows Server 的结果要标注"不等于 Windows 11 桌面验证"。

### E. 非阻断

同第 5 节的 m1、m2。

---

## 7. D1/D2 首次云端全量测试（候选 `cfcb8a7d`）

运行 [35810241052](https://github.com/taiyun668/gogoke/actions/runs/35810241052)。machine-result 已下载核对：
- 服务端测试：44 个文件（Vite 35、node:test 9），532 个发现、532 个执行、532 个通过，跳过 0。
- native-host：lib 单元测试 382/382 通过；release 构建和两条进程轴也都通过。

新的运行器 `tools/ci/gogoke-server-tests.mjs` 有三条硬规则：文件集合必须与预期一致；测试数为 0 记 `FAIL_INSTRUMENT`；任何跳过都判为非 PASS。三条都成立。

A 类 14 项据此由施工方绑定到对应测试轴（MC-014）。这不等于到期检查通过。

## 8. PR #3（B1/B2 返工与集中修复）

**第一轮（head `cfcb8a7d`）：B2 须返工，其余 ACCEPT。**
- B1：负零拒绝与 `Object.freeze` 已补回。与 `4be803e6` 做 token 级比对，唯一差异是尾逗号。负零测试已恢复，断言 `UNSAFE_JSON_NUMBER`。
- 集中修复都是真修，没有削弱测试：
  - `nativeHostClient` 的启动握手改为一个监听器按顺序接收，修复了逐行挂监听时丢行的竞态。
  - 管道 DACL 测试改为读二进制快照，断言 protected、只有 1 条 ACE、ACCESS_ALLOWED、`FILE_ALL_ACCESS`、SID 为当前用户，比原来的 SDDL 字符串断言更严。
  - 进程测试只加了诊断信息，原有断言一条没少。
- B2：生产代码与 `5014096f` 一致，但测试是重写的，用例 21→17、断言 53→42，缺少以下覆盖：
  - 数组的重复、稀疏、非字符串元素；
  - null 原型记录的放行；
  - 可选能力取值器不被执行。

**第二轮（head `efa1a6fd`）：ACCEPT。**
- B2 恢复为 21 个用例、54 条断言，上述情形逐项都有覆盖。
- 运行 [35813586330](https://github.com/taiyun668/gogoke/actions/runs/35813586330)：服务端 536/536，native lib 382/382，release 构建和两条进程轴均通过。

## 9. PR #5（m1 Shutdown 判断）

**结论：ACCEPT，已合入（`cdbff122`）。**
- `successful_shutdown` 只在解码出 `Shutdown` 且处理成功时才退出；嵌套字段不再触发关闭。
- 新增测试覆盖正例和反例。
- 回归：native lib 383/383，服务端 532/532。

## 10. PR #4（资格工具六文件）

**结论：审阅通过，维持草稿，不合并。**
- 缺少授权 receipt 时，`auth_from_candidate` 返回错误，fail closed。
- 它仍锚定私有仓库和私有授权提交，在公开仓库中无法满足，因此不可运行。
- 须先由 GPT 起草公开授权锚，再由 Owner 本人在 GitHub 合并，之后按提案改造、再送审。
- WP01、S1-01-V 维持 BLOCKED。

## 11. PR #6（m2 native-host Rust 依赖许可清单）

**结论：ACCEPT，作为清单准确。**
- 四个外部 crate 的版本和校验和与 `native-host/Cargo.lock` 逐项一致：
  - `ryu-js` 1.0.3（Apache-2.0 OR BSL-1.0），运行时依赖；
  - `cc`、`find-msvc-tools`、`shlex`（均为 MIT OR Apache-2.0），只用于构建，不进入二进制。
- 清单放在会被打包的 `apps/desktop/THIRD_PARTY_NOTICES.md` 中，位置正确。

**非阻断，属于发布前事项**
- 建议写明 `ryu-js` 选用 BSL-1.0（二进制分发时无须附带许可文本）。
- 更大的缺口在桌面应用本体：`src-tauri/Cargo.lock` 有 629 个 crate，现有通知基本没有覆盖；打包进前端的 npm 依赖也一样。建议在 CI 中用工具（如 cargo-about）为 Rust 与 npm 的生产依赖生成完整的许可与版权声明，并随安装包分发。
- 此项不是法律意见。
