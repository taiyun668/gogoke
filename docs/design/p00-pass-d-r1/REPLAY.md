# Pass D 原件接收与重放

输入：固定源码ZIP、R2与R3交付ZIP、R3独立复核ZIP。校验各自外层SHA和内部清单。完整D交付包 tools/ 下保留本轮实际脚本；没有自动联网clone或安装产品依赖。

独立R3包的deliverables/保留independent_r3_ast_review.mjs、independent_negative_mutations.py、independent_preservation_check.py等原脚本。SOURCE为基线checkout；CANDIDATE为R3有效证据投影，登记控制文件另验tree。

```sh
node CANDIDATE/docs/design/p00-pass-c-c2-f01-r3/verify_symbols.mjs SOURCE CANDIDATE SYMBOLS.json
node CANDIDATE/docs/design/p00-pass-c-c2-f01-r3/targeted_fixtures.mjs SOURCE FIXTURES.json
node REVIEW/independent_r3_ast_review.mjs SOURCE CANDIDATE AST.json
python REVIEW/independent_negative_mutations.py SOURCE CANDIDATE REVIEW/independent_r3_ast_review.mjs NEGATIVES.json
python REVIEW/independent_preservation_check.py R2_EVIDENCE CANDIDATE R3_REGISTRATION R2_REVIEW_DELIVERABLES R3_DELIVERY_ROOT PRESERVATION.json
```

前两项各运行两次。使用预安装Node22/TypeScript5.8.3与Python标准库；不修改输入。两次输出需与固定发布字节一致。preservation比较包含的继承文件数可因完整证据投影增加，但声明的不变文件、四个变化路径和250项断言不能改变。
本轮属于Controller replay，脚本中的原作者/独立角色标签原样保留；标签不能让本窗口成为独立审计者。真实产品runtime保持false。阴性案例的内部exit2是预期检错，不计产品测试失败，也不冒称真实故障注入。
