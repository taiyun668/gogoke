import * as NodeAssert from "node:assert/strict";
import { describe, it } from "vite-plus/test";

import { PiJsonlFramer } from "./framing.ts";
import { PI_RPC_PROTOCOL_BASELINE } from "./protocol.ts";
import {
  PiManagedSession,
  PiManagedSessionError,
  snapshotPiProtectionAdmission,
} from "./session.ts";
import type { PiProtectionAdmission, PiRpcByteSink, PiRpcDiagnostic } from "./types.ts";

const assert: typeof NodeAssert = NodeAssert;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

const ordinaryAdmission: PiProtectionAdmission = Object.freeze({
  mode: "ordinary" as const,
  protocolQualified: true,
  protectedDomainQualified: false,
  contextExposure: "UNKNOWN" as const,
});

class FakePiStream implements PiRpcByteSink {
  readonly commands: Array<Record<string, unknown>> = [];
  session: PiManagedSession | undefined;
  handler: ((command: Record<string, unknown>) => void | Promise<void>) | undefined;

  async write(chunk: Uint8Array): Promise<void> {
    const text = decoder.decode(chunk);
    assert.equal(text.endsWith("\n"), true);
    assert.equal(text.slice(0, -1).includes("\n"), false);
    const command = JSON.parse(text) as Record<string, unknown>;
    this.commands.push(command);
    await this.handler?.(command);
  }

  respond(command: Record<string, unknown>, data?: unknown): void {
    this.feed({
      type: "response",
      id: command.id,
      command: command.type,
      success: true,
      ...(data === undefined ? {} : { data }),
    });
  }

  feed(value: unknown, crlf = false): void {
    this.session?.acceptStdout(encoder.encode(`${JSON.stringify(value)}${crlf ? "\r\n" : "\n"}`));
  }
}

const fixture = (options?: {
  readonly diagnostics?: Array<PiRpcDiagnostic>;
  readonly maxFrameBytes?: number;
}): { stream: FakePiStream; session: PiManagedSession } => {
  const stream = new FakePiStream();
  const session = new PiManagedSession({
    sink: stream,
    admission: ordinaryAdmission,
    ...(options?.maxFrameBytes === undefined ? {} : { maxFrameBytes: options.maxFrameBytes }),
    ...(options?.diagnostics === undefined
      ? {}
      : { onDiagnostic: (diagnostic: PiRpcDiagnostic) => options.diagnostics?.push(diagnostic) }),
  });
  stream.session = session;
  return { stream, session };
};

describe("R4-O-PI strict framing", () => {
  it("pins the reviewed upstream RPC contract without importing the Pi SDK", () => {
    assert.equal(PI_RPC_PROTOCOL_BASELINE.commit, "d1230ea2000d876b479a69b8b061f9d670f262f5");
    assert.deepEqual(PI_RPC_PROTOCOL_BASELINE.launchArguments, ["--mode", "rpc"]);
    assert.equal(PI_RPC_PROTOCOL_BASELINE.transportOwnership, "external-managed-host");
    assert.equal(Object.isFrozen(PI_RPC_PROTOCOL_BASELINE), true);
  });

  it("splits only on LF, accepts CRLF, and preserves Unicode separators in JSON strings", () => {
    const frames: unknown[] = [];
    const diagnostics: PiRpcDiagnostic[] = [];
    const framer = new PiJsonlFramer({
      maxFrameBytes: 1024,
      onFrame: (frame) => frames.push(frame),
      onDiagnostic: (diagnostic) => diagnostics.push(diagnostic),
    });
    const payload = `left\u2028middle\u2029right`;
    const first = JSON.stringify({ payload });
    const second = JSON.stringify({ payload: "crlf" });
    const bytes = encoder.encode(`${first}\n${second}\r\n`);
    framer.push(bytes.slice(0, 7));
    framer.push(bytes.slice(7, 19));
    framer.push(bytes.slice(19));
    assert.deepEqual(frames, [{ payload }, { payload: "crlf" }]);
    assert.deepEqual(diagnostics, []);
  });

  it("diagnoses invalid JSON, invalid UTF-8, partial EOF, and oversize while recovering", () => {
    const frames: unknown[] = [];
    const diagnostics: PiRpcDiagnostic[] = [];
    const framer = new PiJsonlFramer({
      maxFrameBytes: 16,
      onFrame: (frame) => frames.push(frame),
      onDiagnostic: (diagnostic) => diagnostics.push(diagnostic),
    });
    framer.push(encoder.encode("{bad}\n"));
    framer.push(new Uint8Array([0xff, 0x0a]));
    framer.push(encoder.encode(`${"x".repeat(40)}\n{"ok":1}\n`));
    framer.push(encoder.encode('{"partial":'));
    framer.finish();
    assert.deepEqual(frames, [{ ok: 1 }]);
    assert.deepEqual(
      diagnostics.map((entry) => entry.code),
      ["INVALID_JSON", "INVALID_UTF8", "OVERSIZE_FRAME", "PARTIAL_FRAME_EOF"],
    );
  });
});

