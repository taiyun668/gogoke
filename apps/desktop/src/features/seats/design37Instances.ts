export const DESIGN37_INSTANCES_SCHEMA = "gogoke.37.instance-page.v1";
export const DESIGN37_TEST_INSTANCE_ID = "codexTestM1";

export type Design37InstanceState =
  | "NOT_INSTALLED"
  | "NOT_LOGGED_IN"
  | "LOGGED_IN"
  | "ERROR";

export type Design37LoginState =
  | "PENDING"
  | "LOGGED_IN"
  | "LOGGED_OUT"
  | "UNKNOWN"
  | "CANCELLED"
  | "ERROR";

export type Design37LoginSnapshot = {
  expectedRevision: number;
  state: Design37LoginState;
  output: string;
  authorizationUrl?: string;
  deviceCode?: string;
  error?: string;
  browserState: "NOT_REQUESTED" | "OPENED" | "FAILED";
  startedAt: number;
  settled: boolean;
};

export type Design37Instance = {
  instanceId: string;
  driverId: string;
  version: string;
  newVersion?: string;
  runtimeIssues?: Design37RuntimeIssue[];
  revision: string;
  state: Design37InstanceState;
  login?: Design37LoginSnapshot;
};

export type Design37RuntimeIssue = {
  seatId: string;
  sessionId: string;
  generation: string;
  reason: string;
  sourceEpoch: string;
  sourceCursor: string;
};

export type Design37InstancesSnapshot = {
  schema: typeof DESIGN37_INSTANCES_SCHEMA;
  instances: Design37Instance[];
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isOneOf<T extends string>(value: unknown, allowed: readonly T[]): value is T {
  return typeof value === "string" && allowed.includes(value as T);
}

function readRuntimeIssues(value: unknown, instanceId: string): Design37RuntimeIssue[] {
  if (!Array.isArray(value)) throw new Error(`Instance ${instanceId} returned invalid runtime issues.`);
  return value.map((issue: unknown): Design37RuntimeIssue => {
    if (!isRecord(issue) ||
        ["seatId", "sessionId", "generation", "reason", "sourceEpoch", "sourceCursor"].some(
          field => typeof issue[field] !== "string" || (issue[field] as string).trim().length === 0)) {
      throw new Error(`Instance ${instanceId} returned an invalid runtime issue.`);
    }
    return {
      seatId: issue.seatId as string,
      sessionId: issue.sessionId as string,
      generation: issue.generation as string,
      reason: issue.reason as string,
      sourceEpoch: issue.sourceEpoch as string,
      sourceCursor: issue.sourceCursor as string,
    };
  });
}

const INSTANCE_STATES: readonly Design37InstanceState[] = [
  "NOT_INSTALLED",
  "NOT_LOGGED_IN",
  "LOGGED_IN",
  "ERROR",
];
const LOGIN_STATES: readonly Design37LoginState[] = [
  "PENDING",
  "LOGGED_IN",
  "LOGGED_OUT",
  "UNKNOWN",
  "CANCELLED",
  "ERROR",
];
const BROWSER_STATES = ["NOT_REQUESTED", "OPENED", "FAILED"] as const;

function readLogin(value: unknown, instanceId: string): Design37LoginSnapshot {
  if (!isRecord(value) ||
      typeof value.requestId !== "string" ||
      !Number.isSafeInteger(value.expectedRevision) ||
      !isOneOf(value.state, LOGIN_STATES) ||
      typeof value.output !== "string" ||
      typeof value.settled !== "boolean" ||
      !isOneOf(value.browserState, BROWSER_STATES) ||
      typeof value.startedAt !== "number" || !Number.isFinite(value.startedAt) ||
      (value.authorizationUrl !== undefined && typeof value.authorizationUrl !== "string") ||
      (value.deviceCode !== undefined && typeof value.deviceCode !== "string") ||
      (value.error !== undefined && typeof value.error !== "string")) {
    throw new Error(`Instance ${instanceId} returned an invalid login snapshot.`);
  }

  return {
    expectedRevision: value.expectedRevision as number,
    state: value.state,
    output: value.output,
    browserState: value.browserState,
    startedAt: value.startedAt,
    settled: value.settled,
    ...(value.authorizationUrl === undefined ? {} : { authorizationUrl: value.authorizationUrl }),
    ...(value.deviceCode === undefined ? {} : { deviceCode: value.deviceCode }),
    ...(value.error === undefined ? {} : { error: value.error }),
  };
}

export function readDesign37InstancesSnapshot(value: unknown): Design37InstancesSnapshot {
  if (!isRecord(value) || value.schema !== DESIGN37_INSTANCES_SCHEMA || !Array.isArray(value.instances)) {
    throw new Error("Instance page returned an unexpected schema.");
  }

  const seen = new Set<string>();
  const instances = value.instances.map((item: unknown): Design37Instance => {
    if (!isRecord(item) ||
        typeof item.instanceId !== "string" || item.instanceId.length === 0 ||
        typeof item.driverId !== "string" ||
        typeof item.version !== "string" ||
        (item.newVersion !== undefined && (typeof item.newVersion !== "string" || item.newVersion.length === 0)) ||
        (item.runtimeIssues !== undefined && !Array.isArray(item.runtimeIssues)) ||
        typeof item.revision !== "string" ||
        !isOneOf(item.state, INSTANCE_STATES) ||
        (item.login !== undefined && item.login !== null && !isRecord(item.login))) {
      throw new Error("Instance page returned an invalid instance record.");
    }
    if (seen.has(item.instanceId)) {
      throw new Error(`Instance page returned duplicate instance ${item.instanceId}.`);
    }
    seen.add(item.instanceId);

    return {
      instanceId: item.instanceId,
      driverId: item.driverId,
      version: item.version,
      ...(item.newVersion === undefined ? {} : { newVersion: item.newVersion }),
      ...(item.runtimeIssues === undefined || item.runtimeIssues.length === 0
        ? {} : { runtimeIssues: readRuntimeIssues(item.runtimeIssues, item.instanceId) }),
      revision: item.revision,
      state: item.state,
      ...(item.login == null ? {} : { login: readLogin(item.login, item.instanceId) }),
    };
  });

  return { schema: DESIGN37_INSTANCES_SCHEMA, instances };
}

export function design37InstanceStateLabel(state: Design37InstanceState): string {
  switch (state) {
    case "NOT_INSTALLED": return "未安装";
    case "NOT_LOGGED_IN": return "未登录";
    case "LOGGED_IN": return "已登录";
    case "ERROR": return "出错";
  }
}

export function design37LoginStateLabel(state: Design37LoginState): string {
  switch (state) {
    case "PENDING": return "正在登录";
    case "LOGGED_IN": return "已登录";
    case "LOGGED_OUT": return "已退出登录";
    case "UNKNOWN": return "登录状态未知";
    case "CANCELLED": return "已取消";
    case "ERROR": return "登录出错";
  }
}
