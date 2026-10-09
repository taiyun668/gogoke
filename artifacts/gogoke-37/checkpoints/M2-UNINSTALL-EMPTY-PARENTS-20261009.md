# M2 卸载空父目录轻检查点

## 改动

finalizer 只对已核验 owned files 的 root 内中间父目录首次以 DELETE 权限 pin，并由深到浅复用原句柄删除。目录返回 Win32 145 时保留并写入 DELETED 回执；其他目录删除错误继续失败。新增云端实物控制，覆盖 owned nested directories 删除和未知内容保留。文件/快捷方式删除显式丢弃 helper 返回值，避免污染 READY stdout；文件删除错误保留 Win32 原码。

## 结果

f3484045 的 desktop web_only run 37901846688 成功；finalizer 云控为 5 tests/OK。该 run 未覆盖大 owned-file 清单的 stdout 背压与文件删除失败原码，因此本次最小修仍需受影响云测。本机不做原生编译、安装或产品数据操作。

## 下一步

由 Controller 在该提交上运行既有 desktop web_only 受影响云测，并把结果绑定到实际嵌入 finalizer 的字节；云测前不宣称修复验收。

## 参照

项目 AGENTS.md、docs/model-routing.md、docs/governance/gogoke-build-and-release.md、现有 update-coordinator 的空目录处理、M2/M3 重估和 PR77 目录根因记录。
