# M3 V15 narrow installed UI read-only slice

`m3-ui-readonly-win11.mjs` uses the existing `ActualProduct` launcher, registry/custody check, installed byte pins and `e2e-webview.mjs` connection. It only reads the actual candidate's rendered UI. The runner never opens an authentication page, reads account text or credentials, calls a model, checks quota, clicks login/check/save/create/delete/forward/send, or clicks the Secretary, Seats or SideChat tabs.

The private config follows the existing installed-candidate entries and contains `installed`, `version`, `sourceCommit`, the three `installedSha256` pins, `registryKey`, signed `pwsh`, `stateRoot`, `testbedSource`, a new separate `evidenceDirectory`, a fresh `result`, and `testerArmy: true`. `m3UiReadonly` must contain `expectedSecretaryState: "UNSET"`, the exact rendered `instanceLabel`, and its exact rendered `instanceSummaryText`. `workspaceName` and `panelLabels: ["席位", "旁聊"]` are optional together: when present, the runner selects that existing workspace row for navigation only and checks both hard tab labels without activating either tab; when absent, the panel-label slice is explicitly `NOT_RUN`.

The directly observed slice is:

- the fixed `.sidebar > .sec-entry` reads `秘书长`, the host-backed UNSET line `还没设置：选一个实例它才能干活`, and is disabled; its direct sibling `.sidebar-body` proves the entry is outside the project scroll region;
- Settings navigation opens the real Instances page and reads one exact `.instances-row` title and summary. No row control is invoked;
- an optional existing `.workspace-row` is selected only to expose the real project panel. `[role=tab][aria-label="席位"]` and `[role=tab][aria-label="旁聊"]` are read as labels; neither is clicked.

Every UI observation is bound to the installed candidate launch in the journal. The run closes the actual product with the normal caption close. On an unknown UI read, failed launch, or unconfirmed close, the original evidence is retained and the process is not killed or retried. This is a preparation slice only: all 25 V15 states, configured Secretary, conversation, operations, model behavior, credential/account facts and V15/M3 acceptance remain `NOT_RUN`.

References checked before writing: `product-cdp.mjs`, `e2e-webview.mjs`, the existing M1 Settings locator, `m3-secretary-win11.mjs`, Claude's original `Secretary.tsx`/`secretaryModel.ts` at `9aa6e76a`, the slow-read repair at `ee41b713`, `MainApp.tsx`, `PanelTabs.tsx`, `InstancesPage.tsx`, `SeatsPanel.tsx`, `SideChatPanel.tsx`, the `SECRETARY-UI-INTEGRATION` checkpoint, and PLAN V15. The implementation keeps those existing host/UI owners and adds no product, G, contract or schema code.

Syntax-only checks, without launching the product:

```powershell
node --check tools/e2e/m3-ui-readonly-win11.mjs
git diff --check
```
