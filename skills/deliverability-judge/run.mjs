import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

// deliverability-judge â€” read-only (SHAPE-A) fusion of sealed provider evidence
// against operator policy thresholds. Mints no authority, holds no state, emits
// no Effect. Produces a verdict always, and a recommendation only when every
// signal is sealed and the signals do not contradict; otherwise an escalation
// record that still seals.

const SCHEMA = "deliverability.judge.result.v1";
const SIGNAL_NAMES = ["postmaster_report", "bounce_metrics", "complaint_metrics", "placement_probe"];

const inputs = readInputs();
const skillRoot = process.cwd();

const evidence = inputs.evidence;
const policy = inputs.policy;

if (!evidence || typeof evidence !== "object") {
  throw new Error("evidence input is required and must be an object");
}
if (!policy || typeof policy !== "object") {
  throw new Error("policy input is required and must be an object");
}

const minReputation = policy.min_reputation_score;
const maxBounce = policy.max_bounce_pct;
const maxComplaint = policy.max_complaint_pct;

// --- 1. Seal check: every signal must be present with source + timestamp. -----
const signals = {};
const missing = [];

for (const name of SIGNAL_NAMES) {
  const sig = evidence[name];
  if (!sig || typeof sig !== "object") {
    missing.push(name);
    signals[name] = { sealed: false, within_policy: false, value: null, threshold: null, reason: "signal absent" };
    continue;
  }
  if (typeof sig.source !== "string" || sig.source.length === 0) {
    missing.push(name);
    signals[name] = { sealed: false, within_policy: false, value: null, threshold: null, reason: "missing source" };
    continue;
  }
  if (typeof sig.timestamp !== "string" || sig.timestamp.length === 0) {
    missing.push(name);
    signals[name] = { sealed: false, within_policy: false, value: null, threshold: null, reason: "missing timestamp" };
    continue;
  }
  signals[name] = { sealed: true, source: sig.source, timestamp: sig.timestamp, within_policy: null, value: null, threshold: null };
}

const everySignalSealed = missing.length === 0;

// --- 2. Policy evaluation for each sealed signal. -----------------------------
const reputationScore = signals.postmaster_report.sealed ? evidence.postmaster_report.reputation_score : null;
const bouncePct = signals.bounce_metrics.sealed ? evidence.bounce_metrics.bounce_pct : null;
const complaintPct = signals.complaint_metrics.sealed ? evidence.complaint_metrics.complaint_pct : null;
const inboxPct = signals.placement_probe.sealed ? evidence.placement_probe.inbox_pct : null;

if (signals.postmaster_report.sealed) {
  signals.postmaster_report.value = reputationScore;
  signals.postmaster_report.threshold = minReputation;
  signals.postmaster_report.within_policy = typeof reputationScore === "number" && reputationScore >= minReputation;
}
if (signals.bounce_metrics.sealed) {
  signals.bounce_metrics.value = bouncePct;
  signals.bounce_metrics.threshold = maxBounce;
  signals.bounce_metrics.within_policy = typeof bouncePct === "number" && bouncePct <= maxBounce;
}
if (signals.complaint_metrics.sealed) {
  signals.complaint_metrics.value = complaintPct;
  signals.complaint_metrics.threshold = maxComplaint;
  signals.complaint_metrics.within_policy = typeof complaintPct === "number" && complaintPct <= maxComplaint;
}
if (signals.placement_probe.sealed) {
  // Placement is a corroborating probe: inbox rate below 90% reads as degraded.
  signals.placement_probe.value = inboxPct;
  signals.placement_probe.threshold = 90;
  signals.placement_probe.within_policy = typeof inboxPct === "number" && inboxPct >= 90;
}

// --- 3. Contradiction detection. ----------------------------------------------
// A contradiction is signals that DISAGREE: the reputation signal reads healthy
// while a hard delivery signal (bounce or complaint) reads out of policy. Signals
// that agree on degradation are NOT a contradiction â€” they consistently degrade.
const contradictions = [];
const reputationHealthy = signals.postmaster_report.within_policy === true;
const bounceOutOfPolicy = signals.bounce_metrics.sealed && signals.bounce_metrics.within_policy === false;
const complaintOutOfPolicy = signals.complaint_metrics.sealed && signals.complaint_metrics.within_policy === false;
const placementDegraded = signals.placement_probe.sealed && signals.placement_probe.within_policy === false;

