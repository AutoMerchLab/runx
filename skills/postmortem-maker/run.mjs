// postmortem-maker: turn incident fragments (timeline, alerts, deploys, chat
// notes, policy) into a traceable postmortem packet, WITHOUT promoting unknowns
// to facts.
//
// Design notes:
//   - Fully deterministic (time correlation + hedge-cue classification over
//     regex, no LLM), so harness runs seal reproducibly.
//   - Every timeline entry and every root-cause claim carries an `evidence`
//     citation that points at the exact input item it came from
//     (e.g. "deploy_events[0]", "alerts[1]"); nothing is asserted without a
//     source.
//   - Facts (timeline events, alerts, deploys) are kept separate from
//     hypotheses (hedged chat speculation). A hypothesis never becomes the
//     root cause on its own and unresolved questions stay in `unknowns`.
//   - `publish_proposal` is a GATED proposal object (requires_approval: true).
//     The skill posts nothing and assigns nothing; a downstream send-as or
//     doc-publisher executor consumes the proposal.

function refuse(reason) {
  console.error(reason);
  process.exit(1);
}

function seal(data) {
  console.log(JSON.stringify(data, null, 2));
  process.exit(0);
}

function parseInput() {
  const inputStr = process.env.RUNX_INPUTS_JSON;
  if (!inputStr) return refuse("No input provided via RUNX_INPUTS_JSON");
  try {
    return JSON.parse(inputStr);
  } catch (e) {
    return refuse("Invalid JSON input");
  }
}

// Inputs may arrive as native arrays/objects (typed runner) or as JSON strings
// (harness inline values). Normalize both to the expected shape.
function asArray(v, label) {
  if (v == null || v === "") return [];
  if (Array.isArray(v)) return v;
  if (typeof v === "string") {
    let parsed;
    try {
      parsed = JSON.parse(v);
    } catch (e) {
      return refuse(`Invalid JSON for ${label}`);
    }
    if (!Array.isArray(parsed)) return refuse(`${label} must be a JSON array`);
    return parsed;
  }
  return refuse(`${label} must be an array`);
}

function asObject(v, label) {
  if (v == null || v === "") return {};
  if (typeof v === "object" && !Array.isArray(v)) return v;
  if (typeof v === "string") {
    let parsed;
    try {
      parsed = JSON.parse(v);
    } catch (e) {
      return refuse(`Invalid JSON for ${label}`);
    }
    if (typeof parsed !== "object" || Array.isArray(parsed))
      return refuse(`${label} must be a JSON object`);
    return parsed;
  }
  return refuse(`${label} must be an object`);
}

// ---- time helpers ----

// Parse an ISO 8601 timestamp or a bare "HH:MM"/"HH:MM:SS" into minutes-of-day.
// Returns null when no time can be read, so an undated fragment is never given
// a fabricated position on the timeline.
function toMinutes(at) {
  if (typeof at !== "string") return null;
  const iso = Date.parse(at);
  if (!Number.isNaN(iso)) return Math.floor(iso / 60000);
  const m = at.match(/^(\d{1,2}):(\d{2})(?::(\d{2}))?$/);
  if (m) return parseInt(m[1], 10) * 60 + parseInt(m[2], 10);
  return null;
}

function atOf(item) {
  if (item && typeof item === "object") return item.at || item.time || item.ts || null;
  return null;
}

// ---- text classification ----

// Hedge cues mark a chat note as speculation (a hypothesis), not an assertion of
// fact. A note carrying any of these is never treated as a confirmed cause.
const HEDGE =
  /\b(maybe|might|may be|possibly|perhaps|i think|i guess|i suspect|suspect|not sure|unsure|could be|probably|seems? like|looks like|my guess|wondering if|any idea|not certain|hard to say)\b|\?/i;

