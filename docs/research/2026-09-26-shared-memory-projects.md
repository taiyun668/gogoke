# 共享记忆项目：可以借什么

- 日期：2026-09-26
- 为什么查：设计 37 里有三类记忆：席位账本、项目记忆、秘书长的全局记忆。它们要跨厂商、跨会话，还要按项目隔离。Owner 提示可以参考做共享记忆的项目。
- 方法：用 GitHub API 读 README 和项目元数据，只读。深度只到 README 层，要借的点动手前还要读源码。

## 1. 对象

| 项目 | 规模 | 许可 | 它的主要做法 |
|---|---|---|---|
| **claude-mem**（thedotmack/claude-mem） | 94.7k★ | Apache-2.0，TS | 用各家 CLI 的**钩子**自动记录工具使用，生成语义摘要，供以后的会话检索。支持 Claude Code、OpenCode、Antigravity CLI、Grok Bot（没有钩子的就监视聊天日志）等。有常驻的 worker 服务和网页查看器。**默认用它的托管服务**，本地运行要另选 |
| **mem0**（mem0ai/mem0） | 66.0k★ | Apache-2.0，Python | 记忆层 SDK：记忆按 `user_id`、`agent_id`、`run_id` 这几个维度**划作用域**，检索时按作用域过滤。有实体链接和按时间检索。给各家编码助手提供了 skill |
| **Graphiti**（getzep/graphiti） | 31.2k★ | Apache-2.0，Python | **时间知识图谱**：每条事实都有**有效期窗口**，记得什么时候成立、什么时候不再成立；每条派生出来的事实都能**追溯回原始数据**（episode）；检索时语义、关键词、图遍历混合使用 |
| **supermemory**（supermemoryai/supermemory） | 30.9k★ | MIT，TS | 从对话里提取事实，**处理随时间的变化、互相矛盾，并自动遗忘**；给 Claude Code、Codex、Cursor、OpenCode、Hermes 等做了插件，也有 MCP 服务 |
| **cognee**（topoteretes/cognee） | 31.0k★ | Apache-2.0，Python | 给 agent 用的记忆平台（知识图谱加向量检索） |
| **Letta**（letta-ai/letta） | 24.9k★ | Apache-2.0 | 有状态的 agent：记忆、身份、对话长期保存；空闲时由后台 agent 整理记忆（sleep-time） |
| **MemOS**（MemTensor/MemOS） | 11.6k★ | Apache-2.0 | 自己会演化的记忆操作系统 |
| **ByteRover**（campfirein/byterover-cli） | 5.0k★ | 许可待核 | 把项目知识整理成一棵"上下文树"，**在 22 种以上的编码 agent 之间共享**；有共享空间，可以按项目、按团队组织 |
| **Basic Memory** | 4.0k★ | **AGPL-3.0** | 用 Markdown 文件存知识，通过 MCP 提供给各家。许可证是 AGPL，不适合搬进来 |
| **Hermes** 的 memory 和 Curator | —— | MIT | 前面已经读过：两类目标（agent 笔记、用户画像）；空闲时整理，确定性的部分默认开，LLM 合并手动开，钉住的内容不动 |

## 2. 对照我们的三类记忆

| 我们要的 | 可以借谁 | 借什么 |
|---|---|---|
| **席位账本的采集**：各家 CLI 的过程都要记下来 | claude-mem | 它覆盖多家 CLI 的方式：有钩子的用钩子，没钩子的看日志。我们的主路径是协议层的事件流（见设计 37 §3），它的钩子做法可以作为兜底 |
| **记忆的作用域**：项目、全局、席位分开 | mem0 按 `user_id`/`agent_id`/`run_id` 划作用域；ByteRover 的共享空间；ChatGPT 的"仅项目记忆" | 我们可以对应成三个维度：**作用域**（项目或全局）、**席位**、**会话**。检索时强制按作用域过滤，这样项目隔离是结构保证的，不靠自觉 |
| **长期记忆里的事实**：要带有效期、出处，能被推翻 | **Graphiti** | 最贴合我们会话架构报告第 10 条"事实带有效期和出处"的做法：每条事实有有效期窗口，并且能追溯到原始数据。新事实推翻旧事实时，旧的不删，只标注失效时间 |
| **遗忘与整理（做梦）** | supermemory 的矛盾处理和自动遗忘；Letta 的 sleep-time；Hermes 的 Curator | 整理过程分两步：先做确定性的过时和归档，再做需要 LLM 的合并（手动开启）。钉住的不动。我们自己的边界不变：**规则、偏好、权限三类记忆，做梦永远不能改** |

