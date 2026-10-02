# G.0 实例页浏览器预览

在 `apps/desktop` 用本机签名 Node 跑 `npm run dev -- --host 127.0.0.1`，打开本机 Vite 地址的 `/src/features/seats/preview/index.html`。此入口只在 DEV 浏览器运行，遇到真实 Tauri 桥会拒绝；默认正式构建入口没有导入它。

复用真实 `Design37InstanceSection`、样式和实例页解析器，桥后复用已提交的 `V37UiForwardingFakePort`。支持四种状态、未登记、开始/成功/失败/取消、关页再打开；只有预览控件选择结果，不访问任何授权网址或真实数据。整个浏览器重载会重置假宿主，不能把假宿主结果当作真实登录或恢复证据。

参照：已读 gogo-party 的浏览器工作台与账号宿主管理、NaveHQ mock display 文档的展示与事实边界；LoomOS 当前为规格及任务资料，没有找到 Vite/Tauri 预览代码。核过仓库 reuse-blueprint、upstream-reference-map、parts/substrate 拆解及 execution-layer 表；采用本仓库现有 K-UI 转发假实现和真实组件测试的桥边界，未另引入上游运行时。与真实宿主不同之处仅是明确的浏览器假数据与手动结果场景，不启动 CLI，不改变登录或 LPAC 权限。
