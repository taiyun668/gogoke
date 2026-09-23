# 独立复核：Route-B `0ec79fcb` 与 native adapter `30be1416`

- 复核：Claude（Opus 5），2026-09-20
- 角色：最终异构审查（不在施工位）
- 候选：
  - `codex/r4-sqlite-route-b-b1` @ `0ec79fcbeed42c659b6c4dbcd39df050fdb669ce`
  - `codex/r4-g2-native-adapters-r1` @ `30be1416195d537ae96cf06cb69c4895a5b1b074`

**方法**：静态阅读全部改动 + 逐路径推演。**本次未执行任何测试**——复核者不在施工位，不写 `target/`，不碰工作树。因此施工位自报的 24/24、12/12、25/25 在本文中一律按 `[文档记载]` 对待，未复现。

**裁定**：两个候选都**不是 FAIL**，但都有必须处理的项。

| 候选 | 裁定 |
|---|---|
| Route-B `0ec79fcb` | **CONDITIONAL — 需修 F1，其余登记** |
| native adapter `30be1416` | **ACCEPTED_WITH_NOTES — G2 建议改，非阻断** |

---

## 一、Route-B `0ec79fcb`

改动范围：`build.rs`（4 行）、`root/mod.rs`（16 行）、`store/same_open.rs`（145 行）、`vendor/sqlite-3.53.2/patches/route_b_same_open.patch`（87 行）。

### F1　测试钩子无编译期护栏，进入生产制品　【需修】

补丁向 SQLite 注入了三个**导出**函数与一个全局开关，**全文没有任何 `#ifdef` 包裹**：

```c
static volatile LONG gogokeRouteBForceNextCloseFailure = 0;          // 第 35 行

SQLITE_API int sqlite3_gogoke_test_enable_full_pathname_reentry(void) // 第 222 行
SQLITE_API int sqlite3_gogoke_test_last_reentry_result(void)          // 第 227 行
SQLITE_API int sqlite3_gogoke_test_force_next_close_failure(void)     // 第 255 行
```

消费点在 winClose 路径：

```c
if( InterlockedExchange(&gogokeRouteBForceNextCloseFailure, 0) ){     // 第 357 行
```

Rust 侧确实做了隔离，但**只隔离了声明**：

```rust
#[cfg(test)] fn sqlite3_gogoke_test_force_next_close_failure() -> c_int;
#[cfg(test)] fn sqlite3_gogoke_test_enable_full_pathname_reentry() -> c_int;
#[cfg(test)] fn sqlite3_gogoke_test_last_reentry_result() -> c_int;
```

`#[cfg(test)]` 管的是 Rust 要不要声明这个外部符号；**C 侧照常编译并 `SQLITE_API` 导出**。`build.rs` 也没有定义任何可用来剥离它们的宏（现有 define 只有 `SQLITE_THREADSAFE`、`SQLITE_DQS`、`SQLITE_DEFAULT_FOREIGN_KEYS`、`SQLITE_DEFAULT_WAL_SYNCHRONOUS`、`SQLITE_OMIT_LOAD_EXTENSION`、`SQLITE_TRUSTED_SCHEMA`）。

**后果**：发布制品里带着一个"让下一次 `CloseHandle` 失败"的开关。按本候选自己的语义，close 失败 → 报 UNKNOWN → poison 物理根。也就是说，**成品里存在一个可触发的"数据库拒绝服务"入口**。

**公平记录的缓解事实**（不改变结论，但决定严重度）：

- `build.rs` 已 define `SQLITE_OMIT_LOAD_EXTENSION` 与 `SQLITE_TRUSTED_SCHEMA=0`，堵掉了可加载扩展这条路
- native-host 是可执行文件而非 DLL，符号暴露面小于共享库
- 因此**不是远程可达**，严重度是中，不是高

**但它触碰的是本项目的分层本身**：R4 立了 `verified_fixture` 与 `verified_native` 的分界，而一个把测试夹具编进被测物的制品，恰好让这条界线在二进制层面消失了。

**修法很便宜**：三个函数与第 357 行的消费分支用 `#ifdef GOGOKE_ROUTE_B_TESTING` 包住，`build.rs` 仅在 `cfg(test)` 下 `.define("GOGOKE_ROUTE_B_TESTING", None)`。之后发布制品**可证明**无法强制 close 失败。

> 注：`same_thread_fullpathname_reentry_cannot_steal_pending_handoff` 这条测试**依赖** `test_enable_full_pathname_reentry` 才能构造攻击场景。所以钩子必须保留，只是要被编译期隔离——不是删掉。

### F2　`end_main_open` 的世代校验是三路 OR，测试只覆盖了一支　【问】

```c
if( expected_generation!=0
 && owned!=expected_generation
 && lastAdopted!=expected_generation
 && lastOwned!=expected_generation ){
  return SQLITE_MISUSE_BKPT;
}
```

