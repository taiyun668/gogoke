# O：开放运行端与模型目录；Pi不是特例

## O1 采用的扩展点

复用T3开放ProviderDriverKind及ProviderInstanceId；合法未知driver保存并呈unavailable，不能fallback Codex。新增driver通常只改adapter目录、组装注册/锁依赖、局部配置UI与自有测试，不改公共Session/Decision/Context表或业务品牌switch。新增全新语义才需合同major增量，不承诺无限无条件兼容。

S1实现可信内置SPI与声明式配置。动态下载插件商店、任意同进程用户脚本、进程外adapter热装载不进入本阶段；留版本化接缝，不先造通用RPC宇宙。可以重编译注册新适配器，这不等于架构重做。

## O2 分离对象

RuntimeDriver是Antigravity/Pi/Codex等harness；RuntimeInstance是已配置binary/profile/host；ModelRef是某实例观察到的模型；Gemini属于ModelRef。Role定义职责，Seat保存身份，ExecutionRecipe组合实例/模型/推理profile/工具/隔离/预算/ContextManifest。不能把Gemini写入driver注册表，也不能把一个Google账号等同一个席位。

一个账号可供多个合格实例/席位共享已核容量，不能按邮箱合并身份，不自动轮转多个账号规避额度。用户愿意划出的Antigravity账号仅在显式登记和后续授权后绑定context-steward；当前仅建禁用配方，真实账号数/可用量未知。

## O3 AdapterManifest v1

必须有packageId、开放driverId、adapterVersion、artifactDigest、hostApiRange、nativeProtocol、platforms、configSchemaRef、declaredCapabilities、requiredHostServices、requestedEffects、source/license、admissionRef。声明不等于授权。未知必需host API拒绝；未知可选capability保留并标unknown。UI从schema/标签生成，缺图标用通用图标；不引入执行脚本。

创建实例独立scope和可变状态；更换版本不得热换旧binding的parser/身份。旧实例drain后再切；opaque history保留。native resume与材料handoff区分，跨harness不伪装resume；nativeFork继承暴露历史，NEW_CLEAN不复制隐私历史。

## O4 Capability三层

每个capability有declared、observed、qualified证据；值supported/unsupported/unknown附限制。资格键至少包含driver+adapter/native摘要+platform+profile/authRevision+model/运行模式+isolation/toolProfile+generation。模型未报告具体版本则记录unknown，不以别名假称固定。

分面prompt/steer/followUp/interrupt/close/resume/fork/compaction/image/tools/approval/question/usage/eventReplay/sandbox/descendants。运行配方按需求求交集，不要求每家所有分面为true，不用自然语言prompt伪装批准。探针也可能启动hook/登录窗口，S1只能fake并捕获env/cwd/子进程；注册表存在不是账号ready。

## O5 Pi stdio RPC决定

固定协议基线earendil-works/pi@d1230ea2000d876b479a69b8b061f9d670f262f5，采用受管pi --mode rpc。不要依赖实验性server/client模块，不先包ACP，不把Pi整个SDK装入主权威进程。

严格LF分帧，CRLF去尾CR，U+2028/U+2029不分帧；partial/oversize/重复ID/无效JSON留可诊断错误。prompt success是accepted/queued/handled，不是完成；streamingBehavior和steer/follow_up交付时机保持。new_session cancelled=true不发布新绑定。pause先封宿主新派发，再按协议clear_queue与abort并串行化竞争；abort后仍可能处理队列，不能当全停。native resumed状态须读回ID/lineage，与Manifest/域核对。

Pi project trust不是sandbox。AGENTS/原生history/插件可能引入额外上下文；若未能观测或限制，exposure记POSSIBLE/UNKNOWN。保护域/盲审要求未达成则拒绝，不降full-access。真实Pi原生工具/账号/图片/复杂续接在GN逐项准入。

## O6 可扩展性验收

先跑原5通路fake，再Pi，再随机合法ID mock_novel_<seed>。随机ID必须在核心构建后才选择，tests不得把该ID硬编码进业务核心。配置保存→卸载→重启不可用→重装识别不丢历史。新driver不要求schema迁移，unknown major拒绝、可选缺失可解释。这个对照证明开放，不以首批数量充当证明。
