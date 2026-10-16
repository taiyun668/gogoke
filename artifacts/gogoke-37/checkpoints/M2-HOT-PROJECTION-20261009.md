# M2 热态历史投影

- 改动：两份共享 thread hook 改为在途读取记录原事件来源；完整历史只覆盖同一绑定、原来源五字段及实际 high-water 内的变化。关闭或 notLoaded 直接使在途读取失效；嵌套事件不借用其他事件的来源。
- 结果：工人类型检查和 84 项相关测试通过；独立聚焦复核已关闭缓存失效遗漏，未见新静态阻塞，已集成。真实热态仍未执行，不把源码或既有测试算产品通过。
- 直接证据：已安装 0.1.45 的冷启动显示来源明确的中断内容；同一原热态页面报 `Newer conversation facts superseded this native history snapshot; full reconciliation remains incomplete.`，原 H stop、释放与正常退出已完成。
- 下一步：受影响浏览器云检、轻候选链激活后用真实安装字节复现热态；V06 真机入口与 V10/V11 工具接线并行。
- 参照：固定 Codex 原事件、宿主 nativeAssociation/nativeSourceRef 与 COMPLETE sourceRefs；既有 thread 缓存和读取顺序保护。保留原保护，用来源事实区分被完整读取覆盖的事件与未覆盖变化，不增加重试或等待补偿。
- 保全：工人依赖目录链接被自动审批拒绝清理，保留在隔离工作树，不绕过、不提交。正式安装与数据未改，范围摘要不变。
