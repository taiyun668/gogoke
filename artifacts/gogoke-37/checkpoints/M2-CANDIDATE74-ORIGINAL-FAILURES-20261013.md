# 0.1.65 原始失败与正常收尾

## 改了什么

Owner 已要求继续施工。0.1.65 在普通用户视图完成安装，实际核心文件与受信资源签名候选一致；安装前后十三项保护及原数据、H/C 和物理身份连续性核对通过。旧 0.1.64 正常退装，仅保留原空安装根，未手工删除。

此前在 0.1.64 上完成 V06 不同实例 BUSY 换实例拒绝：原 USER 请求为 CONFLICT，原绑定、settings 和 F 保持，真实停止释放后回到 IDLE，正常关闭及十三项保护通过。这是已报告的技术切片，不代表完整 V06。

## 结果

0.1.65 的 V11 使用新 H、原 child F 和一个原模型合并轮次。原回复为 DENIED，F 图仍是 REGISTERED；没有重发 merge。随后新 H 真实停止并释放，秘书长第 43 代仅停止，窗口和 native host 正常退出均为 0，运行前后十三项保护相等。

闭库测量核实九个秘书长 session 的原 APPLIED open 和不可变 selection 均存在。八个历史 selection 的授权代不同，不能同时等于一个当前 E generation；当前实现却逐个这样比较。四条历史 UNKNOWN 又属于未 promote 的恢复候选，原本没有正式 generation 行。UNKNOWN 保留原状态，不能因 H 已释放或凭据已撤权就当作停止。

独立 Host choices 实测只发送一次 USER seats-page-read，返回原生错误，未取得页面。秘书长第 44 代真实停止且不释放，窗口和 native host 正常退出均为 0，十三项保护通过。已验证的 MANAGED/READY/LOGGED_IN 实例事实保持；页面显示引用失败不能覆盖它，也不能被记为 choices 为空。

## 下一步

F 独立分支已提交历史来源、停止事实和实际授予根分开的窄修复，受影响云检与 fresh 风险审查在途，尚未集成或换装。Host choices 继续按原生 producer 的注册、tombstone 和席位绑定关系做闭库定位，再确定最小修法。

PR #91 尚未合并，相关新增共享桥接仍未施工。完整 M2/M3、稳定更新链、智能应用控制开启实测和 Owner 验收未通过；未公开发布。

## 参照与过程修正

沿用原 H episode/custody、不可变 native selection/V2、SESSION/private-history 根、ActualProduct、原句柄关闭和三个现成观察器。两项真实产品失败分别保全、正常收尾后再定位，不重跑旧请求。私有仪器的 Git 提交号位宽和 PowerShell 进程筛选错误已按直接原文修正，旧失败保留；后续关闭错误只允许一次自动收尾，避免重复 stop。

## 失败原文

- V11：`merge.cleanup_stop_gate -> cleanup.secretary_native_selection -> Denied`
- Host choices：`Invalid("seat instance presentation unavailable")`
