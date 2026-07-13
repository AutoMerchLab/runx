import type { ResolutionRequestContract as ResolutionRequest } from "@runxhq/contracts";

import type { ParsedArgs } from "../args.js";
import {
  runnerReceiptDisposition,
  runnerReceiptDurationMs,
  runnerReceiptGraphSteps,
  runnerReceiptOutcomeState,
  type CliSkillRunResult as RunLocalSkillResult,
} from "../cli-runtime-contracts.js";
import type { CliIo } from "../index.js";
import { shortId, statusIcon, theme } from "../ui.js";
import { isRecord } from "./internal.js";
import { renderNeedsAgent, renderPolicyDenied } from "./needs-agent.js";

interface NeedsAgentSkillResult {
  readonly status: "needs_agent";
  readonly skill: { readonly name: string };
  readonly skillPath: string;
  readonly runId: string;
  readonly stepIds?: readonly string[];
  readonly stepLabels?: readonly string[];
  readonly requests: readonly ResolutionRequest[];
}

export function writeLocalSkillResult(
  io: CliIo,
  env: NodeJS.ProcessEnv,
  parsed: ParsedArgs,
  result: RunLocalSkillResult,
): number {
  if (result.status === "needs_operator_approval") {
    if (parsed.json) {
      io.stdout.write(`${JSON.stringify({
        status: result.status,
        disposition: "approval_required",
        execution_status: null,
        outcome_state: "pending",
        skill: result.skill.name,
        skill_path: result.skillPath,
        digest: result.digest,
        approval_flag: result.approvalFlag,
      }, null, 2)}\n`);
    } else {
      io.stdout.write(
        `Approval required\nRerun the same command with:\n  ${result.approvalFlag}\n`,
      );
    }
    return 2;
  }
  if (isNeedsAgentResult(result)) {
    return writeNeedsAgentResult(io, env, parsed, result);
  }
  if (result.status === "policy_denied") {
    return writePolicyDeniedResult(io, parsed, result);
  }
  const terminalResult = result as Extract<RunLocalSkillResult, { readonly status: "sealed" | "failure" }>;
  if (parsed.json) {
    const disposition = runnerReceiptDisposition(terminalResult.receipt);
    const status = disposition === "blocked" ? "escalated" : terminalResult.status;
    io.stdout.write(
      `${JSON.stringify(
        {
          ...terminalResult,
          status,
          execution_status: terminalResult.status,
          disposition,
          outcome_state: runnerReceiptOutcomeState(terminalResult.receipt) ?? "complete",
        },
        null,
        2,
      )}\n`,
    );
  } else {
    writeRunResult(io, env, terminalResult);
  }
  return terminalResult.status === "sealed" ? 0 : 1;
}

function isNeedsAgentResult(result: RunLocalSkillResult | NeedsAgentSkillResult): result is NeedsAgentSkillResult {
  return (result as { readonly status?: string }).status === "needs_agent";
}

function writeNeedsAgentResult(
  io: CliIo,
  env: NodeJS.ProcessEnv,
  parsed: ParsedArgs,
  result: NeedsAgentSkillResult,
): number {
  const productionMode = env.RUNX_PRODUCTION === "1";
  if (parsed.json) {
    io.stdout.write(
      `${JSON.stringify(
        {
          status: productionMode ? "failure" : "needs_agent",
          disposition: productionMode ? "failure_no_resolver" : "needs_agent",
          execution_status: productionMode ? "failure" : null,
          outcome_state: "pending",
          skill: result.skill.name,
          skill_path: result.skillPath,
          run_id: result.runId,
          step_ids: result.stepIds,
          step_labels: result.stepLabels,
          requests: result.requests,
          ...(productionMode
            ? { failure_reason: "RUNX_PRODUCTION=1 forbids unresolved cognitive-work requests" }
            : {}),
        },
        null,
        2,
      )}\n`,
    );
  } else {
    io.stdout.write(renderNeedsAgent(result, env));
  }
  if (productionMode) {
    const requestIds = result.requests.map((request) => request.id).join(", ");
    io.stderr.write(
      `runx: production run ${result.runId} halted with unresolved cognitive-work request(s): ${requestIds}\n`
      + "  RUNX_PRODUCTION=1 forbids pausing; unset RUNX_PRODUCTION to allow pause/resume semantics.\n",
    );
  }
  return 2;
}

