# OpenCode 盘根问题：两路方案，待 Owner 裁决

本次只读核对，无新增映射、ACL 修改或提权；没有把方案接入产品。固定 CLI、原 LPAC/Job 与已有 capability 保持原边界。前期别名施工分支已保全，停止继续实施，等待本次裁决；其他 M2 线继续。

## 直接事实与前提核对

原候选的盘根 `lstat EPERM` 与开发对照是不同证据。此前不提权开发试验使用系统 `DefineDosDeviceW`，不是 `subst.exe`：将已授权的独立 F 目录映射为局部盘符，canonicalize 后 FileID 与原 F 一致；相同固定 OpenCode 在真实 LPAC/Job 中取得 `initialize` ACK，原 Job 清零并精确删除映射。另一独立 F 的 canonical 盘路径对照返回 `EPERM ... lstat`。没有登录、模型调用、`session/new`、候选产品 H 或 SAC 强制模式通过结论。

现有证据支持“局部 DOS 映射可在一次 LPAC 启动中起作用”，不能推出“当前产品完整流程已通”或“已经实测了 subst.exe”。原试验不补造新试验。这次只读核对沿用原件，不新建映射。

当前系统卷根与开发数据卷根均没有 `S-1-15-2-2` 的 ACE；分别有六、八条原 ACE。安全描述符读前读后相同。普通进程对系统卷根的原 `WRITE_DAC` 检查为 Windows 5，开发数据卷根的新检查也为 Windows 5；两者分别记录，不能类比替代。没有权限据此在普通宿主中自动加或自动撤根 ACE。

## A：不提权的局部盘符映射

采用 `subst` 类路径映射，让 OpenCode 进程 cwd 和 ACP cwd 使用映射根；宿主授权依据仍是原 F 的 canonical 路径、卷号与 FileID。映射目录为 gogoke 已授权的工作树，不映射整个卷，不更改现有 ACL。微软说明局部 DOS 名称按登录 AuthenticationID 分开，不能只凭“同一用户名”推断 LPAC 一定看得到。[subst](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/subst)；[局部命名空间](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/local-and-global-ms-dos-device-names)。

待准许继续后的证伪点：用系统原 subst.exe，在与宿主相同的登录上下文建立临时映射；真实 LPAC 内查询映射、读取映射根属性并核对原 F FileID；保持固定 CLI，完成产品 H 的 initialize/session-new、工具路径、停止和同实例恢复。看不到映射、映射落到别的对象、仍需物理卷根权限，任一出现就保留原文并拒绝这一方案，不能凭 ACK 跳过其余轴。映射本身是名字，不另授目录权限。

代价是宿主需要记载映射意图、进程代次及精确目标；碰撞拒绝，正常停止后精确删除，崩溃时必须证明旧持有者与原映射身份，未知则保全。不能以宿主 PID 已消失直接推断子进程已消失。已有 H 草稿尚未接 shared runtime，最新修正没有原生编译通过或完整生命周期测试证明，不纳入候选。系统 API 的精确删除有现成定义：[DefineDosDeviceW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-definedosdevicew)。

## B：一次性受限提权设置固定组的根 ACE

仅为 Owner 指定的一个真实卷根对象设置显式 ALLOW；不是自动给所有卷授权，不继承到子目录或文件。主体固定为 `S-1-15-2-2`（ALL RESTRICTED APPLICATION PACKAGES），不是实例 SID，不随代次更换。微软文档明确该组名称与 SID 的对应：[WebView2 部署](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)。

供裁决的精确 ACE 为 `(A;;0x00000081;;;S-1-15-2-2)`：`0x80 FILE_READ_ATTRIBUTES` 加 `0x01 FILE_LIST_DIRECTORY`，AceFlags=0，InheritanceFlags=None，PropagationFlags=None。若只需根属性，最小子方案是 `(A;;0x00000080;;;S-1-15-2-2)`。不能用 `R`、`RX`、generic read、OI/CI 或递归设置替代这两个具体位。[访问位定义](https://learn.microsoft.com/en-us/windows/win32/fileio/file-access-rights-constants)；[ACE 语法](https://learn.microsoft.com/en-us/windows/win32/secauthz/ace-strings)。

威胁模型：需要运行于受限应用包/LPAC 中且通过其余令牌和 ACL 检查的主体；新增边界是卷根属性读取，选 0x81 还会暴露根目录条目名称及列举返回的元数据。固定组影响相符的其他受限应用包和用户上下文，不能限制为 gogoke 或 OpenCode。该 ACE 本身不授子项内容、写入、删除、改 ACL 或提权，但其他已有 ACE 仍共同影响有效访问，不能说全系统权限不变。[LPAC 访问模型](https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox.md)。

获批后才可准备执行：短时管理员辅助动作仅接受已钉住的根对象与上述 SID/位；先保存根卷号/FileID、原安全描述符及 hash，重读仍一致才增加唯一 ACE，再读回确认差异仅为所选根 ACE。宿主和固定 CLI 始终不提权；不建常驻特权服务、SYSTEM 任务或宽泛授权。之后才实测原 LPAC 对根 lstat 和完整 H 流程。仅读 DACL 不能证明 0x80 或 0x81 足够；若真实调用需要额外访问位，保留原错误，不能自行扩大。

撤销同样需要被准许的短时提权动作或 Owner 管理员操作；“一次性设置”不等于普通宿主以后可自行撤销。精确核对原根身份及唯一新增 ACE，只删除本次引入的那个；保留其他 ACE、Owner、继承和保护位。已有同 SID ACE、合并/重复无法区分或并发变更时，拒绝盲删、拒绝恢复整个旧 DACL。原来已存在的匹配权限不能记为本次新增，也不能撤销。删除一个实例不能撤整个组的共享权限；应由明确的产品级撤销动作处理，不能承诺普通实例删除会自动撤销。

## 建议与剩余工作

建议优先继续 A 的系统 subst 与产品本体验证：已有不改 ACL 的开发启动对照，且不扩大固定组的卷根可见面。B 接线较短、可跨实例代次，但影响更广且加/撤都需要特权动作。两路都尚不能报产品通过，交 Owner 选择，不自动实施。最少往返是先确定一条路，再集中核对根 lstat、完整 H、真实停止和恢复，不两路同时改产品。

其他 M2：秘书长实际保存修法的实验构建、原数据连续换装准备，以及 Host 原合法目标复用继续；不等待 OpenCode 决定停整个施工。

## 参照

查阅 gogo-party Room 现有账号/运行时代码、仓库 reuse-blueprint/upstream-reference-map 等调研和当前 H/LPAC 接线；未找到可直接照搬的卷根映射生命周期实现。采用微软现成 subst/DOS 命名空间/精确删除与访问位定义；LPAC 组影响面参照微软 WebView2 与 Chromium 主文档。保留现有开发试验的 ACK、canonical EPERM 和清理原文；不将路径命名改变当新权限，也不把 SDK 文档当当前安装已通过。
