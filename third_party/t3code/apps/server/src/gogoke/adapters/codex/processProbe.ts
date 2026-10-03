/**
 * Local protocol probe for the real pinned Codex app-server. This is not the
 * production process owner: H owns its Windows Job, admission and durable
 * request journal. The probe uses an empty disposable CODEX_HOME and never
 * sends a turn/start, approval or authentication request.
 */
import { execFile, spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, mkdtemp, readFile, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, isAbsolute, join, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { parseStrictJsonBytes } from "../../contracts/strictJson.ts";
import { codexMemoryOffAppServerArgs } from "./adapter.ts";
import { CodexJsonlDecoder, CodexProtocolError, type CodexFrame } from "./protocol.ts";
import type { CodexSessionTransport } from "./session.ts";

type Pending = { resolve: (value: unknown) => void; reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout> };
type RecordValue = Record<string, unknown>;
const execFileAsync = promisify(execFile);
const record = (value: unknown): value is RecordValue =>
  value !== null && typeof value === "object" && !Array.isArray(value);

export interface ProbeLaunch {
  readonly executable: string;
  readonly expectedSha256: string;
  readonly home: string;
  readonly cwd: string;
}

/** A digest is an observed file fact, not an OS launch identity proof. */
export async function sha256OfRegularFile(path: string): Promise<string> {
  const before = await lstat(path);
  if (!before.isFile() || before.isSymbolicLink()) throw new CodexProtocolError("PROGRAM_IDENTITY", "non-regular program");
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  const after = await lstat(path);
  if (!after.isFile() || after.isSymbolicLink() || before.size !== after.size ||
      before.mtimeMs !== after.mtimeMs || before.ino !== after.ino)
    throw new CodexProtocolError("PROGRAM_IDENTITY", "program changed while hashing");
  return hash.digest("hex");
}

/** Explicit child environment: no inherited API keys, auth tokens or home. */
export function isolatedProbeEnvironment(home: string): NodeJS.ProcessEnv {
  if (!isAbsolute(home)) throw new CodexProtocolError("INVALID_INPUT", "probe home");
  const windows = process.env.SystemRoot ?? process.env.WINDIR;
  if (!windows || !isAbsolute(windows)) throw new CodexProtocolError("INVALID_ENV", "Windows root");
  return {
    SystemRoot: windows, WINDIR: windows,
    PATH: join(windows, "System32"),
    CODEX_HOME: home, USERPROFILE: home,
    APPDATA: home, LOCALAPPDATA: home,
    TEMP: home, TMP: home,
  };
}

export function effectiveMemoryOff(response: unknown): boolean {
  if (!record(response) || !record(response.config)) return false;
  const effective = response.config;
  const features = record(effective.features) ? effective.features : null;
  const memories = record(effective.memories) ? effective.memories : null;
  return features?.memories === false && memories?.generate_memories === false &&
    memories?.use_memories === false;
}

export class CodexStdioProbeTransport implements CodexSessionTransport {
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly decoder: CodexJsonlDecoder;
  private stderrTail = "";
  private dead = false;
  readonly unsolicited: CodexFrame[] = [];
  readonly parseErrors: string[] = [];

  private constructor(child: ChildProcessWithoutNullStreams) {
    this.child = child;
    this.decoder = new CodexJsonlDecoder((frame) => this.received(frame), (error) => {
      this.parseErrors.push(error.code);
      this.failAll(error);
    });
    child.stdout.on("data", (chunk: Buffer) => this.decoder.push(chunk));
    child.stderr.on("data", (chunk: Buffer) => {
      this.stderrTail = (this.stderrTail + chunk.toString("utf8")).slice(-64 * 1024);
    });
    child.on("error", (error) => this.failAll(error));
    child.on("exit", (code, signal) => {
      this.decoder.finish();
      const error = new CodexProtocolError("PROCESS_EXIT", `code=${code} signal=${signal}`);
      Object.assign(error, { stderrTail: this.stderrTail });
      this.failAll(error);
    });
  }

  static async launch(spec: ProbeLaunch): Promise<CodexStdioProbeTransport> {
    if (!isAbsolute(spec.executable) || basename(spec.executable).toLowerCase() !== "codex.exe" ||
        !isAbsolute(spec.cwd) || !isAbsolute(spec.home) || !/^[a-f0-9]{64}$/i.test(spec.expectedSha256))
      throw new CodexProtocolError("INVALID_INPUT", "probe launch identity");
    const actual = await sha256OfRegularFile(spec.executable);
    if (actual.toLowerCase() !== spec.expectedSha256.toLowerCase())
      throw new CodexProtocolError("PROGRAM_IDENTITY", "digest mismatch before spawn");
    const args = [...codexMemoryOffAppServerArgs(), "--strict-config", "--stdio"];
    const child = spawn(spec.executable, args, {
      cwd: spec.cwd, env: isolatedProbeEnvironment(spec.home),
      windowsHide: true, stdio: ["pipe", "pipe", "pipe"],
    });
    return new CodexStdioProbeTransport(child);
  }

  private received(frame: CodexFrame): void {
    if (frame.kind !== "response") {
      if (this.unsolicited.length >= 1024) {
        this.failAll(new CodexProtocolError("EVENT_OVERFLOW", "probe event limit"));
        return;
      }
      this.unsolicited.push(frame);
      return;
    }
    if (typeof frame.id !== "number" || !Number.isSafeInteger(frame.id)) {
      this.failAll(new CodexProtocolError("INVALID_ID", "non-numeric response ID"));
      return;
    }
    const pending = this.pending.get(frame.id);
    if (!pending) return; // Late response is not a new receipt.
    this.pending.delete(frame.id);
    clearTimeout(pending.timeout);
    if ("error" in frame && frame.error !== undefined) {
      const error = new CodexProtocolError("RPC_ERROR", JSON.stringify(frame.error));
      pending.reject(error);
    } else pending.resolve(frame.result);
  }

  private failAll(error: Error): void {
    if (this.dead) return;
    this.dead = true;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(error);
    }
    this.pending.clear();
  }

  private async send(value: RecordValue): Promise<void> {
    if (this.dead) throw new CodexProtocolError("PROCESS_EXIT", "probe process unavailable");
    const line = `${JSON.stringify(value)}\n`;
    await new Promise<void>((resolve, reject) => {
      this.child.stdin.write(line, (error) => error ? reject(error) : resolve());
    });
  }

  async request(method: string, params: Readonly<RecordValue>): Promise<unknown> {
    if (this.dead) throw new CodexProtocolError("PROCESS_EXIT", "probe process unavailable");
    const id = this.nextId++;
    const response = new Promise<unknown>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new CodexProtocolError("REQUEST_TIMEOUT", method));
        this.failAll(new CodexProtocolError("REQUEST_TIMEOUT", method));
      }, 10_000);
      this.pending.set(id, { resolve, reject, timeout });
    });
    try { await this.send({ id, method, params }); }
    catch (error) {
      this.failAll(error instanceof Error ? error : new Error(String(error)));
      throw error;
    }
    return response;
  }

  async notify(method: "initialized", params?: Readonly<RecordValue>): Promise<void> {
    await this.send(params === undefined ? { method } : { method, params });
  }

  async close(): Promise<void> {
    if (this.child.exitCode === null) this.child.kill();
    this.failAll(new CodexProtocolError("PROBE_CLOSED", "probe closed"));
    await new Promise<void>((resolve) => {
      if (this.child.exitCode !== null) return resolve();
      const timeout = setTimeout(resolve, 5_000);
      this.child.once("exit", () => { clearTimeout(timeout); resolve(); });
    });
  }
}