if (everySignalSealed && reputationHealthy && bounceOutOfPolicy) {
  contradictions.push({
    signals: ["postmaster_report", "bounce_metrics"],
    reason: `postmaster_report reputation ${reputationScore} meets threshold ${minReputation}, but bounce_metrics ${bouncePct}% exceeds threshold ${maxBounce}%.`,
  });
}
if (everySignalSealed && reputationHealthy && complaintOutOfPolicy) {
  contradictions.push({
    signals: ["postmaster_report", "complaint_metrics"],
    reason: `postmaster_report reputation ${reputationScore} meets threshold ${minReputation}, but complaint_metrics ${complaintPct}% exceeds threshold ${maxComplaint}%.`,
  });
}
if (everySignalSealed && reputationHealthy && placementDegraded) {
  contradictions.push({
    signals: ["postmaster_report", "placement_probe"],
    reason: `postmaster_report reputation ${reputationScore} meets threshold ${minReputation}, but placement_probe inbox rate ${inboxPct}% is below 90%.`,
  });
}

// --- 4. Fuse into a verdict + (conditional) recommendation. --------------------
let verdict;
let recommendation = null;
let escalation = null;
let sealedOk = true; // exit 0 (sealed) vs exit 1 (failure disposition, still sealed receipt)

if (!everySignalSealed) {
  // Partial signal set â€” refuse a verdict, never invent the missing signal.
  verdict = { state: "refused", confidence_window: null, reason: `Refused: partial signal set. Missing or unsealed: ${missing.join(", ")}.` };
  escalation = { kind: "missing_signals", missing_signals: missing, route: "human_reviewer" };
  sealedOk = false;
} else if (contradictions.length > 0) {
  // Signals contradict â€” refuse to fuse, escalate, emit no recommendation.
  const names = [...new Set(contradictions.flatMap((c) => c.signals))];
  verdict = { state: "refused", confidence_window: null, reason: `Refused: contradictory signals cannot be fused. ${contradictions.map((c) => c.reason).join(" ")}` };
  escalation = { kind: "contradictory_signals", contradicting_signals: names, contradictions, route: "human_reviewer" };
  sealedOk = false;
} else {
  const outOfPolicy = SIGNAL_NAMES.filter((n) => signals[n].within_policy === false);
  const evidenceHash = `sha256:${sha256(canonical(evidence))}`;
  const bindings = SIGNAL_NAMES.map((n) => ({
    signal: n,
    source: evidence[n].source,
    timestamp: evidence[n].timestamp,
    within_policy: signals[n].within_policy,
  }));
  if (outOfPolicy.length === 0) {
    verdict = { state: "healthy", confidence_window: "7d", reason: "All four signals are sealed, within policy, and non-contradictory." };
    recommendation = { action: "continue", signal_bindings: bindings, evidence_hash: evidenceHash };
  } else {
    // Signals agree on degradation (no contradiction): recommend throttle/pause.
    const action = outOfPolicy.length >= 2 ? "pause" : "throttle";
    verdict = {
      state: outOfPolicy.length >= 2 ? "critical" : "degraded",
      confidence_window: "7d",
      reason: `Signals agree on degradation: ${outOfPolicy.join(", ")} outside policy, no contradicting healthy signal.`,
    };
    recommendation = { action, signal_bindings: bindings, evidence_hash: evidenceHash };
  }
}

// --- 5. Build the read-only packet. -------------------------------------------
const result = {
  schema: SCHEMA,
  status: sealedOk ? "sealed" : "refused",
  data: {
    verdict,
    signals,
    recommendation,
    contradictions,
    missing_signals: missing,
    escalation,
    validation: {
      valid: sealedOk,
      every_signal_sealed: everySignalSealed,
      every_signal_has_source: SIGNAL_NAMES.every((n) => signals[n].sealed && typeof evidence[n]?.source === "string"),
      every_signal_has_timestamp: SIGNAL_NAMES.every((n) => signals[n].sealed && typeof evidence[n]?.timestamp === "string"),
      no_contradictions: contradictions.length === 0,
      no_invented_signals: SIGNAL_NAMES.every((n) => n in signals),
      read_only: true,
      mints_authority: false,
      holds_state: false,
    },
  },
};

