# V06 实际 H 并发额度切片

## 改了什么

复用已安装 0.1.64、原 MANAGED Codex 测试实例和现有 USER E/H 产品入口，执行新唯一请求的容量切片。没有登录、模型提问或工具写入，也没有重发原 V11 合并请求。

## 结果

实际实例上限为 4，原常驻秘书长占一项。三个新 H 准入保留成功，第四个原请求返回：

```text
native admission: CapacityExceeded { project_active: 3, project_cap: 4, instance_active: 4, instance_cap: 4 }
```

第四项没有 H claim。三个未启动 H 经原产品释放为 RELEASED，没有编造停止事实；四个新 USER 席位均回读为 IDLE。秘书长本次 generation 41 经真实能力读回、stop、STOPPED 读回，保留常驻逻辑 H。产品窗口和 native host 正常退出，退出码均为 0。

前后 13 项正式保护相等。闭库原生只读验证保留历史 H、custody、别名、凭据元数据及源物理身份；秘书长恢复链的新 generation、原前驱 binding、已撤权 profile 和当前 DORMANT 别名互相对应。原任务精确清理并回读不存在；输出与失败日志保留在私有 D 盘。

这是 V06 容量切片的实际结果，完整 V06、M2/M3 和稳定点更新链尚未通过，不自行验收。

## 下一步

并行继续不同实例的 BUSY 换实例拒绝；F2 秘书长目录不重叠修复经受影响云检和聚焦复核后集成，在新候选上验证 V11 实际合并。生命周期新增桥接路径仍待授权 PR 合并读回。

## 参照

沿用 ActualProduct、RemainingHostRoot、原生元数据读取和已存在的 13 项观察器；测试输入直接绑定当前闭库 cap、原 MANAGED 来源和唯一新 ID。闭库读回补齐生产 private_history 恢复链已有的前驱及已撤权 profile 事实，没有新造调度、停止事实或凭据读取机制。
