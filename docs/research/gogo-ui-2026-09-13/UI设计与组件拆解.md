# UI 设计与组件拆解

**产品范围说明：GOGO 面向多种成果生产，软件施工与通用生产两套模式并行。软件模式保留 Git、diff 和测试等既有要求，通用模式采用对应成果的审查依据；共用基础交互，允许同项目协作。完整方向见[改版研究总图](revision-map.html)。**

上一份报告解释了产品怎样工作，这份补齐它们**看起来怎样、控件怎样组成、状态怎样变化**。可点的样本见 [组件图谱](components-atlas.html)。原产品样本按源码中的默认颜色和局部尺寸重建，不是截图；GOGO 组合样本是研究候选，不是已经改好的产品。

设置中的具体内容与作用范围，另见 [设置与功能地图](settings-map.html)。

## 视觉语言

| 产品 | 视觉基底 | 几何与排版 | 主要组件技术 |
|---|---|---|---|
| AionUi | 白 / 冷灰，品牌淡紫 `#7583b2`；默认主按钮另使用 Arco 蓝 | 问题容器 10px、选项组 8px；问题/选项14px，说明12px；发送框20px圆角，发送按钮32px圆形 | Arco Design、UnoCSS、IconPark、自定义聊天和团队组件 |
| Cindy | 暖灰 `#f8f8f6`、白色控件、分隔线 `#d7d7d4`；日常工作区近乎单色 | Inter；容器12px、内控件8px、普通按钮胶囊；按钮32/36px；问题15px、选项14px、说明13px | Radix 基础交互、Tailwind、Lucide、自定义设计 token 与组件 |
| Orca | 白色、近黑文字、灰色弱层，状态色局部出现 | Geist；基础半径10px，派生控件半径；常规按钮36px、小号32px；问题选项14px、说明12px | Radix、Tailwind、Lucide、shadcn 风格自定义组件、终端与分栏 |
| Vibe Kanban | 白/灰开发工具底色，橙色品牌强调 | IBM Plex Sans/Mono；输入和面板偏紧凑、较小圆角；排版尺寸由 rem 比例生成 | 自有 `@vibe/ui`、Radix、Lexical、Phosphor、可调整面板、专用 Diff 组件 |
| Herdr | 真正的终端画面，主题可配置，默认主题名为 Catppuccin | 等宽字符、终端网格、直角 pane 边界；不适用网页 px 圆角体系 | Rust、Ratatui、Crossterm；不是可直接搬进网页的组件库 |

这些是具体默认主题或组件中的值，并非每个页面都使用同一尺寸。Vibe Kanban 的 `text-sm` 由 `0.875rem` 生成；不能采信其代码旁过时的“10px”注释。有效字号还受根字号、缩放和用户主题设置影响。

Cindy 官网的红黑插画**不等于它的工作界面**。其设计规范明确排除工作区里的吉祥物与装饰图，使用近单色表面、细分隔线及有限语义色；因此不能把官网品牌气氛当作产品 UI 模板。

## 最值得拆开的组件

| 组件 | 应显示什么 | 默认收起或省去什么 | 参考 |
|---|---|---|---|
| 项目/任务导航行 | 名称、明确选中态、确实需要的未读提示 | 每行都常驻的一排操作按钮、重复阶段与模型信息 | Cindy / AionUi |
| 主对话消息 | 用户原话与模型的主要结果 | 外层大卡、重复标题、常驻技术按钮 | Cindy / Orca Native Chat |
| 底部输入框 | 输入、附件、收件人、发送/停止 | 为普通发送另造确认表单；工具铺成多排 | AionUi SendBox / Cindy ChatInput |
| 收件人选择器 | 当前人、可选成员、必要的角色说明 | 让用户用多枚常驻 chip 才知道发给谁 | 下拉/Popover 模式；按 GOGO 的实际成员投影 |
| CLI 问题组件 | 原问题、原选项、说明、协议支持的答案输入 | 系统自己加的业务题、推荐与额外确认 | 四家的专用问题组件 |
| 权限请求组件 | 操作、范围、原始可选决定 | 和普通偏好问题共用含糊的“确认一下” | AionUi / Cindy；来源规则仍须分别处理 |
| 就地状态/错误 | 正在发送、已排队、失败原因及可执行下一步 | 成功大弹窗；原始堆栈直接铺满页面 | 同一位置的反馈，详情再展开 |
| 成果入口 | 可理解的名称、类型、直接打开 | 把文件路径和哈希当作主要标题 | AionUi Preview / Vibe Kanban |
| 成果查看器 | 实际页面、文档、图片或修改 | 先要求用户会读代码 | Orca Design Mode / Preview |
| 右侧查看面板 | 当前查看对象、收起、固定/跟随 | 默默改变主输入收件人 | 可拉宽面板与对象绑定 |
| 工作卡 / 画布节点 | 标题、短摘要、必要的状态与关系 | 和消息卡、审批卡、文件卡全部长得一样 | 视觉可统一，语义不能混用 |
| 基础控件 | Button、IconButton、Menu、Popover、Tab、Badge、Tooltip、Separator | 每个业务区重造一套尺寸、圆角、焦点和禁用态 | 一套基础规则覆盖全部业务组件 |

