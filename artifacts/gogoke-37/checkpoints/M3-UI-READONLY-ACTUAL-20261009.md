# M3 只读界面实测片段

改了什么：集成只读 UI E2E，先以原 USER 返回确认秘书长 UNSET/NONE，再观察真实入口、实例页与当前项目面板；不点击登录或写配置，不切项目，不调用模型。失败保全工具补存其持有的原 child exitCode/signal/error，供后续失败回读；旧原件不能补造退出码。

结果：已装旧候选原冻结字节上的秘书长原生 UNSET 与入口片段通过。实例页持续“正在读取实例”、零行，等待选中实例行失败；其余面板未执行。原窗口用 caption 请求关闭，进程消失，无模型会话、无强杀；原退出码未知。闭库正式版、实例记忆及账本观察者的原字段比对通过。完整 V15/M3 未通过，也不替代稳定候选复测。

根因证据：独立只读核对发现，实例页每秒新读且每轮使旧结果失效；instances→management 两条读取共用串行原生 User 通道，可产生积压与持续 loading。原件尚不能证明单次 native 请求永久阻塞。问题与最小修订已交 Claude 的 G 分支；Root 未改 G。

下一步：Claude 采用现有秘书长 pending 读取模式后再实测。无重叠仍卡才查具体未返回边界，不延长超时掩盖。Root 同时跑 V08 真实场景；独占工人补 busy-to-idle 原事实链，黄金样本线确认三家仍缺合格闭库模型协议，不用登录成功替代。

参照：现有 MainApp 秘书长 pending Promise guard、InstancesPage 的 readSeq、design37ManagedInstances 的两次读取、product_entry 的 PRODUCT_RUNTIME_GATE、design37_host 的 User pipe、native authority 单线程处理，以及既有保护观察者。沿原 CDP 硬定位与宿主事实，无 agent.act 或新增 Owner 操作。