export interface ProbeResult {
  readonly installedVersion: string;
  readonly executableSha256: string;
  readonly versionOutput: string;
  readonly serverUserAgent?: string;
  readonly initialize: "PASS" | "FAIL";
  readonly configRead: "PASS" | "FAIL";
  readonly memoryOff: "PASS" | "NOT_VERIFIED";
  readonly threadStart: "PASS" | "NOT_RUN" | "FAIL";
  readonly resume: "PASS" | "NOT_RUN" | "FAIL";
  readonly appendWithoutTurn: "ACK_NO_TURN_OBSERVED_1S" | "NOT_RUN" | "FAIL";
  readonly nativeQuestionCard: "NOT_RUN";
  readonly steer: "NOT_RUN";
  readonly compact: "NOT_RUN";
  readonly reason?: string;
}

/** Inspect the installed npm package from this user's standard layout only. */
export async function installedCodexProgram(): Promise<{ executable: string; version: string; sha256: string }> {
  const appData = process.env.APPDATA;
  if (!appData || !isAbsolute(appData)) throw new CodexProtocolError("INVALID_ENV", "APPDATA");
  const root = join(appData, "npm", "node_modules", "@openai", "codex");
  const rootPackage = parseStrictJsonBytes(await readFile(join(root, "package.json")));
  const platform = join(root, "node_modules", "@openai", "codex-win32-x64");
  const platformPackage = parseStrictJsonBytes(await readFile(join(platform, "package.json")));
  if (!record(rootPackage) || !record(platformPackage) ||
      rootPackage.name !== "@openai/codex" || rootPackage.version !== "0.160.0" ||
      platformPackage.name !== "@openai/codex" || platformPackage.version !== "0.160.0-win32-x64")
    throw new CodexProtocolError("VERSION_MISMATCH", "installed npm package is not pinned 0.160.0");
  const executable = join(platform, "vendor", "x86_64-pc-windows-msvc", "bin", "codex.exe");
  return { executable, version: "0.160.0", sha256: await sha256OfRegularFile(executable) };
}