function writePolicyDeniedResult(
  io: CliIo,
  parsed: ParsedArgs,
  result: Extract<RunLocalSkillResult, { readonly status: "policy_denied" }>,
): number {
  if (parsed.json) {
    const approvalRequired = parsed.nonInteractive && result.approval !== undefined;
    const disposition = approvalRequired ? "approval_required" : (result.receipt ? runnerReceiptDisposition(result.receipt) : "declined");
    const executionStatus = approvalRequired ? null : "failure";
    const outcomeState = approvalRequired ? "pending" : (result.receipt ? runnerReceiptOutcomeState(result.receipt) ?? "complete" : "complete");
    io.stdout.write(
      `${JSON.stringify(
        {
          status: approvalRequired ? "approval_required" : "policy_denied",
          execution_status: executionStatus,
          disposition,
          outcome_state: outcomeState,
          skill: result.skill.name,
          reasons: result.reasons,
          approval: result.approval
            ? {
                gate_id: result.approval.gate.id,
                gate_type: result.approval.gate.type ?? "unspecified",
                reason: result.approval.gate.reason,
                summary: result.approval.gate.summary,
                decision: result.approval.approved ? "approved" : "denied",
              }
            : undefined,
          receipt_id: result.receipt?.id,
        },
        null,
        2,
      )}\n`,
    );
    return approvalRequired ? 2 : 1;
  }
  io.stderr.write(renderPolicyDenied(result.skill.name, result.reasons, result.receipt
    ? {
        disposition: runnerReceiptDisposition(result.receipt),
        outcome_state: runnerReceiptOutcomeState(result.receipt),
      }
    : undefined));
  return 1;
}

function writeRunResult(
  io: CliIo,
  env: NodeJS.ProcessEnv,
  result: {
    readonly status: "sealed" | "failure";
    readonly skill: { readonly name: string };
    readonly execution: { readonly stdout: string; readonly stderr: string; readonly errorMessage?: string };
    readonly receipt: NonNullable<Extract<RunLocalSkillResult, { readonly status: "sealed" | "failure" }>["receipt"]>;
  },
): void {
  if (result.status === "sealed") {
    io.stdout.write(renderRunSuccess(result, io, env));
    return;
  }
  io.stderr.write(renderRunFailure(result, io, env));
}

function renderRunSuccess(
  result: {
    readonly skill: { readonly name: string };
    readonly execution: { readonly stdout: string };
    readonly receipt: NonNullable<Extract<RunLocalSkillResult, { readonly status: "sealed" | "failure" }>["receipt"]>;
  },
  io: CliIo,
  env: NodeJS.ProcessEnv,
): string {
  const t = theme(io.stdout, env);
  const trimmed = result.execution.stdout.trim();
  let parsedOutput: Record<string, unknown> | undefined;
  try {
    const parsed = JSON.parse(trimmed) as unknown;
    if (isRecord(parsed)) {
      parsedOutput = parsed;
    }
  } catch {
    // Success output is often plain text; only use JSON metadata when it parses cleanly.
  }
  if (result.skill.name === "sourcey" && parsedOutput) {
    const outputDir = typeof parsedOutput.output_dir === "string" ? parsedOutput.output_dir : undefined;
    const indexPath = typeof parsedOutput.index_path === "string" ? parsedOutput.index_path : undefined;
    const verified = typeof parsedOutput.verified === "boolean" ? (parsedOutput.verified ? "passed" : "failed") : undefined;
    const lines = [
      "",
      `  ${statusIcon("sealed", t)}  ${t.bold}sourcey${t.reset}  ${t.dim}site built${t.reset}`,
      `  ${t.dim}receipt${t.reset}   ${shortId(result.receipt.id)}`,
      `  ${t.dim}schema${t.reset}    ${result.receipt.schema}`,
    ];
    const duration = formatDurationMs(runnerReceiptDurationMs(result.receipt));
    if (duration) lines.push(`  ${t.dim}duration${t.reset}  ${duration}`);
    if (outputDir) lines.push(`  ${t.dim}site${t.reset}      ${outputDir}`);
    if (indexPath) lines.push(`  ${t.dim}index${t.reset}     ${indexPath}`);
    if (verified) lines.push(`  ${t.dim}verify${t.reset}    ${verified}`);
    lines.push(`  ${t.dim}history${t.reset}   runx history --json`);
    lines.push("");
    return lines.join("\n");
  }
  const lines = [
    "",
    `  ${statusIcon("sealed", t)}  ${t.bold}${result.skill.name}${t.reset}  ${t.dim}sealed${t.reset}`,
    `  ${t.dim}receipt${t.reset}   ${shortId(result.receipt.id)}`,
    `  ${t.dim}schema${t.reset}    ${result.receipt.schema}`,
  ];
  const duration = formatDurationMs(runnerReceiptDurationMs(result.receipt));
  if (duration) lines.push(`  ${t.dim}duration${t.reset}  ${duration}`);
  lines.push(`  ${t.dim}closure${t.reset}   ${runnerReceiptDisposition(result.receipt)}`);
  const outcomeState = runnerReceiptOutcomeState(result.receipt);
  if (outcomeState) lines.push(`  ${t.dim}outcome${t.reset}      ${outcomeState}`);
  const steps = runnerReceiptGraphSteps(result.receipt);
  if (steps.length > 0) lines.push(`  ${t.dim}steps${t.reset}     ${steps.length}`);
  const highlights = extractOutputHighlights(result.execution.stdout);
  for (const [label, value] of highlights) {
    lines.push(`  ${t.dim}${label}${t.reset}  ${value}`);
  }
  if (highlights.length === 0 && result.execution.stdout.trim()) {
    lines.push(`  ${t.dim}output${t.reset}    ${truncateMultiline(result.execution.stdout, 6)}`);
  }
  lines.push(`  ${t.dim}history${t.reset}   runx history --json`);
  lines.push("");
  return lines.join("\n");
}

