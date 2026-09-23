# 范围、固定版本与验证限制

## 固定版本

| 仓库 | Commit | 分支 | 许可证 |
|---|---|---|---|
| Gas City | `19e0862cf8bae6cc631d2828e77bd63878bbe7fd` | `main` | MIT |
| Gas Town | `649b832b7672bc7a2dbef26f5983aba6198b819b` | `main` | MIT |
| Beads | `18c24a3ac3e737c34cb478d58ecf3a805a99e996` | `main` | MIT |
| Omnigent | `0da23edd006a25626ecebe3394fbe54906be7557` | `main` | Apache-2.0 |
| Claudexor | `810627150bccca207d7d798d6baf4898a3bb633a` | `main` | MIT |
| Mission Control | `17186288ef28341723999a040b3b7baa55427a2c` | `main` | MIT |

克隆副本位于 `research/upstream/`。这些 commit 是本次审计的证据边界；上游之后的变化不自动进入结论。

## 审计问题

1. 是否存在真正可扩展的 Harness/Runner 边界？
2. 任务、会话、Agent 身份和项目是否被正确分离？
3. 状态来自结构化证据还是终端文本猜测？
4. 并发认领、重复事件、失败恢复和终态写入是否有明确语义？
5. 上下文是否可选择、可哈希、可轮换，而不是只看 token 百分比？
6. 多项目和项目内 Agent 隔离是否落实到每次查询、命令和事件？
7. Windows 能力是原生、降级、WSL 还是未证明？

## 验证等级

- `SOURCE`：已读取实际实现。
- `TEST-SOURCE`：已读取测试/符合性测试，但未在本机执行。
- `DOC`：仅文档声明，不作为实现已完成的证明。
- `NOT-RUN`：本机未执行。

本机没有 Go 工具链，因此三个 Go 仓库的测试均为 `NOT-RUN`。Node.js 24、pnpm 11 和 Python 3.11 可用，但三个 Python/TypeScript 仓库的依赖未安装；为保持审计副本不引入下载和构建副作用，本轮没有安装依赖，测试同样为 `NOT-RUN`。因此报告不会把“存在测试文件”写成“测试已通过”。

## 代码规模仅作导航

| 仓库 | tracked files | 名称匹配的测试文件 |
|---|---:|---:|
| Gas City | 5,172 | 2,273 |
| Gas Town | 1,563 | 580 |
| Beads | 3,531 | 1,549 |
| Omnigent | 3,752 | 1,911 |
| Claudexor | 1,598 | 402 |
| Mission Control | 800 | 259 |

文件数量不等于覆盖率，只说明进一步验证的资产规模。