// A declarative causal claim: names something as the cause without hedging.
const CAUSE_CUE =
  /\b(caused by|because of|root cause|introduced|due to|triggered by|regression from|broke|broken by|responsible for|is the culprit|from the .* deploy|the .* deploy .* (introduced|caused|broke))\b/i;

const noteText = (n) =>
  typeof n === "string" ? n : (n && (n.text || n.message || n.note)) || "";

// ---- impact detection ----

const IMPACT_CUE =
  /(\b\d+(?:\.\d+)?\s*%|\b\d[\d,]*\s+(?:users?|customers?|requests?|orders?|checkouts?|sessions?)\b|error rate|latency|downtime|outage|unavailable|failed (?:checkouts?|requests?|orders?|logins?)|p9\d|5\d\d error)/i;

function extractImpact(timeline, alerts) {
  // Prefer an explicit impact field on a timeline event.
  for (let i = 0; i < timeline.length; i++) {
    const ev = timeline[i];
    if (ev && typeof ev === "object" && ev.impact) {
      return { text: String(ev.impact), evidence: `incident_timeline[${i}]` };
    }
  }
  // Otherwise, pull an impact-bearing phrase from an event description.
  for (let i = 0; i < timeline.length; i++) {
    const desc = eventText(timeline[i]);
    if (IMPACT_CUE.test(desc)) return { text: desc, evidence: `incident_timeline[${i}]` };
  }
  for (let i = 0; i < alerts.length; i++) {
    const desc = eventText(alerts[i]);
    if (IMPACT_CUE.test(desc)) return { text: desc, evidence: `alerts[${i}]` };
  }
  return null;
}

// ---- unified timeline ----

function eventText(item) {
  if (typeof item === "string") return item;
  if (item && typeof item === "object")
    return String(item.event || item.description || item.name || item.text || item.message || "");
  return "";
}

function buildTimeline(incident_timeline, alerts, deploy_events) {
  const rows = [];
  incident_timeline.forEach((e, i) => {
    rows.push({ at: atOf(e), min: toMinutes(atOf(e)), kind: "event",
      description: eventText(e), evidence: `incident_timeline[${i}]` });
  });
  alerts.forEach((a, i) => {
    const name = (a && typeof a === "object" && a.name) ? a.name : eventText(a);
    const sev = (a && typeof a === "object" && a.severity) ? ` (${a.severity})` : "";
    rows.push({ at: atOf(a), min: toMinutes(atOf(a)), kind: "alert",
      description: `Alert fired: ${name}${sev}`, evidence: `alerts[${i}]` });
  });
  deploy_events.forEach((d, i) => {
    const svc = (d && typeof d === "object") ? (d.service || d.name || "service") : String(d);
    const ver = (d && typeof d === "object" && d.version) ? `@${d.version}` : "";
    rows.push({ at: atOf(d), min: toMinutes(atOf(d)), kind: "deploy",
      description: `Deploy: ${svc}${ver}`, evidence: `deploy_events[${i}]` });
  });
  // Stable chronological sort; undated rows sink to the end but keep their order.
  rows.sort((a, b) => {
    if (a.min == null && b.min == null) return 0;
    if (a.min == null) return 1;
    if (b.min == null) return -1;
    return a.min - b.min;
  });
  return rows;
}

// ---- root cause correlation ----

function firstAlertMinute(alerts) {
  let best = null;
  for (const a of alerts) {
    const m = toMinutes(atOf(a));
    if (m == null) continue;
    if (best == null || m < best) best = m;
  }
  return best;
}

