import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

export function validatePackage({ repoRoot, target, targetDir, runx = process.env.RUNX_BIN || "runx" }) {
  const env = { ...process.env };
  delete env.RUNX_INPUTS_JSON;
  delete env.RUNX_INPUTS_PATH;

  const inspection = invoke({ runx, args: ["skill", "inspect", target, "--json"], cwd: repoRoot, env, timeout: 30_000 });
  let harness = {
    attempted: false,
    status: "skipped",
    reason: "inspection_failed",
  };

  if (inspection.ok) {
    const execution = inspection.data?.capabilities?.execution;
    if (execution === "read" || execution === "plan") {
      const receiptDir = isolatedReceiptDir(repoRoot);
      try {
        const result = invoke({
          runx,
          args: ["harness", target, "--receipt-dir", receiptDir, "--json"],
          cwd: repoRoot,
          env,
          timeout: 120_000,
        });
        harness = {
          attempted: true,
          status: result.data?.status || (result.ok ? "passed" : "failed"),
          reason: result.ok ? null : "native_harness_failed",
          case_count: numberValue(result.data?.case_count),
          assertion_error_count: numberValue(result.data?.assertion_error_count),
          case_names: stringArray(result.data?.case_names),
          error: result.ok ? null : result.error,
        };
      } finally {
        fs.rmSync(receiptDir, { recursive: true, force: true });
      }
    } else {
      harness = {
        attempted: false,
        status: "skipped",
        reason: "needs_consequential_harness_approval",
      };
    }
  }

  const verdict = !inspection.ok
    ? "invalid"
    : harness.status === "failed"
      ? "failed"
      : harness.reason === "needs_consequential_harness_approval"
        ? "needs_consequential_test"
        : "validated";

  return {
    schema: "runx.skill_lab.validation.v1",
    target_dir: targetDir,
    verdict,
    inspect: inspection.ok ? {
      status: inspection.data.status,
      name: inspection.data.name,
      version: inspection.data.version,
      readiness: inspection.data.readiness,
      capabilities: inspection.data.capabilities,
      runner: inspection.data.runner,
      runners: inspection.data.runners,
    } : {
      status: "failed",
      error: inspection.error,
    },
    harness,
  };
}

function isolatedReceiptDir(repoRoot) {
  const root = path.join(repoRoot, ".runx", "skill-lab", "validation");
  fs.mkdirSync(root, { recursive: true });
  return fs.mkdtempSync(path.join(root, "receipts-"));
}

function invoke({ runx, args, cwd, env, timeout }) {
  const result = spawnSync(runx, args, {
    cwd,
    env,
    encoding: "utf8",
    timeout,
    maxBuffer: 4 * 1024 * 1024,
  });
  let data;
  try {
    data = JSON.parse(result.stdout || "{}");
  } catch {
    data = null;
  }
  return {
    ok: result.status === 0 && data !== null,
    data,
    error: structuredError(data) || sanitize(result.stderr || result.error?.message || "runx command failed"),
  };
}

function structuredError(data) {
  if (typeof data?.error?.message === "string") return sanitize(data.error.message);
  if (typeof data?.error === "string") return sanitize(data.error);
  if (typeof data?.message === "string") return sanitize(data.message);
  return null;
}

function sanitize(value) {
  return String(value).replace(/\s+/g, " ").trim().slice(0, 500);
}

function numberValue(value) {
  return Number.isFinite(value) ? value : 0;
}

function stringArray(value) {
  return Array.isArray(value) ? value.map(String).slice(0, 100) : [];
}