function renderRunFailure(
  result: {
    readonly skill: { readonly name: string };
    readonly execution: { readonly stdout: string; readonly stderr: string; readonly errorMessage?: string };
    readonly receipt: NonNullable<Extract<RunLocalSkillResult, { readonly status: "sealed" | "failure" }>["receipt"]>;
  },
  io: CliIo,
  env: NodeJS.ProcessEnv,
): string {
  const t = theme(io.stderr, env);
  const disposition = runnerReceiptDisposition(result.receipt);
  const status = disposition === "blocked" ? "escalated" : "failure";
  const lines = [
    "",
    `  ${statusIcon(status, t)}  ${t.bold}${result.skill.name}${t.reset}  ${t.dim}${status}${t.reset}`,
    `  ${t.dim}receipt${t.reset}   ${shortId(result.receipt.id)}`,
    `  ${t.dim}schema${t.reset}    ${result.receipt.schema}`,
  ];
  const duration = formatDurationMs(runnerReceiptDurationMs(result.receipt));
  if (duration) lines.push(`  ${t.dim}duration${t.reset}  ${duration}`);
  lines.push(`  ${t.dim}closure${t.reset}   ${disposition}`);
  const outcomeState = runnerReceiptOutcomeState(result.receipt);
  if (outcomeState) lines.push(`  ${t.dim}outcome${t.reset}      ${outcomeState}`);
  const steps = runnerReceiptGraphSteps(result.receipt);
  if (steps.length > 0) lines.push(`  ${t.dim}steps${t.reset}     ${steps.length}`);
  const errorText = result.execution.errorMessage ?? result.execution.stderr ?? result.execution.stdout;
  if (errorText.trim()) {
    lines.push(`  ${t.dim}${status === "escalated" ? "reason" : "error"}${t.reset}     ${truncateMultiline(errorText, 8)}`);
  }
  lines.push(`  ${t.dim}history${t.reset}   runx history --json`);
  lines.push("");
  return lines.join("\n");
}

function formatDurationMs(durationMs: number | undefined): string | undefined {
  if (typeof durationMs !== "number" || Number.isNaN(durationMs)) return undefined;
  if (durationMs < 1000) return `${durationMs}ms`;
  const seconds = durationMs / 1000;
  if (seconds < 60) return `${seconds.toFixed(seconds < 10 ? 1 : 0)}s`;
  const minutes = Math.floor(seconds / 60);
  const remainder = Math.round(seconds % 60);
  return `${minutes}m ${remainder}s`;
}

function extractOutputHighlights(stdout: string): Array<[string, string]> {
  const trimmed = stdout.trim();
  if (!trimmed) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed) as unknown;
  } catch {
    return trimmed.includes("\n") ? [] : [["output", trimmed]];
  }
  if (!isRecord(parsed)) return [];
  const output = isRecord(parsed.data) ? parsed.data : parsed;
  const fields: Array<[string, string]> = [];
  const push = (key: string, label = key) => {
    const value = output[key];
    if (value === undefined) return;
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      fields.push([label, String(value)]);
    }
  };
  push("output_dir");
  push("index_path");
  push("command");
  push("verified");
  push("generated");
  push("contains_doctype");
  push("completed_state");
  push("review_path");
  push("spec_path");
  push("action");
  push("status");
  push("summary");
  push("issue");
  push("thread_locator", "thread");
  push("task_id", "task");
  push("lane");
  push("target_repo", "target");
  push("repo_root", "repo");
  push("preview_url", "preview");
  push("review_comment_url", "review");
  push("pull_request_url", "pr");
  return fields;
}

function truncateMultiline(text: string, maxLines = 8): string {
  const lines = text.trim().split("\n");
  if (lines.length <= maxLines) return lines.join("\n");
  return `${lines.slice(0, maxLines).join("\n")}\n…`;
}
