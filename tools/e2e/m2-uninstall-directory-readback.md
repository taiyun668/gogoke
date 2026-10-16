# M2 卸载 owned 父目录只读回读

该 reader 只服务于一个明确的候选安装根。它读取卸载前保存的原 ownership 原件：正式更新链的 `gogoke.update-owned-inventory.v1`，或 candidate 卸载 handoff 的原始 owned payload（必须有 root、rootIdentity、instance、files，并绑定 candidate domain/registry/nonce），从其中的 owned file 路径推导 root 内父目录；可选读取卸载前私有副本 `resource-index.json`，只绑定版本、源提交和 generation。

卸载完成后运行：

```text
python tools/e2e/m2-uninstall-directory-readback.py --root <candidate-root> --instance <original-instance> --nonce <receipt-nonce> --owned-index <private-owned-inventory.json> --output <private-readback.json> [--resource-index <private-resource-index.json>]
```

reader 只打开输入原件、由 instance/nonce 推导的原始卸载回执、候选 root 及 owned file 的 root 内父目录。每个目录使用 Win32 `FileIdInfo` 和 reparse 检查；只读取该目录的直接子项，不递归扫描。缺失父目录记录原始 Win32 2/3；仍存在且有子项的 owned 父目录按卸载语义记录预期 `ERROR_DIR_NOT_EMPTY` 145，并要求原回执包含相同 145 保留计数。未知子项只记录并保留，root 本身始终作为保留对象单独核对，不计入 owned 父目录失败。

输出 JSON 必须写入候选 root 之外的新私有文件；公共 stdout/stderr 只输出状态码，不输出用户名或路径。reader 不删除、移动、修改 ACL、写产品数据或触碰正式安装/正式数据。`resource-index.json` 单独不能覆盖完整 owned 文件集合；缺少原 ownership inventory 时应拒绝回读。
