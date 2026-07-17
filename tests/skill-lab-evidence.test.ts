import { spawnSync } from "node:child_process";
import path from "node:path";

import { describe, expect, it } from "vitest";

const root = process.cwd();

describe("skill-lab improvement evidence", () => {
  it("validates and preserves the stable review-receipt packet", () => {
    const failurePacket = {
      verdict: "needs_update",
      failure_summary: "The downstream step received flattened context.",
      improvement_proposals: [{
        target: "skills/example/X.yaml",
        change: "Consume the named artifact data.",
        rationale: "Preserves the structured packet.",
        risk: "Existing stdout-shaped fixtures need updates.",
        ignored: "must not cross the boundary",
      }],
      next_harness_checks: ["Structured context reaches the consumer."],
      ignored: { raw_output: "must not cross the boundary" },
    };

    const result = runScript("skills/skill-lab/inspect_target.mjs", {
      objective: "Repair the structured context edge.",
      repo_root: ".",
      target_dir: "skills/skill-lab",
      failure_packet: failurePacket,
    });

    expect(result.status).toBe(0);
    expect(result.data.authoring_context.improvement_evidence.failure_packet).toEqual({
      verdict: "needs_update",
      failure_summary: "The downstream step received flattened context.",
      improvement_proposals: [{
        target: "skills/example/X.yaml",
        change: "Consume the named artifact data.",
        rationale: "Preserves the structured packet.",
        risk: "Existing stdout-shaped fixtures need updates.",
      }],
      next_harness_checks: ["Structured context reaches the consumer."],
    });
  });

  it("rejects an internally inconsistent review packet", () => {
    const result = runScript("skills/skill-lab/inspect_target.mjs", {
      objective: "Do not mutate from a passing review.",
      repo_root: ".",
      target_dir: "skills/skill-lab",
      failure_packet: {
        verdict: "pass",
        failure_summary: "No defect was found.",
        improvement_proposals: [{
          target: "skills/example/X.yaml",
          change: "Change it anyway.",
          rationale: "None.",
          risk: "Unjustified mutation.",
        }],
        next_harness_checks: [],
      },
    });

    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("verdict pass must not propose package changes");
  });

  it("forwards a reflect-digest handoff without inventing evidence fields", () => {
    const handoffResult = runScript("skills/reflect-digest/build_handoffs.mjs", {
      grouped_reflections: [{
        skill_ref: "sourcey",
        supporting_receipt_ids: ["rx_sourcey_1", "rx_sourcey_2"],
      }],
      proposals: [{
        skill_ref: "sourcey",
        target_dir: "skills/sourcey",
        objective: "Preserve the stable coverage field.",
        evidence_summary: "Two receipts report the same missing field.",
        supporting_receipt_ids: ["rx_sourcey_1", "rx_sourcey_2"],
        boundaries: ["Do not change publication behavior."],
      }],
    });
    expect(handoffResult.status).toBe(0);
    const [handoff] = handoffResult.data.skill_lab_handoffs;

    const inspectResult = runScript("skills/skill-lab/inspect_target.mjs", {
      ...handoff.inputs,
      repo_root: ".",
    });

    expect(inspectResult.status).toBe(0);
    expect(inspectResult.data.authoring_context.improvement_evidence).toEqual({
      receipt_id: "rx_sourcey_1",
      receipt_summary: "Two receipts report the same missing field. Supporting receipts: rx_sourcey_1, rx_sourcey_2.",
      harness_output: null,
      failure_packet: null,
    });
  });
});

function runScript(script: string, inputs: unknown): {
  readonly status: number | null;
  readonly data: any;
  readonly stderr: string;
} {
  const result = spawnSync(process.execPath, [path.join(root, script)], {
    cwd: root,
    env: {
      ...process.env,
      RUNX_CWD: root,
      RUNX_INPUTS_JSON: JSON.stringify(inputs),
    },
    encoding: "utf8",
  });
  let data: any = null;
  if (result.stdout.trim()) data = JSON.parse(result.stdout);
  return { status: result.status, data, stderr: result.stderr };
}
