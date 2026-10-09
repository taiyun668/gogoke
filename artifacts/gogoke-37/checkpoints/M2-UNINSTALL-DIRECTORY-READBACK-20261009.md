# M2 卸载 owned 父目录只读工具轻检查点

## 改动

新增只读 reader：从原 `gogoke.update-owned-inventory.v1` 或 candidate finalizer owned payload 推导候选 root 内父目录，读取实际 DirectoryID、直接子项、Win32 缺失码和 145 回执对应关系；未知内容只保留记录，root 不计入 owned 父目录失败。可选 resource-index 只做版本/source/generation 绑定。

## 结果

reader、说明和私有输出格式已完成；未运行本机产品、原生编译、卸载或 Owner 数据检查。尚无实际候选回读结果或验收结论。

## 下一步

Root 在下一次稳定候选的实际候选卸载后，用原 ownership inventory、instance、nonce 运行 reader，并审阅私有 JSON；不得用该工具替代安装/卸载行为验收，也不得清理它发现的未知内容。

## 参照

现有 `gogoke.update-owned-inventory.v1` 生成路径、`m2-readback.py`/`m2-provider-capture-readback.py` 的只读输出与 Win32 identity 方式、R2-06a G4 empty-directory inventory/audit、现有 finalizer 的 145 receipt 语义。
