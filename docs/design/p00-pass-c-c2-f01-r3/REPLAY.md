
## 4B. R3 C2-F01 — 逐符号补充检查

以登记 snapshot 指向的新 payload 为候选根。保留原 R2 checker/49场景及历史结果，不用其通过代替逐符号验证。
```sh
node docs/design/p00-pass-c-c2-f01-r3/verify_symbols.mjs SOURCE CANDIDATE OUT_SYMBOLS.json
node docs/design/p00-pass-c-c2-f01-r3/targeted_fixtures.mjs SOURCE OUT_FIXTURES.json
```
Node22、已安装 TypeScript5.8.3；可用 P00_TYPESCRIPT_PATH 指定本地 typescript.js，无自动安装。先核 symbol_tools.mjs 及源身份，再执行。
第一项只解析 AST：14条选择、34个具体声明 token、9个显式行为标签、15条调用边；正确基线0错误，14项缺符号/错类/错路径/错行/blob/行为伪装/删调用边负例必须拒绝，40 import与37 writer计数不变不能掩盖符号错误。
第二项31场景执行精确源方法体，依赖由假件替换；不启动provider，不访问真实Home，不进行真实磁盘或进程操作，不是31项产品验收。与旧场景存在重叠，不相加累计产品通过。两次输出须相同。
R3对象索引更新为新字节；全局index与依赖它的验证结果不互相哈希，最终由Git tree固定。已关闭项用字节/语义字段保全检查，不要求重新盲审。