describe("R4-O-PI managed ACK and binding semantics", () => {
  it("reports prompt success as accepted, not completed", async () => {
    const { stream, session } = fixture();
    stream.handler = (command) => stream.respond(command);
    const result = await session.prompt("work", "followUp");
    assert.deepEqual(result, {
      status: "accepted",
      requestId: "gogoke-pi-1",
      command: "prompt",
    });
    assert.equal(Object.hasOwn(result, "completed"), false);
    assert.equal(stream.commands[0]?.streamingBehavior, "followUp");
  });

  it("never creates a binding when new_session is cancelled", async () => {
    const { stream, session } = fixture();
    stream.handler = (command) => {
      assert.equal(command.type, "new_session");
      stream.respond(command, { cancelled: true });
    };
    assert.deepEqual(await session.newSession(), { status: "cancelled" });
    assert.deepEqual(
      stream.commands.map((command) => command.type),
      ["new_session"],
    );
  });

  it("returns an unbound candidate only after reading state for a non-cancelled session", async () => {
    const { stream, session } = fixture();
    stream.handler = (command) => {
      if (command.type === "new_session") stream.respond(command, { cancelled: false });
      else stream.respond(command, { sessionId: "native-session-2", sessionFile: "fixture.jsonl" });
    };
    assert.deepEqual(await session.newSession("parent.jsonl"), {
      status: "candidate",
      sessionId: "native-session-2",
      sessionFile: "fixture.jsonl",
    });
    assert.deepEqual(
      stream.commands.map((command) => command.type),
      ["new_session", "get_state"],
    );
  });

  it("rejects protected mode unless protocol, domain, and context are all qualified", () => {
    const stream = new FakePiStream();
    assert.throws(
      () =>
        new PiManagedSession({
          sink: stream,
          admission: {
            mode: "protected",
            protocolQualified: true,
            protectedDomainQualified: false,
            contextExposure: "POSSIBLE",
          },
        }),
      (error: unknown) =>
        error instanceof PiManagedSessionError && error.code === "PROTECTED_MODE_NOT_QUALIFIED",
    );
  });
});

