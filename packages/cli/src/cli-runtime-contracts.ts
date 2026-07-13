import type {
  ReceiptContract,
  ClosureDispositionContract,
  ResolutionRequestContract as ResolutionRequest,
  ResolutionResponseContract as ResolutionResponse,
} from "@runxhq/contracts";

export interface ExecutionEvent {
  readonly type:
    | "skill_loaded"
    | "inputs_resolved"
    | "auth_resolved"
    | "resolution_requested"
    | "resolution_resolved"
    | "admitted"
    | "executing"
    | "step_started"
    | "step_waiting_resolution"
    | "step_completed"
    | "warning"
    | "completed";
  readonly message: string;
  readonly data?: unknown;
}

export interface Caller {
  readonly resolve: (request: ResolutionRequest) => Promise<ResolutionResponse | undefined>;
  readonly report: (event: ExecutionEvent) => void | Promise<void>;
}

export type RegistryTrustTier = "first_party" | "verified" | "community";

export interface RegistrySkillVersion {
  readonly markdown: string;
  readonly profile_document?: string;
  readonly profile_digest?: string;
  readonly runner_names: readonly string[];
  readonly skill_id: string;
  readonly name: string;
  readonly category?: string;
  readonly source_category?: string;
  readonly version: string;
  readonly digest: string;
  readonly source_type: string;
  readonly trust_tier: RegistryTrustTier;
}

export interface RegistrySkill {
  readonly skill_id: string;
  readonly name: string;
}

export interface PutVersionOptions {
  readonly upsert?: boolean;
}

export interface RegistryStore {
  readonly getVersion: (
    skillId: string,
    version?: string,
  ) => Promise<RegistrySkillVersion | undefined>;
  readonly listVersions: (skillId: string) => Promise<readonly RegistrySkillVersion[]>;
  readonly listSkills?: () => Promise<readonly RegistrySkill[]>;
  readonly putVersion?: (
    version: RegistrySkillVersion,
    options?: PutVersionOptions,
  ) => Promise<RegistrySkillVersion>;
}

export interface RunLineageMetadata {
  readonly kind: string;
  readonly sourceRunId: string;
  readonly sourceReceiptId?: string;
}

interface CliReceiptRuntimeMetadata {
  readonly outcome_state?: string;
  readonly duration_ms?: number;
  readonly steps?: readonly unknown[];
}

export type CliRuntimeReceipt = Partial<ReceiptContract> & {
  readonly id: string;
  readonly schema: string;
  readonly seal?: {
    readonly disposition?: ClosureDispositionContract | string;
    readonly closed_at?: string;
    readonly [key: string]: unknown;
  };
  readonly metadata?: {
    readonly runx?: CliReceiptRuntimeMetadata;
    readonly [key: string]: unknown;
  };
  readonly duration_ms?: number;
};

export type CliSkillRunResult =
  | {
      readonly status: "needs_operator_approval";
      readonly skill: { readonly name: string };
      readonly skillPath: string;
      readonly digest: string;
      readonly approvalFlag: string;
    }
  | {
      readonly status: "needs_agent";
      readonly skill: { readonly name: string };
      readonly skillPath: string;
      readonly runId: string;
      readonly stepIds?: readonly string[];
      readonly stepLabels?: readonly string[];
      readonly requests: readonly ResolutionRequest[];
    }
  | {
      readonly status: "policy_denied";
      readonly skill: { readonly name: string };
      readonly reasons: readonly string[];
      readonly approval?: {
        readonly gate: {
          readonly id: string;
          readonly type?: string;
          readonly reason: string;
          readonly summary?: unknown;
        };
        readonly approved: boolean;
      };
      readonly receipt?: CliRuntimeReceipt;
    }
  | {
      readonly status: "sealed" | "failure";
      readonly skill: { readonly name: string };
      readonly inputs?: Readonly<Record<string, unknown>>;
      readonly execution: {
        readonly stdout: string;
        readonly stderr: string;
        readonly errorMessage?: string;
        readonly [key: string]: unknown;
      };
      readonly receipt: CliRuntimeReceipt;
      readonly [key: string]: unknown;
    };

export function runnerReceiptDisposition(
  receipt: CliRuntimeReceipt,
): ClosureDispositionContract | string {
  return receipt.seal?.disposition ?? "failed";
}

export function runnerReceiptDurationMs(receipt: CliRuntimeReceipt): number | undefined {
  return runtimeMetadata(receipt).duration_ms ?? receipt.duration_ms;
}

export function runnerReceiptGraphSteps(receipt: CliRuntimeReceipt): readonly unknown[] {
  return runtimeMetadata(receipt).steps ?? [];
}

export function runnerReceiptOutcomeState(receipt: CliRuntimeReceipt): string | undefined {
  return runtimeMetadata(receipt).outcome_state;
}

function runtimeMetadata(receipt: CliRuntimeReceipt): CliReceiptRuntimeMetadata {
  const runx = receipt.metadata?.runx;
  return runx && typeof runx === "object" ? runx : {};
}