const report = renderReport(result);
writeArtifacts(inputs.output_dir, result, report, skillRoot);

process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);

// Refused verdicts (partial set or contradiction) still seal a receipt, but the
// run exits non-zero so the harness records a "failure" disposition.
if (!sealedOk) {
  process.exit(1);
}

// --- helpers ------------------------------------------------------------------
function readInputs() {
  const raw = process.env.RUNX_INPUTS_PATH
    ? fs.readFileSync(process.env.RUNX_INPUTS_PATH, "utf8")
    : process.env.RUNX_INPUTS_JSON || "{}";
  try {
    return JSON.parse(raw);
  } catch (e) {
    throw new Error(`Invalid input JSON: ${e.message}`);
  }
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

// Stable stringify so the evidence hash is order-independent.
function canonical(value) {
  if (Array.isArray(value)) {
    return `[${value.map(canonical).join(",")}]`;
  }
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function renderReport(packet) {
  const d = packet.data;
  const out = [];
  out.push("# Deliverability Judge Verdict");
  out.push("");
  out.push(`- **State:** ${d.verdict.state}`);
  if (d.verdict.confidence_window) out.push(`- **Confidence window:** ${d.verdict.confidence_window}`);
  out.push(`- **Reason:** ${d.verdict.reason}`);
  out.push("");
  out.push("## Signals");
  out.push("");
  out.push("| Signal | Sealed | Source | Value | Threshold | Within policy |");
  out.push("| --- | --- | --- | --- | --- | --- |");
  for (const name of SIGNAL_NAMES) {
    const s = d.signals[name];
    out.push(`| ${name} | ${s.sealed ? "yes" : "no"} | ${s.source ?? "â€”"} | ${s.value ?? "â€”"} | ${s.threshold ?? "â€”"} | ${s.within_policy === null ? "â€”" : s.within_policy ? "yes" : "no"} |`);
  }
  out.push("");
  if (d.recommendation) {
    out.push("## Recommendation (read-only)");
    out.push("");
    out.push(`- **Action:** ${d.recommendation.action}`);
    out.push(`- **Evidence hash:** \`${d.recommendation.evidence_hash}\``);
    out.push("");
  }
  if (d.contradictions.length > 0) {
    out.push("## Contradictions (escalated)");
    out.push("");
    for (const c of d.contradictions) out.push(`- [${c.signals.join(" vs ")}] ${c.reason}`);
    out.push("");
  }
  if (d.missing_signals.length > 0) {
    out.push("## Missing signals (escalated)");
    out.push("");
    for (const m of d.missing_signals) out.push(`- ${m}`);
    out.push("");
  }
  out.push("## Read-only guarantees");
  out.push("");
  out.push("- Recommendation is a signal, not an Effect; no throttle/pause is executed.");
  out.push("- Contradictory or partial signals escalate to a human reviewer and emit no recommendation.");
  out.push("- Only the four declared signals are evaluated; no signal is invented.");
  out.push("- The skill mints no authority and holds no state.");
  out.push("");
  return `${out.join("\n")}\n`;
}

function writeArtifacts(outputDir, packet, report, root) {
  if (!outputDir) {
    packet.data.artifacts = {};
    return;
  }
  const resolved = path.resolve(root, outputDir);
  const normalizedRoot = root.endsWith(path.sep) ? root : `${root}${path.sep}`;
  if (resolved !== root && !resolved.startsWith(normalizedRoot)) {
    throw new Error("output_dir must stay inside the skill directory");
  }
  fs.mkdirSync(resolved, { recursive: true });
  const evidencePath = path.join(resolved, "evidence.json");
  const reportPath = path.join(resolved, "report.md");
  fs.writeFileSync(evidencePath, `${JSON.stringify(packet, null, 2)}\n`);
  fs.writeFileSync(reportPath, report);
  packet.data.artifacts = {
    evidence_json: path.relative(root, evidencePath),
    report_md: path.relative(root, reportPath),
  };
}
