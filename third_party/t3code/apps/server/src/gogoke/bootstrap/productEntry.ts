// @effect-diagnostics nodeBuiltinImport:off - executable product boundary owns stdin/stdout.
import * as NodeFS from "node:fs";

import { constructGogokeService } from "./index.ts";
import { parseStrictJsonBytes } from "../contracts/strictJson.ts";
import { readGitHubFact } from "../context/repository/gitFact.ts";

const R2_02_TEST_LEDGER_REPOSITORY = "taiyun668/gogoke";

export interface ProductLedgerReference {
  readonly repository: string;
  readonly commit: string;
  readonly path: string;
  readonly contentHash: string;
}

export interface ProductGoalRequest {
  readonly goal: { readonly id: string; readonly title: string };
  readonly ledger: ProductLedgerReference;
}

export interface ProductGoalView extends ProductGoalRequest {
  readonly caller: {
    readonly policyRevision: string;
    readonly principalId: string;
    readonly profileId: string;
    readonly admitted: true;
    readonly revocationHead: string;
    readonly role: "controller";
    readonly seatId: string;
  };
  readonly nativeHost: { readonly reachable: true; readonly elapsedMicros: number };
  readonly ledgerReadback: {
    readonly state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED";
    readonly gitBlob: string;
  };
  readonly acceptance: "TEST_FIXTURE_NOT_ADOPTED";
}

const invalid = (path: string, detail: string): never => {
  throw new Error(`INVALID_PRODUCT_ENTRY: ${path} ${detail}`);
};

const exactRecord = (
  value: unknown,
  path: string,
  keys: readonly string[],
): Record<string, unknown> => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return invalid(path, "must be an object");
  }
  const record = value as Record<string, unknown>;
  const own = Reflect.ownKeys(record);
  if (
    own.length !== keys.length ||
    own.some((key) => typeof key !== "string" || !keys.includes(key)) ||
    keys.some((key) => !Object.hasOwn(record, key))
  ) {
    return invalid(path, "fields are invalid");
  }
  return record;
};

const text = (value: unknown, path: string): string => {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value !== value.trim() ||
    value.length > 512
  ) {
    return invalid(path, "must be canonical text");
  }
  return value;
};

export function decodeProductGoalRequest(bytes: Uint8Array): ProductGoalRequest {
  const root = exactRecord(parseStrictJsonBytes(bytes), "request", ["goal", "ledger"]);
  const goal = exactRecord(root.goal, "request.goal", ["id", "title"]);
  const ledger = exactRecord(root.ledger, "request.ledger", [
    "repository",
    "commit",
    "path",
    "contentHash",
  ]);
  const repository = text(ledger.repository, "request.ledger.repository");
  const commit = text(ledger.commit, "request.ledger.commit");
  const path = text(ledger.path, "request.ledger.path");
  const contentHash = text(ledger.contentHash, "request.ledger.contentHash");
  if (!/^[0-9a-f]{40}(?:[0-9a-f]{24})?$/.test(commit)) {
    return invalid("request.ledger.commit", "must be an immutable Git object id");
  }
  if (
    path.startsWith("/") ||
    path.includes("\\") ||
    path.split("/").some((part) => part === "" || part === "." || part === "..")
  ) {
    return invalid("request.ledger.path", "must be a canonical repository-relative path");
  }
  if (!/^(?:sha256:)?[0-9a-f]{64}$/.test(contentHash)) {
    return invalid("request.ledger.contentHash", "must be a SHA-256 content hash");
  }
  return Object.freeze({
    goal: Object.freeze({
      id: text(goal.id, "request.goal.id"),
      title: text(goal.title, "request.goal.title"),
    }),
    ledger: Object.freeze({ repository, commit, path, contentHash }),
  });
}

export function parseProductProcessArgs(
  argv: readonly string[],
): { readonly root: string; readonly hostBinary: string } {
  if (argv.length !== 4 || argv[0] !== "--root" || argv[2] !== "--native-host") {
    return invalid("argv", "expected exactly --root <path> --native-host <path>");
  }
  return Object.freeze({
    root: text(argv[1], "argv.root"),
    hostBinary: text(argv[3], "argv.nativeHost"),
  });
}

export async function handleProductGoalRequest(
  request: ProductGoalRequest,
  paths: { readonly root: string; readonly hostBinary: string },
): Promise<ProductGoalView> {
  const service = await constructGogokeService({
    request: {
      authority: "public",
      requestedCapabilities: ["local-non-model"],
      enabledRuntimeDriverIds: [],
    },
    root: paths.root,
    hostBinary: paths.hostBinary,
  });
  try {
    if (typeof service.store.admitControllerCaller !== "function") {
      throw new Error("PRODUCT_CALLER_ADMISSION_UNAVAILABLE");
    }
    const admission = await service.store.admitControllerCaller({
      policyRevision: service.identity.policyRevision,
      principalId: service.identity.principalId,
      profileId: service.identity.profileId,
      revocationHead: service.identity.revocationHead,
      role: "controller",
      seatId: service.identity.seatId,
    });
    // This construction Goal is test-only. The repository scope comes from
    // Owner's R2-02 authorization, not from the caller's ledger field.
    const ledgerReadback = await readGitHubFact(request.ledger, R2_02_TEST_LEDGER_REPOSITORY);
    return Object.freeze({
      goal: request.goal,
      ledger: request.ledger,
      caller: Object.freeze({
        admitted: admission.admitted,
        policyRevision: admission.policyRevision,
        principalId: admission.principalId,
        profileId: admission.profileId,
        revocationHead: admission.revocationHead,
        role: admission.role,
        seatId: admission.seatId,
      }),
      nativeHost: Object.freeze({
        reachable: true as const,
        elapsedMicros: admission.elapsedMicros,
      }),
      ledgerReadback: Object.freeze({ state: ledgerReadback.state, gitBlob: ledgerReadback.gitBlob }),
      acceptance: "TEST_FIXTURE_NOT_ADOPTED" as const,
    });
  } finally {
    await service.close();
  }
}

export async function runGogokeProductProcess(argv: readonly string[]): Promise<void> {
  const paths = parseProductProcessArgs(argv);
  const input = NodeFS.readFileSync(0);
  const request = decodeProductGoalRequest(input);
  const response = await handleProductGoalRequest(request, paths);
  // @effect-diagnostics-next-line preferSchemaOverJson:off - local process response DTO.
  NodeFS.writeFileSync(1, `${JSON.stringify(response)}\n`);
}