## 3. 要注意的

- **隐私**：claude-mem 默认用它的托管服务，ByteRover 和 supermemory 也有云同步。我们的记忆只能放在本地，不能默认上传到第三方。
- **许可**：Basic Memory 是 AGPL；ByteRover、Memori 的许可证 GitHub 上显示不明确，要核。其余多数是 Apache-2.0 或 MIT。
- **运行时**：mem0、Graphiti、cognee 是 Python，Graphiti 还需要一个图数据库。放进我们的 Tauri 壳，要么只借数据模型自己实现，要么作为单独的服务运行，这样会引入新的运行时和智能应用控制的问题。**建议先只借数据模型**：作用域维度、带有效期和出处的事实、两步整理。

## 4. 对设计 37 的补充（建议）

- §3 席位账本：采集走协议事件流，协议覆盖不到的地方参考 claude-mem 的钩子做法兜底。
- 新增一节"长期记忆"：
  - 按"作用域 × 席位 × 会话"来划分；
  - 事实采用 Graphiti 式的有效期加出处；
  - 秘书长的全局记忆和各项目的记忆物理上分开存放；
  - 整理由一个空闲时运行的后台任务来做，边界同上。

---

## 5. 补充：专门做会话汇总和转换的一类项目（2026-09-26）

Owner 注意到 Codex 能把 Claude 的上下文拉过去。查下来，这一类项目分两种。

### 5.1 转换：把一家的会话改写成另一家的原生格式，然后用原生续接接着跑

| 项目 | 规模和许可 | 做法 |
|---|---|---|
| **Codex 官方的外部 agent 导入**（openai/codex 的 `codex-rs/external-agent-migration`） | Apache-2.0，Rust | 能读 **Claude Code**（`records_cla.rs`）和 **Cursor**（`records_cur.rs`）的会话记录，导入成 Codex 线程，导入后在 Codex App 或 TUI 里能接着聊。有 `append.rs`（往已有线程追加）和 `ledger.rs`（导入记录）。同一个模块还能导入配置、钩子、MCP、记忆、子 agent 设置 |
| CASR（Dicklesworthstone/cross_agent_session_resumer） | 122★，Rust，许可证是"MIT 加附加限制" | 有一套统一的中间格式，覆盖十几家的读写器 |
| sessport（lanternsmith/sessport） | 2★，MIT，TS | 在 Claude Code、Codex、Gemini 之间搬会话 |
| authsec-bridge | 未核 | 从磁盘读会话，改写成目标 CLI 的格式放进它的会话目录 |

### 5.2 汇总：把各家 CLI 存在本机的会话历史收进一个索引，统一检索

| 项目 | 规模和许可 | 做法 |
|---|---|---|
| **cass**（Dicklesworthstone/coding_agent_session_search） | 1.1k★，Rust，许可证同样是"MIT 加 OpenAI/Anthropic 附加限制" | 11 家以上会话历史的统一索引和检索（TUI 加 CLI），另有社区做的 MCP 封装 |
| **Agent Sessions**（jazzyalex/agent-sessions） | 878★，MIT，Swift（macOS） | 本地优先，浏览、检索、分析、续接各家会话历史 |

### 5.3 对我们的用处

- **跨厂商换实例**：设计 37 的通用做法是用账本文字重放。**转换**做得好的话，能带上工具调用这类原生回合，比文字重放损失更小，可以当作**优化**，前提是行为跟通用机制一致。其中 **Codex 官方的导入器是 Apache-2.0、用 Rust 写的，跟我们的 Rust 宿主同语言**，它解析 Claude Code 会话记录的部分最值得借。CASR 和 cass 的附加限制，跟我们公开的 MIT 仓库不兼容，只借思路。
- **把已有会话收进席位**：汇总类项目的"各家会话记录读取器"，可以用来把 Owner 在 CLI 里自己开的会话，导入成某个席位的账本（Paseo 也有 `importSession`）。
- **席位账本本身就是我们自己的汇总层**：区别在于，它按"作用域 × 席位"来组织，并且受权限管控；这些项目是按厂商、按会话组织的，也不做隔离。
