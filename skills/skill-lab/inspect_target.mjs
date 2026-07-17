import fs from "node:fs";
import path from "node:path";

const inputs = JSON.parse(process.env.RUNX_INPUTS_JSON || "{}");
const callerRoot = path.resolve(process.env.RUNX_CWD || process.cwd());
const requestedRoot = String(inputs.repo_root || ".");
const repoRoot = path.isAbsolute(requestedRoot)
  ? path.normalize(requestedRoot)
  : path.resolve(callerRoot, requestedRoot);
const targetDir = normalizeRelativePath(inputs.target_dir, { optional: true });
const targetRoot = targetDir ? path.resolve(repoRoot, targetDir) : null;

if (targetRoot && !isInside(repoRoot, targetRoot)) {
  throw new Error("target_dir must stay inside repo_root");
}

const catalogRoot = fs.existsSync(path.join(repoRoot, "skills"))
  ? path.join(repoRoot, "skills")
  : repoRoot;
const catalogSkills = fs.readdirSync(catalogRoot, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .filter((name) => fs.existsSync(path.join(catalogRoot, name, "SKILL.md")))
  .sort()
  .slice(0, 500);

const targetFiles = targetRoot && fs.existsSync(targetRoot)
  ? listFiles(targetRoot, targetRoot, 200)
  : [];
const improvementEvidence = buildImprovementEvidence(inputs);

process.stdout.write(`${JSON.stringify({
  authoring_context: {
    schema: "runx.skill_lab.authoring_context.v1",
    repo_root: repoRoot,
    target_dir: targetDir,
    target_exists: Boolean(targetRoot && fs.existsSync(targetRoot)),
    target_files: targetFiles,
    catalog_root: path.relative(repoRoot, catalogRoot) || ".",
    catalog_skills: catalogSkills,
    objective: boundedString(inputs.objective, "objective", 10_000),
    failure_evidence_present: improvementEvidence !== null,
    improvement_evidence: improvementEvidence,
  },
}, null, 2)}\n`);

function listFiles(root, current, remaining) {
  if (remaining <= 0) return [];
  const files = [];
  for (const entry of fs.readdirSync(current, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (files.length >= remaining) break;
    if ([".git", ".runx", "node_modules"].includes(entry.name)) continue;
    const absolute = path.join(current, entry.name);
    if (entry.isSymbolicLink()) continue;
    if (entry.isDirectory()) {
      files.push(...listFiles(root, absolute, remaining - files.length));
      continue;
    }
    if (!entry.isFile()) continue;
    const stat = fs.statSync(absolute);
    files.push({
      path: path.relative(root, absolute),
      bytes: stat.size,
    });
  }
  return files;
}

function normalizeRelativePath(value, { optional = false } = {}) {
  const text = stringValue(value);
  if (!text) {
    if (optional) return null;
    throw new Error("target_dir is required");
  }
  if (path.isAbsolute(text)) throw new Error("target_dir must be repo-relative");
  const normalized = path.normalize(text);
  if (normalized === "." || normalized.startsWith(`..${path.sep}`) || normalized === "..") {
    throw new Error("target_dir must name a child path inside repo_root");
  }
  return normalized;
}

function isInside(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function stringValue(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function isRecord(value) {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function buildImprovementEvidence(inputs) {
  const receiptId = boundedString(inputs.receipt_id, "receipt_id", 500);
  const receiptSummary = boundedString(inputs.receipt_summary, "receipt_summary", 10_000);
  const harnessOutput = boundedString(inputs.harness_output, "harness_output", 20_000);
  const failurePacket = inputs.failure_packet === undefined || inputs.failure_packet === null
    ? null
    : validateFailurePacket(inputs.failure_packet);
  if (!receiptId && !receiptSummary && !harnessOutput && !failurePacket) return null;
  return {
    receipt_id: receiptId,
    receipt_summary: receiptSummary,
    harness_output: harnessOutput,
    failure_packet: failurePacket,
  };
}

function validateFailurePacket(value) {
  if (!isRecord(value)) throw new Error("failure_packet must be a runx.review.receipt.v1 object");
  const verdict = requiredEnum(value.verdict, "failure_packet.verdict", ["pass", "needs_update", "blocked"]);
  const failureSummary = requiredBoundedString(value.failure_summary, "failure_packet.failure_summary", 10_000);
  const proposals = boundedArray(value.improvement_proposals, "failure_packet.improvement_proposals", 3)
    .map((proposal, index) => {
      if (!isRecord(proposal)) throw new Error(`failure_packet.improvement_proposals[${index}] must be an object`);
      return {
        target: requiredBoundedString(proposal.target, `failure_packet.improvement_proposals[${index}].target`, 1_000),
        change: requiredBoundedString(proposal.change, `failure_packet.improvement_proposals[${index}].change`, 5_000),
        rationale: requiredBoundedString(proposal.rationale, `failure_packet.improvement_proposals[${index}].rationale`, 5_000),
        risk: requiredBoundedString(proposal.risk, `failure_packet.improvement_proposals[${index}].risk`, 5_000),
      };
    });
  const checks = boundedArray(value.next_harness_checks, "failure_packet.next_harness_checks", 20)
    .map((check, index) => requiredBoundedString(check, `failure_packet.next_harness_checks[${index}]`, 2_000));
  if (verdict === "pass" && proposals.length > 0) {
    throw new Error("failure_packet with verdict pass must not propose package changes");
  }
  return {
    verdict,
    failure_summary: failureSummary,
    improvement_proposals: proposals,
    next_harness_checks: checks,
  };
}

function boundedArray(value, field, limit) {
  if (!Array.isArray(value)) throw new Error(`${field} must be an array`);
  if (value.length > limit) throw new Error(`${field} may contain at most ${limit} entries`);
  return value;
}

function boundedString(value, field, maxLength) {
  const text = stringValue(value);
  if (text && text.length > maxLength) throw new Error(`${field} exceeds ${maxLength} characters`);
  return text;
}

function requiredBoundedString(value, field, maxLength) {
  const text = boundedString(value, field, maxLength);
  if (!text) throw new Error(`${field} must be a non-empty string`);
  return text;
}

function requiredEnum(value, field, allowed) {
  const text = requiredBoundedString(value, field, 100);
  if (!allowed.includes(text)) throw new Error(`${field} must be one of: ${allowed.join(", ")}`);
  return text;
}
