import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || "{}");
const draft = record(inputs.policy_proposal);
const decision = enumValue(draft.decision, ["ready", "needs_input", "reject"], "decision");
const existingPolicy = optionalRecord(inputs.existing_policy, "existing_policy");

if (decision !== "ready") {
  emit({
    ...baseProposal(draft, decision),
    policy: draft.policy && typeof draft.policy === "object" ? draft.policy : null,
    validation: {
      status: "not_run",
      engine: "runx policy lint",
      findings: [],
      readback: null,
      reason: "The draft stopped before native lint because required governance inputs were unresolved.",
    },
  });
}

const policy = requiredRecord(draft.policy, "policy");
const attenuationFindings = existingPolicy ? wideningFindings(existingPolicy, policy) : [];
if (attenuationFindings.length > 0) {
  emit({
    ...baseProposal(draft, "reject"),
    policy,
    validation: {
      status: "fail",
      engine: "runx policy lint",
      findings: attenuationFindings,
      readback: null,
      reason: "The proposed change widens existing authority and cannot use the tightening lane.",
    },
  });
}

const native = lintWithRunx(policy);
emit({
  ...baseProposal(draft, native.status === "pass" ? "ready" : "reject"),
  policy,
  validation: native,
});

function lintWithRunx(policy) {
  const directory = mkdtempSync(path.join(os.tmpdir(), "runx-policy-author-"));
  const policyPath = path.join(directory, "policy.json");
  try {
    writeFileSync(policyPath, `${JSON.stringify(policy, null, 2)}\n`, { encoding: "utf8", mode: 0o600 });
    const result = spawnSync("runx", ["policy", "lint", policyPath, "--json"], {
      env: process.env,
      encoding: "utf8",
    });
    const output = parseJson(result.stdout);
    if (output) {
      return {
        status: result.status === 0 && output.status === "success" ? "pass" : "fail",
        engine: "runx policy lint",
        findings: Array.isArray(output.findings) ? output.findings.map(projectFinding) : [],
        readback: output.policy && typeof output.policy === "object" ? output.policy : null,
        reason: result.status === 0 ? "Native policy lint passed." : "Native policy lint rejected the proposal.",
      };
    }
    return {
      status: "fail",
      engine: "runx policy lint",
      findings: [{
        code: "policy.native_lint.invalid",
        path: "$",
        message: "The proposal could not be parsed or validated by the native policy engine.",
      }],
      readback: null,
      reason: "Native policy lint did not return a structured verdict.",
    };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

function wideningFindings(existing, proposed) {
  const findings = [];
  requireSubset(findings, "targets", ids(existing.targets, "repo"), ids(proposed.targets, "repo"));
  requireSubset(findings, "sources", ids(existing.sources, "source_id"), ids(proposed.sources, "source_id"));
  requireSubset(findings, "runners", ids(existing.runners, "runner_id"), ids(proposed.runners, "runner_id"));
  compareRules(findings, existing.sources, proposed.sources, "source_id", ["allowed_locators", "allowed_actions"]);
  compareRules(findings, existing.runners, proposed.runners, "runner_id", ["allowed_actions", "target_repos"]);
  compareRules(findings, existing.targets, proposed.targets, "repo", ["allowed_actions", "runner_ids"]);
  compareConfidence(findings, existing.sources, proposed.sources);
  comparePermissions(findings, record(existing.permissions), record(proposed.permissions));
  return findings;
}

function requireSubset(findings, field, existing, proposed) {
  for (const value of proposed) {
    if (!existing.has(value)) addWidening(findings, `${field}.${value}`);
  }
}

function compareRules(findings, existingRules, proposedRules, key, fields) {
  const existing = indexBy(existingRules, key);
  for (const proposed of records(proposedRules)) {
    const id = stringValue(proposed[key]);
    const prior = existing.get(id);
    if (!prior) continue;
    for (const field of fields) {
      requireSubset(findings, `${key}.${id}.${field}`, stringSet(prior[field]), stringSet(proposed[field]));
    }
  }
}

function compareConfidence(findings, existingSources, proposedSources) {
  const existing = indexBy(existingSources, "source_id");
  for (const proposed of records(proposedSources)) {
    const prior = existing.get(stringValue(proposed.source_id));
    if (!prior) continue;
    const previous = numberValue(prior.minimum_confidence);
    const next = numberValue(proposed.minimum_confidence);
    if (previous !== null && (next === null || next < previous)) {
      addWidening(findings, `source.${proposed.source_id}.minimum_confidence`);
    }
  }
}

function comparePermissions(findings, existing, proposed) {
  if (existing.auto_merge !== true && proposed.auto_merge === true) addWidening(findings, "permissions.auto_merge");
  if (existing.mutate_target_repo !== true && proposed.mutate_target_repo === true) addWidening(findings, "permissions.mutate_target_repo");
  if (existing.require_human_merge_gate === true && proposed.require_human_merge_gate !== true) {
    addWidening(findings, "permissions.require_human_merge_gate");
  }
}

function addWidening(findings, pathValue) {
  findings.push({
    code: "policy.attenuation.widened",
    path: pathValue,
    message: "The tightening lane cannot add or widen this authority.",
  });
}

function baseProposal(draft, decision) {
  return {
    decision,
    rationale: stringValue(draft.rationale) || "",
    blockers: stringArray(draft.blockers),
    needs_input: stringArray(draft.needs_input),
    success_checkpoint: record(draft.success_checkpoint),
  };
}

function projectFinding(value) {
  const finding = record(value);
  return {
    code: stringValue(finding.code) || "policy.native_lint.finding",
    path: stringValue(finding.path) || "$",
    message: stringValue(finding.message) || "Native policy validation finding.",
  };
}

function indexBy(values, key) {
  return new Map(records(values).map((value) => [stringValue(value[key]), value]).filter(([id]) => id));
}

function ids(values, key) {
  return new Set(records(values).map((value) => stringValue(value[key])).filter(Boolean));
}

function records(value) {
  return Array.isArray(value) ? value.map(record) : [];
}

function stringSet(value) {
  return new Set(stringArray(value));
}

function stringArray(value) {
  return Array.isArray(value) ? [...new Set(value.map(stringValue).filter(Boolean))] : [];
}

function stringValue(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function numberValue(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function record(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function requiredRecord(value, field) {
  const parsed = record(value);
  if (Object.keys(parsed).length === 0) throw new Error(`${field} must be a non-empty object`);
  return parsed;
}

function optionalRecord(value, field) {
  if (value === undefined || value === null) return null;
  return requiredRecord(value, field);
}

function enumValue(value, allowed, field) {
  if (!allowed.includes(value)) throw new Error(`${field} must be one of ${allowed.join(", ")}`);
  return value;
}

function parseJson(value) {
  try {
    return JSON.parse(value || "");
  } catch {
    return null;
  }
}

function emit(policyProposal) {
  process.stdout.write(`${JSON.stringify({ policy_proposal: policyProposal }, null, 2)}\n`);
  process.exit(0);
}
