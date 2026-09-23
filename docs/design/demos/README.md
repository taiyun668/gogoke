# 演示文件

这些是**我们自己做的**，Owner 逐版看过。`kit/index.json` 的 `chosen` 指着它们——不要删，也不要在别处重做一遍。

| 文件 | 是什么 |
|---|---|
| `gogo-work-tree.html` | **派活的工作树**，第八版。整页可直接在浏览器打开，点「重播」看一次完整派活 |
| `wt-head.html` / `wt-script.js` | 上面那页的样式与脚本源（整页由这两个拼成） |
| `gogo-kit.html` | **组件库展示面**的整页源（发布版在 https://claude.ai/artifact/FSkMBLHwHPxHrDJU9yiqWy）。读的是 `kit/index.json` 那套槽 |
| `slot1.html` / `slot1.css` / `slot1.js` | **槽 `user-input-card` 的全保真候选**，尺寸照各自源码取值（Beautiful UI RecommendationCard 380px/14px/500、min-h 48px、按钮 27px；gogoke RequestUserInputMessage；AionUi MessageQuestion） |

## 工作树这一版定死了什么

- 「正在进行」**永远在主控输出的最下方**，主控之后说的话堆在它上面
- **发散**：一条 `@` 派给三个席位就分出三条线
- **收敛**：一条线收口之后，行上显示**它交回的第一句结论，不是状态词**
- **归一**：三条线收回主控，折成一行；那一行右边写状态字加一句事实
- 席位有问题先交主控；主控答得了就只留一句说明，答不了才出「要你定」的卡，挂主控名下
- 没收到回执就写 `待核对`，不计入任何一边

动效用 gogoke 的 `ds-tokens`，结构参考 Aster 的 `AgentGroup`（第三方源码没有搬进仓库，取值记在 `kit/index.json` 的候选里）。

2026-09-16 起：折起来那一行的状态字从 `可验收` 改成 `改动就绪`，不再有验收动作。演示文件本身没改，读的时候照这条换。