function main() {
  const parsed = parseInput();
  const incident_timeline = asArray(parsed.incident_timeline, "incident_timeline");
  const alerts = asArray(parsed.alerts, "alerts");
  const deploy_events = asArray(parsed.deploy_events, "deploy_events");
  const chat_notes = asArray(parsed.chat_notes, "chat_notes");
  const policy = asObject(parsed.postmortem_policy, "postmortem_policy");

  // Refuse an empty incident: with no timeline, alerts, or deploys there is no
  // evidence to fold and any postmortem would be invented.
  if (incident_timeline.length === 0 && alerts.length === 0 && deploy_events.length === 0) {
    return refuse("Insufficient evidence: incident_timeline, alerts, and deploy_events are all empty");
  }

  const windowMin = Number.isFinite(policy.max_correlation_window_min)
    ? policy.max_correlation_window_min : 30;
  const requireConfirmed = policy.require_confirmed_root_cause !== false; // default true
  const publishTarget = policy.publish_target || "incident-review";
  const visibility = policy.visibility || "internal";

  const timeline = buildTimeline(incident_timeline, alerts, deploy_events);
  const unknowns = [];

  // ---- impact ----
  const impactHit = extractImpact(incident_timeline, alerts);
  const impact = impactHit
    ? { summary: impactHit.text, status: "known", evidence: impactHit.evidence }
    : { summary: "Impact not quantified in the provided evidence", status: "unknown" };
  if (impact.status === "unknown") {
    unknowns.push({
      question: "What was the customer/system impact (error rate, duration, affected users)?",
      reason: "No timeline event or alert carried a quantified impact signal",
    });
  }

  // ---- root-cause candidates: deploys that precede the first alert within window ----
  const tAlert = firstAlertMinute(alerts);
  const candidates = [];
  deploy_events.forEach((d, i) => {
    const m = toMinutes(atOf(d));
    if (m == null) return;
    if (tAlert == null) return;
    if (m <= tAlert && tAlert - m <= windowMin) {
      const svc = (d && typeof d === "object") ? (d.service || d.name || "service") : String(d);
      const ver = (d && typeof d === "object" && d.version) ? d.version : null;
      candidates.push({
        service: svc, version: ver, at: atOf(d),
        statement: `Deploy of ${svc}${ver ? `@${ver}` : ""} at ${atOf(d)} ${tAlert - m} min before the first alert`,
        evidence: [`deploy_events[${i}]`, `alerts (first at minute ${tAlert})`],
      });
    }
  });

  // ---- classify chat notes into confirming facts vs hedged hypotheses ----
  const hypotheses = [];
  const confirmingNotes = [];
  chat_notes.forEach((n, i) => {
    const text = noteText(n);
    if (!text) return;
    const hedged = HEDGE.test(text);
    const causal = CAUSE_CUE.test(text);
    if (!causal && !hedged) return; // ordinary status chatter, not a cause claim
    if (hedged) {
      hypotheses.push({ statement: text, evidence: `chat_notes[${i}]` });
    } else if (causal) {
      confirmingNotes.push({ statement: text, evidence: `chat_notes[${i}]` });
    }
  });

  // ---- decide root cause ----
  let root_cause;
  const distinctHypServices = new Set(
    hypotheses.map((h) => (h.statement.match(/([a-z0-9][a-z0-9_-]*)(?=\s*(?:v\d|@|deploy|change|release))/i) || [])[1])
      .filter(Boolean).map((s) => s.toLowerCase())
  );

  if (candidates.length === 1 && confirmingNotes.length >= 1) {
    // Single correlated deploy AND a declarative (non-hedged) note naming a cause.
    root_cause = {
      statement: candidates[0].statement,
      status: "confirmed",
      evidence: candidates[0].evidence.concat(confirmingNotes.map((c) => c.evidence)),
    };
  } else if (candidates.length === 1 && hypotheses.length === 0) {
    // Single correlated deploy, no competing speculation: probable, not confirmed.
    root_cause = {
      statement: candidates[0].statement,
      status: "probable",
      evidence: candidates[0].evidence,
    };
    unknowns.push({
      question: `Confirm that ${candidates[0].service} deploy is the cause (no declarative confirmation in chat notes)`,
      reason: "Correlation is circumstantial; no note asserts the cause",
    });
  } else if (candidates.length > 1 || distinctHypServices.size > 1) {
    // Multiple deploys correlate, or notes blame different services: conflicting.
    root_cause = { statement: "Undetermined: conflicting candidates", status: "unknown", evidence: [] };
    candidates.forEach((c) =>
      unknowns.push({ question: `Rule in/out candidate: ${c.statement}`, reason: "Multiple deploys correlate with the alert window", evidence: c.evidence }));
    hypotheses.forEach((h) =>
      unknowns.push({ question: `Hypothesis to verify: ${h.statement}`, reason: "Hedged speculation in chat, not corroborated", evidence: h.evidence }));
  } else {
    // No correlated deploy at all.
    root_cause = { statement: "Undetermined: no deploy correlates with the first alert", status: "unknown", evidence: [] };
    unknowns.push({
      question: "What triggered the incident? No deploy correlates with the first alert within the window",
      reason: tAlert == null ? "No timestamped alert to correlate against" : `No deploy within ${windowMin} min before the first alert`,
    });
    hypotheses.forEach((h) =>
      unknowns.push({ question: `Hypothesis to verify: ${h.statement}`, reason: "Hedged speculation in chat, not corroborated", evidence: h.evidence }));
  }

  // ---- action items (deterministic, evidence-linked, each names an owner lane) ----
  const action_items = [];
  if (root_cause.status === "confirmed" || root_cause.status === "probable") {
    action_items.push({
      title: `Add an automated rollback / deploy guard for ${candidates[0].service}`,
      owner_lane: "improve-skill",
      priority: "high",
      evidence: candidates[0].evidence,
    });
  }
  alerts.forEach((a, i) => {
    const name = (a && typeof a === "object" && a.name) ? a.name : eventText(a);
    action_items.push({
      title: `Ensure a runbook exists for alert "${name}"`,
      owner_lane: "policy-author",
      priority: "medium",
      evidence: [`alerts[${i}]`],
    });
  });
  if (impact.status === "unknown") {
    action_items.push({
      title: "Quantify customer impact (error rate, duration, affected users)",
      owner_lane: "ops",
      priority: "high",
      evidence: [],
    });
  }
  if (root_cause.status === "unknown") {
    action_items.push({
      title: "Investigate and confirm the root cause; resolve the open unknowns",
      owner_lane: "ops",
      priority: "high",
      evidence: [],
    });
  }

  // ---- postmortem status ----
  let status;
  if (root_cause.status === "confirmed" && impact.status === "known") status = "complete";
  else if (root_cause.status === "unknown") status = "needs_review";
  else status = "draft";

  // ---- publish proposal (gated) ----
  const rootOk = requireConfirmed
    ? root_cause.status === "confirmed"
    : (root_cause.status === "confirmed" || root_cause.status === "probable");
  const publishAllowed = rootOk && impact.status === "known";

  const publish_proposal = publishAllowed
    ? {
        action: "publish_postmortem",
        target: publishTarget,
        visibility,
        title: `Postmortem: ${candidates[0] ? candidates[0].service : "incident"} incident`,
        requires_approval: true,
        note: "Gated proposal. This skill posts nothing; a downstream send-as/doc-publisher executor acts on approval.",
        grounded_in: { root_cause_status: root_cause.status, impact_evidence: impact.evidence || null },
      }
    : null;

  // ---- summary (deterministic, cites counts) ----
  const summary =
    `Incident folded from ${incident_timeline.length} timeline event(s), ${alerts.length} alert(s), ` +
    `${deploy_events.length} deploy(s), and ${chat_notes.length} chat note(s). ` +
    `Root cause: ${root_cause.status} (${root_cause.statement}). ` +
    `Impact: ${impact.status}. ${unknowns.length} open unknown(s). ` +
    (publish_proposal ? "A gated publish proposal was drafted for approval." : "No publish proposal (evidence insufficient to publish).");

  const postmortem = {
    summary,
    timeline,
    impact,
    root_cause,
    status,
  };

  seal({ postmortem, unknowns, action_items, publish_proposal });
}

main();
