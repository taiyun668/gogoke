export type PiContextExposure = "OBSERVED_LIMITED" | "POSSIBLE" | "UNKNOWN";

export interface PiProtectionAdmission {
  readonly mode: "ordinary" | "protected";
  readonly protocolQualified: boolean;
  readonly protectedDomainQualified: boolean;
  readonly contextExposure: PiContextExposure;
}

/**
 * The process-owning host supplies this byte sink.  The Pi adapter deliberately
 * has no spawn, account, filesystem, network, or process-custody authority.
 */
export interface PiRpcByteSink {
  write(chunk: Uint8Array): void | Promise<void>;
}

export type PiRpcDiagnosticCode =
  | "INVALID_UTF8"
  | "INVALID_JSON"
  | "INVALID_MESSAGE"
  | "OVERSIZE_FRAME"
  | "PARTIAL_FRAME_EOF"
  | "DUPLICATE_RESPONSE_ID"
  | "UNKNOWN_RESPONSE_ID"
  | "RESPONSE_COMMAND_MISMATCH";

export interface PiRpcDiagnostic {
  readonly code: PiRpcDiagnosticCode;
  readonly detail: string;
}

export interface PiAcceptedCommand {
  /** A successful Pi command response is an ACK, never task completion. */
  readonly status: "accepted";
  readonly requestId: string;
  readonly command: "prompt" | "steer" | "follow_up";
}

export interface PiPauseResult {
  readonly admission: "paused";
  readonly cleared: {
    readonly steering: ReadonlyArray<string>;
    readonly followUp: ReadonlyArray<string>;
  };
  /** Pi's abort response means session idle, not process-tree termination. */
  readonly sessionIdle: true;
  readonly processStopped: false;
}

export type PiNewSessionResult =
  | {
      readonly status: "cancelled";
    }
  | {
      readonly status: "candidate";
      /** This is an unbound candidate. A separate authority must verify/publish it. */
      readonly sessionId: string;
      readonly sessionFile?: string;
    };

export interface PiManagedSessionOptions {
  readonly sink: PiRpcByteSink;
  readonly admission: PiProtectionAdmission;
  readonly maxFrameBytes?: number;
  readonly onDiagnostic?: (diagnostic: PiRpcDiagnostic) => void;
  readonly onEvent?: (event: Readonly<Record<string, unknown>>) => void;
}
