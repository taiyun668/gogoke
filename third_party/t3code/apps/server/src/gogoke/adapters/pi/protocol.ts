export const PI_RPC_PROTOCOL_BASELINE = Object.freeze({
  repository: "earendil-works/pi",
  commit: "d1230ea2000d876b479a69b8b061f9d670f262f5",
  documentation: Object.freeze([
    "packages/coding-agent/docs/rpc.md",
    "packages/coding-agent/docs/security.md",
  ]),
  launchArguments: Object.freeze(["--mode", "rpc"]),
  framing: "strict-lf-jsonl",
  transportOwnership: "external-managed-host",
} as const);
