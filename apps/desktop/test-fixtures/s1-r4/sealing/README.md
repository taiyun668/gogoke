# S1-R4 sealing fixture harness

This directory is the isolated fixture/harness write scope for `S1-10A-T`.
It uses Node's built-in test runner and fake dependency recording only. It does
not start a Tauri process, connect to a remote endpoint, invoke a provider,
access a microphone, download a model, start a daemon, or change Tailscale.

The tests read the current production source as evidence and exercise a fake
side-effect recorder for the route matrix below:

- deep-link navigation with an already-known connected workspace;
- old remote configuration during settings bootstrap;
- direct voice/remote/Tailscale/daemon calls;
- desktop auto-start's daemon route;
- mobile setup with an already configured legacy remote;
- disabled voice entry points; and
- model asset byte/path preservation.

The mobile test is intentionally a real negative control. It must fail while
`useMobileServerSetup.ts` still probes `listWorkspaces`, persists remote
settings, or refreshes workspaces during the sealed phase. That production file
is outside this task's write scope; the failure is returned to the Controller
for the S1-10A-U production decision.

Run the self-contained selector from the repository root:

```text
node --test apps/desktop/test-fixtures/s1-r4/sealing/sealing.test.mjs
```

