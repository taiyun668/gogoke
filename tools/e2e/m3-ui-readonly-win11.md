# M3 V15 narrow installed UI read-only slice

`m3-ui-readonly-win11.mjs` uses the existing `ActualProduct` launcher, registry/custody check, installed byte pins and `e2e-webview.mjs` connection. It only reads the actual candidate's rendered UI and the two existing read-only User sources used by the UI. The runner never opens an authentication page, reads account text or credentials, calls a model, checks quota, clicks login/check/save/create/delete/forward/send, clicks the Secretary/Seats/SideChat tabs, or selects a workspace.

The private config follows the existing installed-candidate entries and contains `installed`, `version`, `sourceCommit`, the three `installedSha256` pins, `registryKey`, signed `pwsh`, `stateRoot`, `testbedSource`, a new separate `evidenceDirectory`, a fresh `result`, and `testerArmy: true`. `m3UiReadonly` must contain `expectedSecretaryState: "UNSET"` and the actual target `instanceId`. The runner first invokes the real User `secretary-configuration-read` frame and retains only its exact minimal UNSET/NONE request/reply; then it projects only `instanceId`, vendor, version, login/state, profile name, enabled, cap presence and an error-presence flag from the real `gogoke_design37_instances` and `instance-management-read` replies. Account, plan, raw error, model list, usage and full DOM fields are never collected. The panel-label slice is read only when the real visible `.workspace-row.active` proves a current project is selected and the panel is visible; otherwise it is explicitly `NOT_RUN`.

The directly observed slice is:

- the actual User `secretary-configuration-read` reply is `gogoke.37.secretary-configuration.v1`, `state=UNSET`, `conversation.state=NONE`; the fixed `.sidebar > .sec-entry` reads `秘书长` and `还没设置：选一个实例它才能干活`, and is not clicked; its direct sibling `.sidebar-body` proves the entry is outside the project scroll region;
- the actual `gogoke_design37_instances` snapshot and User `instance-management-read` profile bind one instance ID, version, vendor, name and minimal state facts to one `.instances-row`. No row control is invoked;
- the currently selected project, proven by the existing visible `.workspace-row.active` class, exposes `[role=tab][aria-label="席位"]` and `[role=tab][aria-label="旁聊"]` when its panel is visible; both labels are read without selecting a workspace or clicking either tab. Without that active-row proof the slice remains `NOT_RUN`.

Every UI observation is bound to the installed candidate launch in the journal. The run closes the actual product with the normal caption close. On an unknown UI read, failed launch, or unconfirmed close, the original evidence is retained and the process is not killed or retried. This is a preparation slice only: all 25 V15 states, configured Secretary, conversation, operations, model behavior, credential/account facts and V15/M3 acceptance remain `NOT_RUN`.

References checked before writing: `product-cdp.mjs`, `e2e-webview.mjs`, the existing M1 Settings locator, `m3-secretary-win11.mjs`, `createDesign37SecretarySource`/`design37UserFrame`, `design37Instances`/`design37ManagedInstances`, Claude's original `Secretary.tsx`/`secretaryModel.ts` at `9aa6e76a`, the slow-read repair at `ee41b713`, `MainApp.tsx`, `PanelTabs.tsx`, `InstancesPage.tsx`, `SeatsPanel.tsx`, `SideChatPanel.tsx`, the `SECRETARY-UI-INTEGRATION` checkpoint, and PLAN V15. The implementation keeps those existing host/UI owners and adds no product, G, contract or schema code.

Syntax-only checks, without launching the product:

```powershell
node --check tools/e2e/m3-ui-readonly-win11.mjs
git diff --check
```