describe("R4-O-PI passive protection admission", () => {
  it("rejects missing and unknown modes instead of admitting an unrecognized mode", () => {
    const unknownMode = { ...ordinaryAdmission, mode: "future" } as unknown;
    assert.throws(
      () => snapshotPiProtectionAdmission(unknownMode),
      (error: unknown) =>
        error instanceof PiManagedSessionError && error.code === "INVALID_ADMISSION",
    );

    const missingMode: Record<string, unknown> = { ...ordinaryAdmission };
    delete missingMode.mode;
    assert.throws(
      () => snapshotPiProtectionAdmission(missingMode),
      (error: unknown) =>
        error instanceof PiManagedSessionError && error.code === "INVALID_ADMISSION",
    );
  });

  it("rejects a volatile protocol getter without invoking it", () => {
    let reads = 0;
    const volatile = { ...ordinaryAdmission };
    Object.defineProperty(volatile, "protocolQualified", {
      configurable: true,
      enumerable: true,
      get: () => {
        reads += 1;
        return false;
      },
    });
    assert.throws(
      () => snapshotPiProtectionAdmission(volatile),
      (error: unknown) =>
        error instanceof PiManagedSessionError && error.code === "INVALID_ADMISSION",
    );
    assert.equal(reads, 0);
  });

  it("snapshots admission scalars and does not retain a mutable caller alias", () => {
    const input = { ...ordinaryAdmission };
    const snapshot = snapshotPiProtectionAdmission(input);
    input.mode = "protected";
    input.protocolQualified = false;
    input.contextExposure = "POSSIBLE";
    assert.deepEqual(snapshot, ordinaryAdmission);
    assert.equal(Object.isFrozen(snapshot), true);
    assert.equal(Reflect.set(snapshot, "mode", "protected"), false);
    assert.equal(snapshot.mode, "ordinary");
  });

  it("rejects Proxy, symbols, non-enumerable, extra, and nonplain admission records", () => {
    const symbol = Symbol("unexpected");
    const symbolRecord = { ...ordinaryAdmission, [symbol]: true };
    const nonEnumerable = { ...ordinaryAdmission };
    Object.defineProperty(nonEnumerable, "mode", {
      configurable: true,
      enumerable: false,
      value: "ordinary",
    });
    const invalid: ReadonlyArray<unknown> = [
      new Proxy({ ...ordinaryAdmission }, {}),
      symbolRecord,
      nonEnumerable,
      { ...ordinaryAdmission, extra: true },
      Object.create(null),
      new Date(),
    ];
    for (const value of invalid) {
      assert.throws(
        () => snapshotPiProtectionAdmission(value),
        (error: unknown) =>
          error instanceof PiManagedSessionError && error.code === "INVALID_ADMISSION",
      );
    }
  });
});

describe("R4-O-PI pause and queue races", () => {
  it("closes admission before serialized clear_queue then abort and does not claim process stop", async () => {
    const { stream, session } = fixture();
    let releaseClear: (() => void) | undefined;
    const clearSeen = new Promise<void>((resolve) => {
      releaseClear = resolve;
    });
    stream.handler = async (command) => {
      if (command.type === "clear_queue") {
        await clearSeen;
        stream.respond(command, { steering: ["queued steer"], followUp: ["queued followup"] });
      } else if (command.type === "abort") {
        stream.respond(command);
      }
    };
    const paused = session.pause();
    assert.equal(session.dispatchOpen, false);
    assert.throws(
      () => session.resumeDispatch(),
      (error: unknown) =>
        error instanceof PiManagedSessionError && error.code === "PAUSE_IN_PROGRESS",
    );
    await assert.rejects(
      session.followUp("must not revive"),
      (error: unknown) =>
        error instanceof PiManagedSessionError && error.code === "DISPATCH_PAUSED",
    );
    releaseClear?.();
    assert.deepEqual(await paused, {
      admission: "paused",
      cleared: { steering: ["queued steer"], followUp: ["queued followup"] },
      sessionIdle: true,
      processStopped: false,
    });
    assert.deepEqual(
      stream.commands.map((command) => command.type),
      ["clear_queue", "abort"],
    );
    session.resumeDispatch();
    assert.equal(session.dispatchOpen, true);
  });
});

describe("R4-O-PI correlation diagnostics", () => {
  it("diagnoses a duplicate response ID without resolving it twice", async () => {
    const diagnostics: PiRpcDiagnostic[] = [];
    const { stream, session } = fixture({ diagnostics });
    stream.handler = (command) => {
      stream.respond(command);
      stream.respond(command);
    };
    await session.steer("once");
    assert.deepEqual(
      diagnostics.map((entry) => entry.code),
      ["DUPLICATE_RESPONSE_ID"],
    );
  });
});