即：`expected` 只要等于**当前世代、上次被采纳的世代、上次被绑定的世代**三者之一，就通过。

我把 `stale_owner_end_does_not_clear_a_newer_owner` 逐行读了一遍，它走的路径是：

1. require → bind（`first`）→ cancel(`first`) → end(`first`) OK
2. require → bind（`second`），此时 `owned=second`、`lastOwned=second`、`lastAdopted=0`
3. `end(first)` → 三个值全不匹配 → `SQLITE_MISUSE_BKPT` → 断言通过

**也就是说测试验的是"三者全不匹配"这一支，OR 的另外两支没有被覆盖。**

我尝试构造走 `lastAdopted` 分支的破绽，推演下来都被后面的 `pending != 0 → SQLITE_BUSY` 兜住了，**没能构造出可利用的序列**。所以这不是确认缺陷，是一个问题：

- 为什么需要接受两个历史世代？若无必要，收紧为 `owned == expected_generation`
- 若确有必要（例如采纳后 `owned` 被清零导致合法的 end 无法匹配），请补一条**专门走 `lastAdopted` 分支**的测试，把那条路径的安全性也钉住

理由是这个项目自己的教训：`isolation.test.ts` 开头那段记的就是"判据挑中了唯一被满足的维度"。一个三路 OR 只测一支，正是同一个形状。

### F3　`sqlite3_gogoke_extension_admitted()` 是硬编码常量，且无人消费　【登记】

```c
SQLITE_API int sqlite3_gogoke_extension_admitted(void){
  return 0;
}
```

我在 Rust 侧全文搜索 `extension_admitted`，**零命中**。

所以它**现在不是假判据**（没有任何断言建立在它上面）。但它是一个**长得像测量的常量**。五条原始 finding 里有一条是"strict open 后仍可注册并运行 automatic extension"，将来很容易有人拿这个函数当"已确认无扩展"的证据。

建议二选一：删掉，或者真的实现成计数。留一个恒为 0 的访问器是在埋雷。

### F4　close ledger 的查询没有"最新优先"规则　【登记】

`gogokeRouteBCloseLedger` 是 32 槽环形缓冲，游标 `InterlockedIncrement(&cursor)-1` 再取模。`sqlite3_gogoke_get_close_ledger` **线性扫描返回第一个世代匹配的槽**。

两个后果：

- 超过 32 次 close 之后，被查询的世代记录可能已被覆盖，查询返回 `SQLITE_NOTFOUND`——**与"从未发生过 close"无法区分**
- 若同一世代因某种路径写了两条记录，扫描返回的是索引顺序上的第一条，不是最新一条

建议：要么加溢出计数让"被覆盖"可辨，要么在文档里明确 **ledger 仅供诊断，不得作为关闭收据的权威**。

### 做对的地方（值得留档）

**写入顺序是对的。** `gogokeRouteBRecordClose` 先写 `calls`/`result`/`error`，**最后写 `generation`**。读者按 generation 扫描，因而不会读到半写的槽。这是正确的无锁发布顺序，不是碰巧。

**重复 close 被挡住了。**

```c
if( pFile->gogokeRouteBAdopted && pFile->gogokeRouteBCloseAttempted ){
  return SQLITE_IOERR_CLOSE;   /* 重复的 SQLite close 不得再次到达 CloseHandle */
}
```

直接对应第一批五条里的"CloseHandle 失败后重试"。

**绑定入口的校验是齐的。** `bind_main_handle` 在写入前检查：strict 已开启、调用方是 strict owner 线程、路径非空、**路径不含 URI query**、rootFileId 长度恰为 16、句柄可用（非 INVALID、readWrite、`GetFileType==FILE_TYPE_DISK`）。而且 `pending` 用 `InterlockedCompareExchange(1,0)` 抢占，失败返回 `SQLITE_BUSY`，不是覆盖。

### 三条 finding 的覆盖情况

| finding | 覆盖 |
|---|---|
| 过期 owner end 清掉新 owner | ✅ `stale_owner_end_does_not_clear_a_newer_owner`，我逐行读过 |
| 同线程重入窃取 handshake | ✅ `same_thread_fullpathname_reentry_cannot_steal_pending_handoff`（依赖 F1 的钩子构造） |
| UNKNOWN close 后丢失 root custody | ⚠️ **部分。** `checked_close_reports_unknown_after_one_native_attempt` 覆盖了"只尝试一次就报 UNKNOWN"；但**"UNKNOWN 之后物理根进入 poisoned、且后续操作被拒"这一步，我没有找到独立断言** |

第三条是我上一轮就提出的疑问，现在确认。**建议补一条测试**：UNKNOWN close 之后，对同一物理根的后续操作必须失败，而不只是 close 的返回值是 UNKNOWN。否则"poison 物理根"这个行为只有实现、没有验收点。

