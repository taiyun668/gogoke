/// <reference lib="es2022.object" />
import { V37UiForwardingFakePort } from "../../../../../../third_party/t3code/apps/server/src/gogoke/contracts/v37/uiFake";
import {
  decodeV37Receipt, decodeV37Request, encodeV37Receipt, encodeV37Request,
  V37_SCHEMA, type V37Request, type V37TrustedCaller,
} from "../../../../../../third_party/t3code/apps/server/src/gogoke/contracts/v37/protocol";
import type { JsonObject } from "../../../../../../third_party/t3code/apps/server/src/gogoke/contracts/model";
import { DESIGN37_INSTANCES_SCHEMA, DESIGN37_TEST_INSTANCE_ID, type Design37InstanceState } from "../design37Instances";
import { DESIGN37_OWNER_NOTICES_PROJECTION, type Design37OwnerNoticesRequest } from "../design37OwnerNotices";

/** Browser-only K-UI fixture. It never opens a browser, CLI, credential or native bridge. */
export function createPreviewHost() {
  let state: Design37InstanceState = "NOT_LOGGED_IN";
  let present = true;
  let counter = 0;
  let revision = 1;
  let login: Record<string, unknown> | undefined;
  let showNewVersion = false;
  let showRuntimeIssues = false;
  let ownerNoticeVisible = false;
  let ownerNoticeCause = 1;
  let ownerNoticeFailure: string | null = null;
  let ownerNoticeDelay = 0;
  let ownerReads = 0;
  let ownerCompletedReads = 0;
  let ownerInFlight = 0;
  let ownerMaxInFlight = 0;
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
    ...(showRuntimeIssues ? { runtimeIssues: [
      { seatId: "previewSeatA", sessionId: "previewSessionA", generation: "2",
        reason: "CLI_ERROR_PREVIEW: 第一席位的模拟错误，当前实例仍保持原登录状态。",
        sourceEpoch: "previewEpochA", sourceCursor: "7" },
      { seatId: "previewSeatB", sessionId: "previewSessionB", generation: "3",
        reason: '{"code":"CLI_ERROR_PREVIEW","message":"第二席位的模拟原始错误；这是 K-UI 假数据，不是 CLI 实测。"}',
        sourceEpoch: "previewEpochB", sourceCursor: "11" },
    ] } : {}),
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
    /** Explicit synthetic User source response, not native delivery or acknowledgement. */
    async executeUserSourceOperation(input: Design37OwnerNoticesRequest) {
      ownerReads += 1;
      ownerInFlight += 1;
      ownerMaxInFlight = Math.max(ownerMaxInFlight, ownerInFlight);
      const failure = ownerNoticeFailure;
      const delay = ownerNoticeDelay;
      try {
        const request = decodeV37Request(encodeV37Request(input as V37Request));
        if (request.family !== "K-INBOX" || request.operation !== "check-unknown" ||
            request.domainId !== "global" || request.targetId !== "OWNER" || request.expectedRevision !== "0" ||
            Object.keys(request.payload).length !== 1 || request.payload.projection !== DESIGN37_OWNER_NOTICES_PROJECTION) {
          throw new Error("PREVIEW_UNSUPPORTED_OWNER_SOURCE_OPERATION");
        }
        const receipt = decodeV37Receipt(encodeV37Receipt({ schema: V37_SCHEMA,
          family: request.family, operation: request.operation, requestId: request.requestId,
          targetId: request.targetId, status: "APPLIED", previousRevision: "0", revision: "0",
          result: { projection: DESIGN37_OWNER_NOTICES_PROJECTION, notices: ownerNoticeVisible ? [{
            domainId: "preview", messageId: `previewOwnerMessage${ownerNoticeCause}`, revision: "1", state: "PENDING",
            sourceSeatId: "previewSeatA", causeEventId: `previewCause${ownerNoticeCause}`, triggerId: "REJECT_CAP",
            body: `REJECT_CAP: 席位 previewSeatA 的宿主操作已被拒绝。\n升级目标：OWNER。\n原因事件：previewCause${ownerNoticeCause}。`,
          }] : [] } }));
        if (delay) await new Promise(resolve => setTimeout(resolve, delay));
        if (failure) throw new Error(failure);
        return receipt;
      } finally {
        ownerInFlight -= 1;
        ownerCompletedReads += 1;
      }
    },
    setOwnerNoticeFailure(reason: string | null) { ownerNoticeFailure = reason; },
    setOwnerNoticeDelay(milliseconds: number) { ownerNoticeDelay = milliseconds; },
    ownerNoticeReadStats() {
      return { reads: ownerReads, completed: ownerCompletedReads, inFlight: ownerInFlight, maxInFlight: ownerMaxInFlight };
    },
    setOwnerNotice(visible: boolean, newCause = false) {
      if (newCause) ownerNoticeCause += 1;
      ownerNoticeVisible = visible;
    },
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
    setRuntimeIssues(visible: boolean) { showRuntimeIssues = visible; },
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
