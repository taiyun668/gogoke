# M2 卸载空父目录轻检查点

## 改动

finalizer 只对已核验 owned files 的 root 内中间父目录首次以 DELETE 权限 pin，并按深度从大到小复用原句柄删除。目录返回 Win32 145 时保留并写入 DELETED 回执；其他目录删除错误继续失败。新增云端实物控制，覆盖 owned nested directories 删除和未知内容保留。

## 结果

源码与云端行为控制已改，尚未执行云端行为检查；本机不做原生编译、安装或产品数据操作。

## 下一步

由 Controller 在该提交上运行既有 desktop web_only 受影响云测，并把结果绑定到实际嵌入 finalizer 的字节；云测前不宣称修复验收。

## 参照

项目 AGENTS.md、docs/model-routing.md、docs/governance/gogoke-build-and-release.md、现有 update-coordinator 的空目录处理、M2/M3 重估和 PR77 目录根因记录。
