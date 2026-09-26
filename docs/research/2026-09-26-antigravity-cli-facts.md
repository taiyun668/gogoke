# Antigravity CLI（agy）：已知和未知

- 日期：2026-09-26
- 为什么写：2026-09-17 的[要做的和不做的 §三点七](../design/GOGO-要做的和不做的.md)依据 AionUi 的源码注释，判断 Antigravity "一次调用一个进程，中途插话不可能、上下文不累积，只能当一次性席位"。Owner 指出这一块没有摸清。本文把能查到的事实分成已核实、有冲突、未知三类。**本机目前没有安装 agy**（只有 Antigravity 和 Antigravity IDE 两个桌面应用），所以下面没有一条经过本机实测。

## 已核实（有源码或官方文档）

- **安装**：官方文档说支持 macOS、Linux、Windows 原生运行。Windows 的安装方式是 PowerShell 执行 `irm https://antigravity.google/cli/install.ps1 | iex`（[官方安装页](https://antigravity.google/docs/cli/install/)）。
- **上下文可以累积**：claw-orchestrator 的 `src/persistent-agy-session.ts`（基于 agy 1.1.13 到 1.2.9 实测写成的注释）写明：
  - 第一个 `init` 事件会带上 conversation id；
  - 之后每次调用传 `--conversation <id>`，模型就能看到之前的回合，"真正的多轮上下文，跟 Codex 续接 thread 一样"。
  - **所以"上下文不累积"是错的。**
- **输出**：`--output-format stream-json`，一行一个 JSON，包括 `init`（带 conversation id）、`step_update`、`result`（带回复和真实 token 用量）。
- **这种做法下每回合起一个新进程**：claw-orchestrator 就是每次发送都起一个 `agy` 打印模式的进程。
- **其他几条来自同一份源码注释的实测**：
  - 1.2.6 起，无头运行默认不设超时，必须自己传 `--print-timeout`；
  - 1.2.9 起，超时到了会以退出码 0、状态 SUCCESS 结束，只在 stderr 里说明；
  - 模型名传错时，返回空回复加 `status: ERROR`，stderr 里什么都没有；
  - 工具被拒时，只在日志里有一行 `soft-denying tool confirmation`；
  - stream-json 里没有模型字段，读不回它实际用了哪个模型。
- **Google 有一个官方的 ACP 内核** `agy_acp_server`：ACP v1，走 NDJSON，闭源，按操作系统从 Google 下载，大约 320 MiB，第三方 Apache-2.0 的 paseo-agy-acp 就是接它的（[README](https://github.com/tiezbro/paseo-agy-acp)）。
  - 在它的做法里，一个连接器进程对应一个内核子进程，也就是**常驻会话**；
  - 支持 `session/request_permission`；
  - 模式有 `default`、`auto_edit`、`yolo`，**没有 plan 模式**（Paseo 的 plan 被映射成 default）。
- **官方 CLI 仓库里"加 ACP stdio 模式"的功能请求还开着**（google-antigravity/antigravity-cli #31，2026-05-20 提出）。所以 `agy_acp_server` 是独立下载的内核，不是 `agy` 命令本身的一个模式。

## 有冲突，要实测

- **能不能用 stdin 持续输入**：有第三方汇总说 agy 支持 `--input-format=stream-json --output-format=stream-json` 双向流，但另一份第三方指南说只支持 `-p` 一次性运行。两份都不是官方文档。
- **有没有只读、计划模式**：一份指南说有 `--mode plan` 和 `--sandbox`（"开启终端限制"）；ACP 内核的模式表里没有 plan。
- **中途插话**：打印模式下每回合一个进程，插不进去；如果双向流成立，就可能插得进去。ACP 标准本身没有"引导正在进行的回合"这个方法。

## 未知

- `agy_acp_server` 有没有 Windows 版，有没有签名，智能应用控制会不会拦。
- ACP 内核支不支持 `session/load`（续接）。

## 对设计的影响

- 设计 37 里"席位要求的能力 × 实例提供的能力"这种绑定检查仍然需要，但 **Antigravity 能不能当主控，现在不能下结论**，要等实测。
- 做旁聊和当施工席位，至少有 `--conversation` 续接这条路可以走。
- 只读：如果没有 plan 模式，就要靠我们的宿主来保证。例如通过权限请求自动拒绝写操作，再加路径围栏。

## 实测要回答的问题（需要在本机安装 agy）

1. `agy --help` 里的全部参数，确认有没有 `--input-format stream-json`、`--mode plan`、`--sandbox`。
2. 双向流能不能在一个进程里持续做多轮，能不能在回合进行中写入新消息。
3. `--conversation` 续接后，上下文是不是真的在。
4. Windows 11 智能应用控制下，`agy` 和 `agy_acp_server` 能不能运行。