---

## 二、native adapter `30be1416`

文件：`types.ts`（1121）、`native.test.ts`（740）、`ports.ts`（598）、`registry.ts`（424）、`fake.ts`（319）、`native.static.test.ts`（28）、`index.ts`（4）。

### G1　边界设计是对的　【表扬，要留档】

`snapshotActiveProcessPort` 的结构值得作为本项目的范式：

```ts
const record = snapshotPassiveRecord(value, "activePort", undefined, "PORT_PROTOCOL_ERROR");
```

**先快照，之后一切判断读 `record`，不读 `value`。** 这一步堵死了"getter 每次返回不同值"这类重读攻击——而这正是 MC-005 在 codec 上查出的那一类（"已验证对象在上锁前可被掉包"）。

更细的三点：

1. **返回的 port 携带的是可信值，不是 port 自称的值**：`binding` 用校验后的副本、`custodyRef` 用**调用方的期望值**而非 `record.custodyRef`、`processIdentity` 用校验后的副本
2. **`captureMethod` 检查描述符是 data property 而非 accessor**（`ports.ts:96-101`），防的是"取方法时返回 A、调用时已换成 B"
3. **方法先捕获再调用**：`write.call(raw, ...)`，捕获后对象上的同名属性即便被替换也无效

这是我在这个项目里读到的最扎实的边界代码。

### G2　`as Promise<T>` 的安全性靠调用方自觉，不是边界强制　【建议改，非阻断】

快照返回的三个方法是：

```ts
write(bytes, context) { return write.call(raw, bytes, context) as Promise<void>; }
read(context)        { return read.call(raw, context) as Promise<Readonly<NativeFrame>>; }
close(reason, ctx)   { return close.call(raw, reason, ctx) as Promise<NativeCloseReceipt>; }
```

返回值**未经校验**，直接 `as`。而 port 实现是不可信输入。

实际上现在是安全的——因为调用点包了一层：

```ts
Promise.resolve().then(() => active.write(bytes, context))    // ports.ts:286
Promise.resolve().then(() => active.read(context))            // ports.ts:302
```

`Promise.resolve().then(...)` 会按规范走 promise 解析流程，thenable 的 `then` 至多被调用一次，非 promise 返回值也会被正常化。这也正是 `0e95cd91 fix: capture R4 thenables once` 那一轮的成果。

**问题在于这是"靠每个调用点记得包"的不变量，不是边界自己保证的。** 未来任何一处直接 `await activePort.write(...)`，保护就没了，而且不会有任何东西报错。

**建议**：把 `Promise.resolve().then(...)` 挪进 `snapshotActiveProcessPort` 内部，让边界自己兑现承诺。这条正是 MC-005 立下的那句——**"TypeScript `as T` 不是校验"**——在这个文件里的同款。

非阻断：当前所有调用点都包了，我逐个看过。

### G3　未复现测试

施工位自报 `native.test.ts` + `native.static.test.ts` **25/25**。我**没有执行**。本文对该数字的引用一律是 `[文档记载]`。

---

## 三、给施工位的处置建议

| 编号 | 处置 | 阻断？ |
|---|---|---|
| F1 | 用 `#ifdef GOGOKE_ROUTE_B_TESTING` 隔离三个测试钩子与第 357 行消费分支；`build.rs` 仅 `cfg(test)` 下 define | **是**——G1/G2 转 ACCEPTED 前必须闭合 |
| F2 | 回答为何需要三路 OR；收紧或补 `lastAdopted` 分支的测试 | 否，但要有书面回答 |
| F3 | 删除或实现 `extension_admitted()` | 否 |
| F4 | ledger 加溢出计数，或明确其只作诊断 | 否 |
| 第三条 finding | 补"UNKNOWN close 后物理根被 poison 且后续操作被拒"的断言 | 否，但**该 finding 在此之前不得记为已闭合** |
| G2 | 把 promise 包装下沉到快照边界 | 否 |

**两个候选都不是 FAIL。** F1 是唯一需要在门禁前闭合的。

---

## 四、本次复核的边界

- **未执行任何测试。** 所有测试通过数均为施工位自报，本文按 `[文档记载]` 处理，未复现
- **未在 Windows 上做任何运行时验证**：句柄语义、Job 归属、`CloseHandle` 真实失败行为、SQLite 采纳路径的实际时序，全部只做了静态推演
- **未审**：`types.ts`（1121 行）、`registry.ts`（424 行）、`fake.ts`（319 行）仅做了针对性检索，未逐行通读
- **未审**：`root/mod.rs` 本候选新增的 16 行改动
- F2 的破绽我**尝试构造但未能构造成功**，因此登记为问题而非缺陷；若施工位能构造出来，应升级处置

以上未覆盖项若要纳入门禁，需要另一轮在施工流程内的实测复核。
