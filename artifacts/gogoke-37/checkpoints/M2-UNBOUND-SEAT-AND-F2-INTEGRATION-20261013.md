# 未绑定席位与 F2 历史来源修复

## 改了什么

Host choices 原失败来自一个既有合法的未绑定 USER 席位。原生读取现在返回 `instance:null` 并保留席位、七态和原 actions；已有非空绑定缺失仍拒绝，可选实例仍要求启用及原始当前登录观测。席位页如实显示未绑定，侧聊排除该目标但不把已有聊天误判删除。调用层只保留原 permission/effort tune，未绑定席位的实例和模型变更在 mutation 前拒绝。

F2 集成历史 Secretary 原 selection/V2、generation-change、StopFact 与原物理根的分离校验。历史原记录不再被要求等于当前 E generation，也不再要求已经撤销的历史 ACE。当前 reservation 仍要求当前 E、有效 grant 和 ACE；UNKNOWN 不变成 STOPPED，不加入目标 F 的停止证明列表。缺根、替换和重解析点仍拒绝。

## 结果与证据边界

F2 独立分支 `50bf5a57073094e035eb3c7ca06866c53fbb61b6` 的受影响云端运行 [38347496812](https://github.com/taiyun668/gogoke/actions/runs/38347496812) 为 21 passed、0 failed、0 ignored。已完成 fresh 聚焦复核及 fresh R1–R7 全轴源码审计，未发现本次新增阻断。Secretary fixture 部分使用合成 custody/StopFact，不能代替现场真实生命周期。

未绑定修复的独立聚焦复核覆盖实际 create/bind producer、原生投影、调用器和两个直接 UI 消费者。签名 Node 的两文件共 23 项测试、TypeScript 类型检查通过。原生回归通过真实 `configure_user_v37` 读取创建后绑定前及绑定后的席位；它已接入 `seat-scope`，组合候选另用 `f2-seat-page` 同时运行两处受影响测试。完整集成源仍须取得自己的云端结果，旧分支通过不自动继承为候选通过。

## 下一步与参照

准备 0.1.66 实验候选，将 F2 与未绑定修复合在一次换装中验证。下一真实启动先读 Host choices，再以新 H 和一次新模型轮次尝试原 child 的 V11 合并；原失败请求不重发。沿用现有 native episode、SESSION 历史根、实例观测和正常停止关闭路径，减少重复构建与启动，不新建授权或数据写入口。

0.1.65 仍是已安装版本，两处失败均已正常关闭并完成十三项保护。工人独占工作树的依赖和临时目录因工具策略拒绝清理而保留，供本轮复核和续接使用；未尝试绕过删除限制。主仓原有未跟踪材料未动。

组合入口首次云检保留原失败：新 selector 未被加入既有 Git backend 准备条件，F2 的 18 例在缺少实际后端 pin 时拒绝，另 3 例通过；新增原生席位回归单例通过。已将组合入口接入原准备分支，测试内容和断言未变。旧结果不作组合通过，更新后的精确源重新运行受影响检查和轻构建。

PR #91 仍 OPEN。完整 M2/M3、三条正式命令、全量原生与 production mutation、稳定更新链、SAC 开启实测、外部 Claude 和 Owner 验收未执行或未完成。本记录不表示候选接受、合并 PR 或公开发布。
