# 独立复核 R2：Route-B `6fc8c634` / native `37def260` / capabilities `fccffc8d`

- 复核：Claude（Opus 5），2026-09-20
- 角色：最终异构审查，不在施工位
- 触发：`MC-026` 的 `next_action` —— "Bind fresh Claude review to exact Route-B 6fc8c634, native 37def260 and capabilities fccffc8d"

## 裁定

| 候选 | 裁定 |
|---|---|
| Route-B `6fc8c634bffcc5096bf44fc9d8f390d3505a2ca4` | **ACCEPTED**（附一条新登记项 N1） |
| native adapter `37def2606682ec64155cea4f839d8238fdd42a59` | **ACCEPTED** |
| capabilities `fccffc8dad68419efd66bf8d23831eeef503fa30` | **ACCEPTED_WITH_NOTES**（针对性复核，非逐行） |

**上一轮 R1 的全部阻断项与登记项已闭合。** 没有新的阻断项。

> **这三条 ACCEPTED 是对代码的裁定，不是对"测试跑绿了"的裁定。** 见第四节——Route-B 的完整命令在本机并未通过，两条轴被 Application Control 阻断。**本复核不得被用来覆盖那两条轴。**

---

## 一、R1 遗留项的闭合情况

### F1（上轮唯一阻断项）测试钩子无编译期护栏 —— **已闭合，且好于要求**

补丁四处全部包上：

```
第 36–40 行    #ifdef GOGOKE_ROUTE_B_TESTING  ... gogokeRouteBForceNextCloseFailure ... #endif
第 227–236 行  enable_full_pathname_reentry / last_reentry_result
第 273–278 行  force_next_close_failure
第 373–381 行  winClose 里的消费分支
```

`build.rs` 的门是 **`PROFILE != "release"`**，并且注释交代了为何不能用 `cfg(test)`：

> build.rs is never itself cfg(test), and CARGO_CFG_TEST is not set for this cc unit either. Release is the publishable image: test hooks stay compiled out there.

**这个措辞是准确的**：保证是"发布镜像无钩子"，不是"只有 cargo test 才有钩子"。debug 构建仍带钩子——这是 `cargo test` 需要的，且 debug 不是发布物。

超出我要求的部分是**两个真检查器**：

1. `assert_release_object_omits_test_hooks` —— release 构建后**读编译产物 `.lib`**，断言四个符号名不出现
2. `assert_route_b_test_hooks_are_ifdef_gated` —— 扫生成源码，对每个钩子的每一处出现，断言最近的 `#ifdef GOGOKE_ROUTE_B_TESTING` 晚于最近的 `#endif`

我要的只是"加一条断言"，他们做成了**制品级检查**。

### F2　`end_main_open` 三路 OR —— **已闭合，且找到了成因**

现在是严格相等：

```c
if( owned!=expected_generation ){
  return SQLITE_MISUSE_BKPT;
}
```

`lastAdopted` / `lastOwned` 在补丁里**零命中**，整个历史 OR 被拆掉了。而且补丁里留了根因说明：

> Keep generation until end_main_open. Session close is owned==expected only;
> **wiping generation here forced a historical-OR that a stale end could satisfy.**

也就是说：`cancel_main_handle` 原先会清掉 generation，才逼出了那个历史 OR。**他们没有去收紧判据，而是去掉了判据必须放松的那个原因。** 这是正确的修法顺序。

### 第三条 finding　UNKNOWN close 后是否真 poison —— **已闭合**

新增测试 `unknown_close_poisons_the_physical_root_against_later_acquire`，断言的是**后续行为**而非返回值：

```rust
matches!(reused, Err(RootLockError::Poisoned { .. }))
```

即 UNKNOWN close 之后，**对同一物理根的再次 acquire 必须失败**。这正是我上轮指出缺失的那个验收点。

### F3　`extension_admitted()` 硬编码常量 —— **已闭合**

该函数在补丁中**零命中**，Rust 侧消费者仍为 0。**删掉了**，没有留一个恒为 0 的伪测量。

### F4　close ledger 无"最新优先"、缺失与从未发生不可辨 —— **已闭合，四点全覆盖**

`same_open.rs` 的注释与实现：

> Diagnostic ring, **newest-first**. **Missing generation is fail-closed (CloseLedgerMissing → poison), not a successful close.** Overflow is counted in `sqlite3_gogoke_close_ledger_overflow`; **the ring is not the sole durable close receipt.**

- newest-first ✅
- 缺失即 poison（失效安全），不再与"从未 close"混淆 ✅
- 溢出计数 ✅
- 明确不作关闭收据权威 ✅

### G2　native adapter 的 `as Promise<T>` 靠调用方自觉 —— **已闭合**

包装已下沉进快照边界本身：

```ts
write(bytes, context) {
  return Promise.resolve().then(() => write.call(raw, bytes, context)) as Promise<void>;
}
```

`read` / `close` 同样。现在边界自己兑现承诺，不再依赖"每个未来的消费者都记得包"。

---

## 二、新登记项

### N1　release 检查器可能空跑通过　【登记，非阻断】

```rust
fn assert_release_object_omits_test_hooks(out_dir: &PathBuf) {
    let lib = out_dir.join("gogoke_sqlite3.lib");
    let bytes = fs::read(&lib).unwrap_or_default();      // ← 这里
    let as_ascii = String::from_utf8_lossy(&bytes);
    for hook in [...] {
        assert!(!as_ascii.contains(hook), "...");
    }
}
```

