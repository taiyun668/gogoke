# 32 · Gemini CLI 适配记录

状态：调研完成，待实现
日期：2026-08-23
对应：`31-product-design.md` 第 7.4 节「上架一家的九步」

本稿逐条记录实测结果，**不写没验过的东西**。

---

## 结论先说

**可以适配，而且比预想省事**：Gemini CLI 有 `--acp`，说的是和 Grok 同一套协议，
现有的 `PersistentGrokSeat` 大概率能复用，不必从零写一个适配器。

**还差一步才能上架**：登录成功时的状态输出没观测到 —— 手上没有已登录的 Gemini。
按 INV-PROBE-2，只有否定用例就写判据是不允许的。

---

## 九步进度

| 步 | 内容 | 状态 |
|---|---|---|
| ① | 核实分发方式 | ✅ `@google/gemini-cli` 0.56.0，`bin: gemini` |
| ② | 手动跑状态命令的两种情形 | ⚠️ **只有未登录那种** |
| ③ | 判据 + 双向用例 | ❌ 等 ② |
| ④ | 实测隔离变量 | ✅ 见下 |
| ⑤ | 空目录反向验收 | ✅ 通过 |
| ⑥ | 跨实例共享 / 环境放宽沙箱 | ⚠️ 部分，见下 |
| ⑦ | 写适配器 | 待做（可复用 ACP） |
| ⑧ | 能力边界映射 + 反向用例 | 待做，映射已找到 |
| ⑨ | 席位启停与流式测试 | 待做 |

---

## ① 分发

```
包名   @google/gemini-cli
版本   0.56.0
bin    gemini → bundle/gemini.js
仓库   github.com/google-gemini/gemini-cli
```

`npm install -g @google/gemini-cli` 实测可装（29 秒，5 个包）。

## ④ 隔离变量：认 HOME，不认专用变量

**我先猜了三个专用变量，实测全部落空** —— 这正是「别照抄别家」那条的由来：

| 变量 | 生效吗 |
|---|---|
| `HOME` / `USERPROFILE` | ✅ **只有它管用** |
| `GEMINI_HOME` | ❌ 无效 |
| `GEMINI_CONFIG_DIR` | ❌ 无效 |
| `GEMINI_DIR` | ❌ 无效 |

配置与凭据落 `<HOME>/.gemini`。设了 `HOME` 之后，它确实在新目录下生成了
`.gemini/`、`projects.json`、`history/`、`tmp/`。

> 三家已知的对照：Claude 认 `CLAUDE_CONFIG_DIR`（也认 HOME）、
> Codex 认 `CODEX_HOME`（也认 HOME）、**Grok 只认 `GROK_HOME`、完全不看 HOME**、
> **Gemini 只认 HOME、没有专用变量**。四家四个样。

## ⑤ 反向验收：通过

`HOME` 指向一个空目录后：

```
exit code 41
Please set an Auth method in your <HOME>\.gemini\settings.json
```

**明确报未登录，没有落到宿主上。** 隔离成立。

### 一个必须注意的行为

**不做隔离时（默认 HOME），它会直接弹出交互提示**：

```
Opening authentication page in your browser. Do you want to continue? [Y/n]:
```

这对非交互探测是危险的 —— 会挂在那里等输入。**所以探测必须始终带隔离 HOME**，
不能图省事跑一次默认的。

## ⑥ 跨实例共享 / 沙箱

- **没发现 leader socket 一类的跨实例共享**（Grok 那种）。但这是「没找到」，
  不是「确认没有」—— 上架前应再查一遍它的进程模型
- **沙箱**：有 `-s/--sandbox`（布尔）与 `--approval-mode`。
  未发现能从环境放宽的变量（`GROK_SANDBOX` 那种），同样属于「没找到」
- **认证方式有四种**：`oauth-personal`、`gemini-api-key`、`vertex-ai`、`gateway`。
  产品只应走 `oauth-personal` —— 另外三种是 API key 计费，不是订阅额度

## ⑦ 运行层：ACP，与 Grok 同一套

`gemini --acp` 起来后，发一条标准 ACP `initialize`，它正常应答：

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,
 "authMethods":[{"id":"oauth-personal","name":"Log in with Google"},
                {"id":"gemini-api-key",...},{"id":"vertex-ai",...},{"id":"gateway",...}]}}
```

**形状与 Grok ACP 完全一致**，包括 `authMethods` 数组 —— 而 `PersistentGrokSeat`
里的 `selectAuthMethod` 正是读这个字段。

所以适配器大概率是「把 grok-acp 抽成通用 ACP 适配器 + 一份 Gemini 的参数与认证
偏好」，而不是新写一个。**但这需要在真正接上之后验证**，现在只验到握手。

> 顺带修正一条：官方文档只提 `--output-format stream-json`，**没提 `--acp`**。
> 照文档做会得出「只能一次性调用、做不了持久席位」的错误结论。
> `--help` 才是权威。

## ⑧ 能力边界怎么映射

`--approval-mode` 有四档：`default` / `auto_edit` / `yolo` / `plan`。

| 我们的能力 | Gemini 参数 |
|---|---|
| 只读 | `--approval-mode plan`（文档写明是 read-only mode） |
| 可写工作区 | `--approval-mode auto_edit` |

**不用 `yolo`** —— 那是自动批准一切工具，超出「可写工作区」的含义。

按 INV-ZERO-1，这是**启动参数**，不是配置文件，符合现有做法。
上架前要配反向用例：验证 `plan` 档真的写不了（Codex 那个 camelCase 静默退回
只读的坑，反过来也可能发生）。

## 还差什么

1. **登录一次**，观测：
   - 状态命令在已登录时的输出与退出码（写判据的正向用例）
   - ACP `initialize` 在已登录时 `authMethods` 长什么样
2. 把 grok-acp 抽成通用 ACP 适配器，接上 Gemini
3. 能力边界的反向用例
4. 席位启停与流式测试

**在 1 做完之前不能上架** —— 只有否定用例的判据，正是 `/Logged in/i` 匹配上
`Not logged in` 那类事故的温床。

---

## 本机改动记录

为做这次调研，在本机全局安装了 `@google/gemini-cli@0.56.0`。
撤销：`npm uninstall -g @google/gemini-cli`。

调研产生的临时目录 `<local>/harness-spike/gemini-iso` 用完即删。
