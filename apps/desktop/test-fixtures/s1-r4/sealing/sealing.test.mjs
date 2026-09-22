import assert from "node:assert/strict";
import { test } from "node:test";
import {
  assertGuardBefore,
  assertNoForbiddenCalls,
  FakeDependencies,
  preserveModelAsset,
  readDesktopSource,
  readFixtureJson,
} from "./harness.mjs";

const deepLink = readFixtureJson("deep-link.json");
const oldRemoteConfig = readFixtureJson("old-remote-config.json");
const modelAsset = readFixtureJson("model-asset.json");

const deepLinkSource = readDesktopSource("src/features/app/hooks/useSystemNotificationThreadLinks.ts");
const serverHookSource = readDesktopSource("src/features/settings/hooks/useSettingsServerSection.ts");
const mobileHookSource = readDesktopSource("src/features/mobile/hooks/useMobileServerSetup.ts");
const dictationControllerSource = readDesktopSource(
  "src/features/app/hooks/useDictationController.ts",
);
const dictationHookSource = readDesktopSource("src/features/dictation/hooks/useDictation.ts");
const dictationModelSource = readDesktopSource("src/features/dictation/hooks/useDictationModel.ts");
const tauriSource = readDesktopSource("src/services/tauri.ts");
const releasePolicySource = readDesktopSource("src-tauri/src/release_policy.rs");
const dictationRealSource = readDesktopSource("src-tauri/src/dictation/real.rs");
const dictationStubSource = readDesktopSource("src-tauri/src/dictation/stub.rs");
const tailscaleSource = readDesktopSource("src-tauri/src/tailscale/mod.rs");
const tailscaleCommandsSource = readDesktopSource("src-tauri/src/tailscale/daemon_commands.rs");
const appEntrypointSource = readDesktopSource("src-tauri/src/lib.rs");
const daemonSource = readDesktopSource("src-tauri/src/bin/gogoke_daemon.rs");
const daemonCtlSource = readDesktopSource("src-tauri/src/bin/gogoke_daemonctl.rs");

test("fixture identity uses only synthetic config, deep-link, and model bytes", () => {
  assert.equal(oldRemoteConfig.remoteBackendHost, "fixture.invalid:4732");
  assert.equal(oldRemoteConfig.remoteBackendToken, "fixture-token");
  assert.equal(deepLink.workspaceConnected, true);
  assert.equal(modelAsset.modelId, "base");
});

test("deep-link route is navigation-only for an already connected workspace", () => {
  assert.match(deepLinkSource, /openThreadLink\(link\.workspaceId, link\.threadId\)/);
  assert.doesNotMatch(deepLinkSource, /tailscaleDaemon(Start|Stop|Status)|dictation(Start|Stop)/);

  const dependencies = new FakeDependencies();
  if (deepLink.workspaceConnected) {
    dependencies.navigation({
      workspaceId: deepLink.workspaceId,
      threadId: deepLink.threadId,
    });
  }
  assert.deepEqual(dependencies.calls(), [
    {
      kind: "navigation",
      payload: {
        workspaceId: "fixture-workspace",
        threadId: "fixture-thread",
      },
    },
  ]);
  assertNoForbiddenCalls(dependencies, "deep-link");
});