const errorCode = (error: unknown): string =>
  error instanceof CodexProtocolError ? `${error.code}: ${error.message.slice(0, 500)}` :
  error instanceof Error ? error.name : "UNKNOWN_ERROR";

/** Disposable local proof; optional thread path still never starts a model turn. */
export async function probeInstalledCodex(threadProbe = false): Promise<ProbeResult> {
  const installed = await installedCodexProgram();
  const home = await mkdtemp(join(tmpdir(), "gogoke-codex-probe-"));
  let transport: CodexStdioProbeTransport | undefined;
  let result: ProbeResult = {
    installedVersion: installed.version, executableSha256: installed.sha256,
    versionOutput: "NOT_RUN",
    initialize: "FAIL", configRead: "FAIL", memoryOff: "NOT_VERIFIED",
    threadStart: "NOT_RUN", resume: "NOT_RUN", appendWithoutTurn: "NOT_RUN",
    nativeQuestionCard: "NOT_RUN", steer: "NOT_RUN", compact: "NOT_RUN",
  };
  try {
    const versionProcess = await execFileAsync(installed.executable, ["--version"], {
      cwd: home, env: isolatedProbeEnvironment(home), windowsHide: true,
      timeout: 5_000, maxBuffer: 4_096,
    });
    if (versionProcess.stdout.trim() !== "codex-cli 0.160.0")
      throw new CodexProtocolError("VERSION_MISMATCH", "binary --version differs from package");
    result = { ...result, versionOutput: versionProcess.stdout.trim() };
    transport = await CodexStdioProbeTransport.launch({
      executable: installed.executable, expectedSha256: installed.sha256, home, cwd: home,
    });
    const initialized = await transport.request("initialize", {
      clientInfo: { name: "gogoke-probe", title: "gogoke probe", version: "0.0.0" },
      capabilities: { experimentalApi: true },
    });
    if (!record(initialized)) throw new CodexProtocolError("INVALID_RESPONSE", "initialize");
    await transport.notify("initialized");
    result = { ...result, initialize: "PASS",
      ...(typeof initialized.userAgent === "string"
        ? { serverUserAgent: initialized.userAgent } : {}) };
    const config = await transport.request("config/read", { cwd: home, includeLayers: true });
    if (!record(config) || !record(config.config)) throw new CodexProtocolError("INVALID_RESPONSE", "config/read");
    const effective = config.config;
    result = {
      ...result, configRead: "PASS",
      memoryOff: effectiveMemoryOff(config) ? "PASS" : "NOT_VERIFIED",
    };
    if (!threadProbe) return { ...result, reason: "Read-only probe: no thread or turn started." };
    if (result.memoryOff !== "PASS")
      return { ...result, reason: "Effective memory-off configuration was not verified; thread probe skipped." };
    let threadId: string;
    try {
      const started = await transport.request("thread/start", {
        cwd: home, sandbox: "read-only",
      });
      if (!record(started) || !record(started.thread) || typeof started.thread.id !== "string" ||
          started.thread.id.length === 0)
        throw new CodexProtocolError("INVALID_RESPONSE", "thread/start.thread.id");
      threadId = started.thread.id;
      result = { ...result, threadStart: "PASS" };
    } catch (error) {
      return { ...result, threadStart: "FAIL", reason: `thread/start: ${errorCode(error)}` };
    }
    try {
      const before = transport.unsolicited.filter((event) =>
        event.kind === "notification" && event.method === "turn/started").length;
      const appended = await transport.request("thread/inject_items", { threadId,
        items: [{ type: "message", role: "user", content: [{ type: "input_text",
          text: "Probe reference only; no response requested." }] }],
      });
      if (!record(appended) || Object.keys(appended).length !== 0)
        throw new CodexProtocolError("INVALID_RESPONSE", "thread/inject_items must ACK with {}");
      await new Promise((resolve) => setTimeout(resolve, 1_000));
      const after = transport.unsolicited.filter((event) =>
        event.kind === "notification" && event.method === "turn/started").length;
      if (after !== before) throw new CodexProtocolError("UNEXPECTED_TURN", "append started a turn");
      result = { ...result, appendWithoutTurn: "ACK_NO_TURN_OBSERVED_1S" };
    } catch (error) {
      return { ...result, appendWithoutTurn: "FAIL", reason: `thread/inject_items: ${errorCode(error)}` };
    }
    try {
      const resumed = await transport.request("thread/resume", { threadId });
      if (!record(resumed) || !record(resumed.thread) || resumed.thread.id !== threadId)
        throw new CodexProtocolError("INVALID_RESPONSE", "thread/resume returned different thread");
      result = { ...result, resume: "PASS" };
    } catch (error) {
      return { ...result, resume: "FAIL", reason: `thread/resume: ${errorCode(error)}` };
    }
    return { ...result,
      reason: "No model turn was started; question card, steer and compaction remain NOT_RUN." };
  } catch (error) {
    return { ...result, reason: `probe: ${errorCode(error)}` };
  } finally {
    if (transport) await transport.close();
    const tempRoot = await realpath(tmpdir());
    const actualHome = await realpath(home);
    if (!actualHome.startsWith(`${tempRoot}${sep}`) ||
        !basename(actualHome).startsWith("gogoke-codex-probe-") ||
        (await lstat(home)).isSymbolicLink())
      throw new CodexProtocolError("PROBE_CLEANUP", "temporary home identity changed");
    await rm(home, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  probeInstalledCodex(process.argv.includes("--thread-probe")).then((result) => {
    process.stdout.write(`${JSON.stringify(result)}\n`);
    if (result.initialize !== "PASS" || result.configRead !== "PASS" ||
        (process.argv.includes("--thread-probe") && (result.threadStart !== "PASS" || result.resume !== "PASS" ||
          result.appendWithoutTurn !== "ACK_NO_TURN_OBSERVED_1S"))) process.exitCode = 1;
  }, (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    process.stdout.write(`${JSON.stringify({ status: "FAIL", reason: message })}\n`);
    process.exitCode = 1;
  });
}
