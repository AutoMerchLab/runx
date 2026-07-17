import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
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

  it("runs safe harness validation in an isolated project receipt store", async () => {
    const validationModule = "../skills/skill-lab/validation.mjs";
    const { validatePackage } = await import(validationModule) as {
      validatePackage(options: {
        repoRoot: string;
        target: string;
        targetDir: string;
        runx: string;
      }): { verdict: string };
    };
    const directory = mkdtempSync(path.join(os.tmpdir(), "runx-skill-lab-validation-test-"));
    const runx = path.join(directory, "runx");
    const log = path.join(directory, "harness.json");
    writeFileSync(runx, `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
if (args[0] === "skill") {
  process.stdout.write(JSON.stringify({ status: "ok", name: "fixture", version: "0.1.0", readiness: { status: "ready" }, capabilities: { execution: "read" }, runner: {}, runners: ["read"] }));
} else {
  const index = args.indexOf("--receipt-dir");
  const receiptDir = index >= 0 ? args[index + 1] : null;
  fs.writeFileSync(process.env.RUNX_TEST_LOG, JSON.stringify({ args, receiptDir, existed: receiptDir ? fs.existsSync(receiptDir) : false }));
  process.stdout.write(JSON.stringify({ status: "passed", case_count: 1, assertion_error_count: 0, case_names: ["fixture"] }));
}
`);
    chmodSync(runx, 0o755);

    const previousLog = process.env.RUNX_TEST_LOG;
    process.env.RUNX_TEST_LOG = log;
    try {
      const result = validatePackage({ repoRoot: root, target: path.join(root, "skills/weather-forecast"), targetDir: "skills/weather-forecast", runx });
      const invocation = JSON.parse(readFileSync(log, "utf8"));
      expect(result.verdict).toBe("validated");
      expect(invocation.args).toContain("--receipt-dir");
      expect(invocation.existed).toBe(true);
      expect(invocation.receiptDir.startsWith(
        `${path.join(root, ".runx", "skill-lab", "validation")}${path.sep}`,
      )).toBe(true);
      expect(existsSync(invocation.receiptDir)).toBe(false);
    } finally {
      if (previousLog === undefined) delete process.env.RUNX_TEST_LOG;
      else process.env.RUNX_TEST_LOG = previousLog;
      rmSync(directory, { recursive: true, force: true });
    }
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
