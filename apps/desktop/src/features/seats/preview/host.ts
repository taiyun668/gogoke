/// <reference lib="es2022.object" />
import { V37UiForwardingFakePort } from "../../../../../../third_party/t3code/apps/server/src/gogoke/contracts/v37/uiFake";
import {
  decodeV37Receipt, decodeV37Request, encodeV37Receipt, encodeV37Request,
  V37_SCHEMA, type V37Request, type V37TrustedCaller,
} from "../../../../../../third_party/t3code/apps/server/src/gogoke/contracts/v37/protocol";
import type { JsonObject } from "../../../../../../third_party/t3code/apps/server/src/gogoke/contracts/model";
import { DESIGN37_INSTANCES_SCHEMA, DESIGN37_TEST_INSTANCE_ID, type Design37InstanceState } from "../design37Instances";

/** Browser-only K-UI fixture. It never opens a browser, CLI, credential or native bridge. */
export function createPreviewHost() {
  let state: Design37InstanceState = "NOT_LOGGED_IN";
  let present = true;
  let counter = 0;
  let revision = 1;
  let login: Record<string, unknown> | undefined;
  let showNewVersion = false;
  const caller: V37TrustedCaller = {
    principalId: "previewUser", seatId: "previewUser", domainId: "preview",
    role: "user", policyRevision: "1", revocationHead: "preview",
  };
  const allowed = (_caller: V37TrustedCaller, request: V37Request) =>
    request.domainId === "preview" && request.targetId === DESIGN37_TEST_INSTANCE_ID;
  const page = () => ({ schema: DESIGN37_INSTANCES_SCHEMA, instances: present ? [{
    instanceId: DESIGN37_TEST_INSTANCE_ID, driverId: "codex",
    version: showNewVersion ? "0.149.0" : "0.160.0",
    ...(showNewVersion ? { newVersion: "0.160.0" } : {}),
    revision: String(revision), state, ...(login ? { login } : {}),
  }] : [] });
  const port = new V37UiForwardingFakePort({
    caller: () => caller, granted: allowed,
    resolve: request => ({ ...request, family: "K-INSTANCE",
      operation: request.payload.command === "gogoke_design37_instance_register" ? "register" : "login-state" }),
    source: { async execute(bytes) {
      const request = decodeV37Request(bytes);
      const previousRevision = String(revision);
      const command = request.payload.command;
      if (command === "gogoke_design37_instance_register") {
        present = true;
        state = "NOT_LOGGED_IN";
      } else if (command === "gogoke_design37_instance_login") {
        if (!present || state === "NOT_INSTALLED") throw new Error("PREVIEW_INSTANCE_NOT_INSTALLED");
        if (state !== "LOGGED_IN" && !login?.settled && login?.state === "PENDING") return reply();
        if (state !== "LOGGED_IN") {
          state = "NOT_LOGGED_IN";
          login = { requestId: request.requestId, expectedRevision: revision, state: "PENDING",
            output: "预览登录正在进行。", browserState: "NOT_REQUESTED", startedAt: Date.now(), settled: false };
        }
      } else if (command === "gogoke_design37_instance_cancel" && login?.state === "PENDING") {
        state = "NOT_LOGGED_IN";
        login = { ...login, state: "CANCELLED", output: "预览登录已取消。", settled: true };
      }
      return reply();
      function reply() {
        return encodeV37Receipt({ schema: V37_SCHEMA, family: request.family, operation: request.operation,
          requestId: request.requestId, targetId: request.targetId, status: "APPLIED", previousRevision,
          revision: String(revision), result: { page: JSON.parse(JSON.stringify(page())) as JsonObject } });
      }
    } },
  });
  return {
    async invoke(command: string, args?: Record<string, unknown>) {
      if (!["gogoke_design37_instances", "gogoke_design37_instance_register",
        "gogoke_design37_instance_login", "gogoke_design37_instance_cancel"].includes(command)) {
        throw new Error(`PREVIEW_UNSUPPORTED_COMMAND: ${command}`);
      }
      if (args?.instanceId !== undefined && args.instanceId !== DESIGN37_TEST_INSTANCE_ID) {
        throw new Error("PREVIEW_INSTANCE_MISMATCH");
      }
      const request: V37Request = { schema: V37_SCHEMA, family: "K-UI",
        operation: command === "gogoke_design37_instances" ? "read-models" : "actions",
        requestId: `preview_${++counter}`, targetId: DESIGN37_TEST_INSTANCE_ID,
        domainId: "preview", expectedRevision: String(revision), payload: { command } };
      const receipt = decodeV37Receipt(await port.execute(encodeV37Request(request)));
      if (receipt.status !== "APPLIED") throw new Error(`PREVIEW_RECEIPT: ${receipt.status}`);
      return receipt.result.page;
    },
    setState(next: Design37InstanceState | "ABSENT") {
      present = next !== "ABSENT";
      state = next === "ABSENT" ? "NOT_LOGGED_IN" : next;
      login = next === "ERROR" ? { requestId: "preview_failure", expectedRevision: revision,
        state: "ERROR", output: "PREVIEW_CLI_FAILED: synthetic failure", error: "PREVIEW_CLI_FAILED: synthetic failure",
        browserState: "NOT_REQUESTED", startedAt: Date.now(), settled: true } : undefined;
    },
    setNewVersion(visible: boolean) { showNewVersion = visible; },
    settle(success: boolean) {
      if (login?.state !== "PENDING") return;
      state = success ? "LOGGED_IN" : "ERROR";
      if (success) revision += 1;
      login = { ...login, state: success ? "LOGGED_IN" : "ERROR", settled: true,
        output: success ? "预览宿主检测到登录成功。" : "PREVIEW_CLI_FAILED: synthetic failure",
        ...(success ? {} : { error: "PREVIEW_CLI_FAILED: synthetic failure" }) };
    },
  };
}