## 同一个问题，四种具体样式

**AionUi：** 问题放在有细边框的容器里，选项是整行单选/多选控件，说明在标签下面；整批准备好后提交。外壳和权限请求属于同一视觉家族，但回答通道分开。

**Cindy：** 问题临时占据底部输入区域。标题、选项、说明有清晰字号层次；单选推进，多选再确认；能收成44px条，恢复时保留答案。答完在历史里留短问答记录。

**Orca：** 选项左侧有编号/选中标记，正文保持中性。选中只高亮，再按 Submit/Next，避免控件突然消失；与输入区共用宽度，而不是另开一张全屏表单。

**Vibe Kanban：** 问题更接近输入框上方的一条专用区域，细分隔、紧凑布局；单选推进，多选或自定义输入再确认。它另外有带品牌/错误着色的计划消息容器，这种着色不适合泛化到所有消息。

图谱用同一条**静态示例问题**比较外观，只改变本地演示状态，没有模型请求、权限操作或项目写入。字体使用本机回退，非像素级复刻。

## 一套视觉规则，比换组件库重要

对 GOGO，值得验证的候选是：平静的主对话、足够大的正文、细边界、少量有含义的颜色、底部稳定输入和可调整的查看区域。

- 普通消息不套大框；成果、待回答问题、浮层才有明确容器。
- 主操作、次操作、图标操作各有一种形状和状态规则；不在不同页面反复变种。
- 强颜色表达真实状态或焦点，不把整个任务、整列或每条确认都染色。
- 正文、选项说明与元信息要分级，但“次要”不等于小到难读、淡到看不见。
- 菜单、选项和按钮的 hover/pressed/focus 不改变外部尺寸，避免点击目标移动。
- 正在提交时保留按钮宽度、阻止重复点击；失败保留草稿；成功用就地反馈。
- 桌面紧凑控件与触屏命中区分开设计。不能把某家的24px/32px外观当成所有设备的通用点击尺寸。

图谱中的 GOGO 组合候选使用15px主要阅读文字、12–13px辅助文字、8/12px容器层级与少量胶囊控件，仅用于观察整体一致性。它不规定项目流程，也没有把新的 UI 组件接进 GOGO。

## 组件库与当前项目

Arco、Radix 或 shadcn 风格组件提供的是基础行为和控件，不会自动解决层级、信息密度、文案、对象绑定或重复确认。当前 GOGO 仍可在现有 HTML/CSS/JS 中统一这些规则；本研究不要求为了外观重写框架。

如果后续实施，优先把“输入框、收件人菜单、CLI 问题、就地反馈、成果入口、侧栏”做成一致的一组，再处理卡片与画布。先统一组件语言，才能避免每次加功能就长出一种新的确认框。

## 源码定位

- AionUi：[默认颜色](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/styles/themes/default-color-scheme.css)、[问题样式](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/conversation/Messages/MessageQuestion.module.css)、[权限外壳](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/conversation/Messages/components/MessagePermission/PermissionRequestPanel.module.css)、[输入框](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/components/chat/SendBox/index.tsx)、[发送按钮](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/components/chat/SendBox/sendbox.css)、[依赖](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/package.json)。
- Cindy：[视觉规范](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/docs/design-rules/DESIGN.md)、[按钮](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/renderer/components/ui/button.tsx)、[问题容器](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/renderer/components/interaction-portal/InteractionPromptCardShell.tsx)、[问题行](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/renderer/components/new-chat/AskUserQuestionPrompt.tsx)、[依赖](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/package.json)。
- Orca：[颜色与字体](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/assets/main.css)、[按钮](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/ui/button.tsx)、[问题组件](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/native-chat/NativeChatQuestionCard.tsx)、[看板卡](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/dashboard-popout/AgentKanbanCard.tsx)、[依赖](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/package.json)。
- Vibe Kanban：[默认主题](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/web-core/src/app/styles/new/index.css)、[尺寸与字体](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/local-web/tailwind.new.config.js)、[输入框](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/ui/src/components/SessionChatBox.tsx)、[消息容器](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/ui/src/components/ChatEntryContainer.tsx)、[组件依赖](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/ui/package.json)。
- Herdr：[终端 UI 依赖](https://github.com/herdrdev/herdr/blob/2746ac7ebf75d55289aa522d99ac66fc40f062c4/Cargo.toml)、[主题配置](https://github.com/herdrdev/herdr/blob/2746ac7ebf75d55289aa522d99ac66fc40f062c4/src/config/theme.rs)。