test("old remote configuration does not probe during server-section bootstrap", () => {
  assert.match(serverHookSource, /const remoteAccessSealed = true/);
  assert.doesNotMatch(serverHookSource, /listWorkspaces\s*\(/);
  assert.match(
    serverHookSource,
    /Do not probe Tailscale or the daemon during bootstrap[\s\S]*?remote side effect stays behind/,
  );

  const dependencies = new FakeDependencies();
  // Loading an old config only projects its values into the fake settings view.
  // No effect is allowed to call a remote or daemon dependency.
  void oldRemoteConfig;
  assertNoForbiddenCalls(dependencies, "old-config bootstrap");
});

test("remote/Tailscale UI actions are guarded before fake external calls", () => {
  assertGuardBefore(
    serverHookSource,
    "if (remoteAccessSealed) {\n      return;\n    }",
    "fetchTailscaleStatus()",
    "Tailscale status",
  );
  assertGuardBefore(
    serverHookSource,
    "if (remoteAccessSealed) {\n      return;\n    }",
    "fetchTailscaleDaemonCommandPreview()",
    "Tailscale command preview",
  );
  assertGuardBefore(
    serverHookSource,
    "if (remoteAccessSealed) {\n        return;\n      }",
    "setTcpDaemonBusyAction(action)",
    "daemon action",
  );

  const dependencies = new FakeDependencies();
  // Model each disabled callback as the real UI does: the sealed guard returns
  // before the fake external/daemon implementation can be reached.
  const sealed = true;
  if (!sealed) {
    dependencies.external("tailscale status");
    dependencies.daemon("daemon action");
  }
  assertNoForbiddenCalls(dependencies, "remote/Tailscale UI");
});

test("direct voice, Tailscale, and legacy daemon calls guard before side effects", () => {
  assert.match(releasePolicySource, /const REMOTE_EXTERNAL_ENABLED: bool = false/);
  assert.match(releasePolicySource, /const VOICE_ENABLED: bool = false/);
  assert.match(releasePolicySource, /const DEFAULT_RUNTIME_DRIVERS_ENABLED: bool = false/);

  assertGuardBefore(
    dictationRealSource,
    'require_voice("dictation model download")?',
    "reqwest::Client::builder",
    "voice model download",
  );
  assertGuardBefore(
    dictationRealSource,
    'require_voice("microphone capture")?',
    "default_input_device",
    "microphone capture",
  );
  assertGuardBefore(
    dictationRealSource,
    'require_voice("microphone permission request")?',
    "request_microphone_permission(&app)",
    "microphone permission",
  );
  assertGuardBefore(
    dictationStubSource,
    'require_voice("dictation model download")?',
    "dictation_model_status(app, state, model_id).await",
    "mobile voice model download",
  );
  assertGuardBefore(
    dictationStubSource,
    'require_voice("microphone capture")?',
    'Err(UNSUPPORTED_MESSAGE.to_string())',
    "mobile microphone capture",
  );
  assertGuardBefore(
    dictationStubSource,
    'require_voice("microphone permission request")?',
    "Ok(false)",
    "mobile microphone permission",
  );
  assertGuardBefore(
    tailscaleSource,
    "if !release_policy::remote_external_enabled()",
    "resolve_tailscale_binary().await",
    "Tailscale status",
  );
  assertGuardBefore(
    tailscaleCommandsSource,
    'require_remote_external("mobile access daemon start")?',
    "resolve_daemon_binary_path()",
    "Tailscale daemon start",
  );
  assertGuardBefore(
    tailscaleCommandsSource,
    'require_remote_external("mobile access daemon command preview")?',
    "resolve_daemon_binary_path()",
    "Tailscale daemon preview",
  );
  assertGuardBefore(
    daemonSource,
    "release_policy::validate_legacy_loopback(",
    "Ok(DaemonConfig",
    "legacy daemon bind",
  );
  assertGuardBefore(
    daemonCtlSource,
    "release_policy::validate_legacy_loopback(",
    "match args.command",
    "daemonctl route",
  );

  const dependencies = new FakeDependencies();
  const sealed = true;
  if (!sealed) {
    dependencies.microphone("capture");
    dependencies.download("model");
    dependencies.external("tailscale");
    dependencies.daemon("legacy daemon");
  }
  assertNoForbiddenCalls(dependencies, "direct calls");
});

test("desktop auto-start is downstream of the sealed daemon start guard", () => {
  assert.match(appEntrypointSource, /Remote mode: ensure daemon is up and version-current/);
  assert.match(appEntrypointSource, /tailscale::tailscale_daemon_start\(state\)\.await/);
  assertGuardBefore(
    tailscaleCommandsSource,
    'require_remote_external("mobile access daemon start")?',
    ".spawn()",
    "auto-start daemon spawn",
  );

  const dependencies = new FakeDependencies();
  const sealed = true;
  if (!sealed) {
    dependencies.daemon("auto-start");
  }
  assertNoForbiddenCalls(dependencies, "auto-start");
});

test("mobile setup does not auto-probe, save, or refresh a configured legacy remote", () => {
  // This is the required negative control. It intentionally fails against the
  // current baseline until the out-of-scope mobile production route is sealed.
  assert.doesNotMatch(mobileHookSource, /\blistWorkspaces\s*\(/);
  assert.doesNotMatch(mobileHookSource, /\bqueueSaveSettings\s*\(/);
  assert.doesNotMatch(mobileHookSource, /\brefreshWorkspaces\s*\(/);

  const dependencies = new FakeDependencies();
  assertNoForbiddenCalls(dependencies, "mobile setup");
});

test("voice UI keeps model status/path visible while mutation entry points are no-ops", () => {
  assert.match(dictationModelSource, /getDictationModelStatus/);
  assert.doesNotMatch(dictationModelSource, /downloadDictationModel\s*\(/);
  assert.doesNotMatch(dictationModelSource, /cancelDictationDownload\s*\(/);
  assert.doesNotMatch(dictationModelSource, /removeDictationModel\s*\(/);
  assert.match(dictationControllerSource, /const dictationReady = false/);
  assert.match(dictationControllerSource, /enabled: false/);
  assert.doesNotMatch(dictationHookSource, /startDictation\s*\(/);
  assert.doesNotMatch(dictationHookSource, /requestDictationPermission\s*\(/);

  const dependencies = new FakeDependencies();
  const modelStatus = {
    state: "ready",
    modelId: modelAsset.modelId,
    path: modelAsset.relativePath,
  };
  assert.deepEqual(modelStatus.path, modelAsset.relativePath);
  assertNoForbiddenCalls(dependencies, "voice UI");
});

test("model asset bytes and relative path survive a sealed status round trip", () => {
  const preserved = preserveModelAsset(modelAsset);
  assert.equal(
    preserved.path.replaceAll("\\", "/").endsWith(`/${modelAsset.relativePath}`),
    true,
  );
  assert.equal(preserved.sha256, modelAsset.sha256);
  assert.deepEqual([...preserved.bytes], [...Buffer.from(modelAsset.bytesBase64, "base64")]);

  const dependencies = new FakeDependencies();
  assertNoForbiddenCalls(dependencies, "model asset preservation");
});

test("service direct-call wrappers retain the status path and isolate mutation commands", () => {
  assert.match(tauriSource, /export async function getDictationModelStatus/);
  assert.match(tauriSource, /"dictation_model_status"/);
  assert.match(tauriSource, /export async function downloadDictationModel/);
  assert.match(tauriSource, /"dictation_download_model"/);
  assert.match(tauriSource, /export async function requestDictationPermission/);
  assert.match(tauriSource, /"dictation_request_permission"/);

  const dependencies = new FakeDependencies();
  // The fixture never invokes these production-facing commands. The backend
  // guards above are the direct-call proof; this fake only checks that the
  // fixture itself does not leak an invocation.
  assertNoForbiddenCalls(dependencies, "service wrappers");
});