`unwrap_or_default()` 在读取失败时返回**空 Vec**。空字符串不包含任何钩子名，**四条断言全部通过**。

于是：文件名写错、路径变更、产物尚未落盘、权限问题——任一情况下，这个检查器都会**安静地通过**，而不是报错。

这正是本项目自己的规矩在构建检查上的同款：

> RUNBOOK R3：**0 个测试或 test 目标缺失为 `FAIL_INSTRUMENT`；checker 实现先用一项真实失败反例证明没有空跑。**

**建议**（两行的事）：

1. 断言 `bytes` 非空，读取失败直接 panic 而不是 `unwrap_or_default()`
2. 再断言一个**已知必然存在的非钩子符号**（例如 `sqlite3_gogoke_bind_main_handle`）**确实出现**——证明检查器读的是正确的产物，而不是在检查一段空气

第 2 条是关键：它让"没找到钩子"这个结论具备了正向对照。

### N2　`assert_route_b_test_hooks_are_ifdef_gated` 的文本判据对嵌套敏感　【登记，当前正确】

该检查用 `before.rfind("#endif")` 找最近的 `#endif`，而 `#endif` 是**任意** `#ifdef` 的收尾。若将来在 `GOGOKE_ROUTE_B_TESTING` 守卫块**内部**出现任何其它 `#ifdef ... #endif`（例如 `SQLITE_OS_WIN`），最近的 `#endif` 会是那个内层的，判据将误报。

当前守卫块内无嵌套，检查正确。仅作为将来修改补丁时的注意事项登记。

---

## 三、capabilities `fccffc8d` 的复核范围与结论

**针对性复核，非逐行通读。** `catalog.ts` 1003 行、`types.ts` 266 行、`catalog.test.ts` 559 行，我查的是 R4 的 O4 规则是否落实。

查到并成立的：

- **`qualified` 来自 `observation.capabilities`（`catalog.ts:625`），不是来自 `declared`。** 这是 O4「静态 true 不等于探测结果」的核心分界，成立
- `declared` 只在 `:719-724` 被用于**携带来源标注**，不参与生成资格
- 资格按 `qualificationKey` 整体替换（`:649-654`）并有撤销路径（`:691-695`），符合"binary/认证/profile/模式变化使旧快照失效"
- `AUTHORITY_REPLAY`（`:429`，"transition evidence was already used"）—— 权威转移有重放保护
- `:315` 注释明确 passive claim **never** 成为 ReadyHost / grant / source / qualification
- 与 codec 一致使用 `cloneAndFreezeJson`

**未覆盖**：三个文件均未逐行通读；`catalog.test.ts` 的 17 个用例未逐条验判据强度；未执行任何测试。

因此裁定为 **ACCEPTED_WITH_NOTES**：就 O4 分层而言成立，**但不构成对该模块的完整审计**。若要计入 WP09a，需要一轮逐行 + 判据强度复核。

---

## 四、必须与本裁定一起读的：两条被阻断的轴

`MC-026` 的 Controller 复现记录：

```
cargo test --locked --manifest-path apps/desktop/native-host/Cargo.toml -- --test-threads=1
exit_code: 1

通过（阻断之前）： native-host lib 66/66、root lock probe 12/12

b07_two_process    BLOCKED_BY_APPLICATION_CONTROL
                   Win32 error 4551；Code Integrity 3077/3033 指向未签名的
                   gogoke-root-lock-probe.exe
host_typed_ops     BLOCKED_BY_APPLICATION_CONTROL
                   Win32 error 4551；Code Integrity 3077 指向未签名的
                   gogoke-native-host.exe
```

**Route-B 的完整命令没有通过。** Codex 如实记录了这一点，并且**没有尝试任何策略绕过或签名例外**——这是正确的。

我要把这条钉在裁定旁边，原因是：

1. **本复核是静态的**，本来就不依赖那两条轴的绿灯；但**反过来也成立——我的 ACCEPTED 不能被用来替代那两条轴。**
2. 被阻断的恰好是**真实子进程**相关的两条：双进程 root lock 与 typed ops 宿主。也就是说，Route-B 关于"进程间唯一 writer"的保证，**目前只有单进程内的测试支持**。
3. 解除阻断的两条路——**给二进制签名**，或**调整 Application Control 策略**——**都是 Owner 的决定**。复核方与施工方都不应触碰系统安全设置。

**在这两条轴取得真实结果之前，Route-B 不得被记为进程间保证已验证。**

---

## 五、本次复核的边界

- **未执行任何测试。** lib 66/66、probe 12/12、native 25/25、capabilities 17/17 均为 Controller 复现自报，本文按 `[文档记载]` 处理
- **未做任何 Windows 运行时验证**：句柄语义、Job 归属、真实 `CloseHandle` 失败、SQLite 采纳时序，全为静态推演
- **Route-B**：补丁、`build.rs`、`same_open.rs` 的相关区段已读；`root/mod.rs`（1951 行）未逐行通读
- **native adapter**：仅复核 G2 的闭合；`types.ts` 1121 行、`registry.ts`、`fake.ts` 未逐行通读
- **capabilities**：见第三节范围说明
- **未审 B4 `0e9e676a`** —— 该候选在本复核绑定之后产生，需另行绑定复核

以上未覆盖项若要纳入门禁，需要另一轮在施工流程内的实测复核。
