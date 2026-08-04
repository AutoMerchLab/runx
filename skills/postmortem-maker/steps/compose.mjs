// postmortem-maker / step 3 of 5: reconstruct the postmortem from the thread.
//
// Deterministic (no LLM): every timeline entry and every root-cause claim is
// derived from a specific event in the incident thread read by step 1, and
// carries that event's id, URL, and the quoted line it came from. The skill
// never invents a cause: a cause is only "confirmed" when exactly one candidate
// change is named in an unhedged causal statement. Competing candidates or
// hedged language ("might be", "not sure") leave the root cause unconfirmed,
// push the open questions into unknowns[], and withhold publication.
//
// This step decides; it delivers nothing. The authority record it produces is
// send-as shaped — principal, provider, channel, audience, content digest,
// consent basis, approval gate — because the canonical runx/send-as skill is a
// planning layer that never delivers. Actual delivery is step 4's job, and it
// only runs when this step says the postmortem is publishable.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

function refuse(reason) {
  console.error(reason);
  process.exit(1);
}

function seal(data) {
  console.log(JSON.stringify(data, null, 2));
  process.exit(0);
}

function parseInput() {
  const raw = process.env.RUNX_INPUTS_PATH
    ? readFileSync(process.env.RUNX_INPUTS_PATH, "utf8")
    : process.env.RUNX_INPUTS_JSON;
  if (!raw) return refuse("No input provided via RUNX_INPUTS_PATH or RUNX_INPUTS_JSON");
  try {
    return JSON.parse(raw);
  } catch (e) {
    return refuse("Invalid JSON input");
  }
}

function asObject(v) {
  if (v && typeof v === "object" && !Array.isArray(v)) return v;
  if (typeof v === "string" && v.trim()) {
    try {
      const p = JSON.parse(v);
      return p && typeof p === "object" && !Array.isArray(p) ? p : {};
    } catch (e) {
      return {};
    }
  }
  return {};
}

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value === undefined ? null : value);
}

function digestOf(value) {
  return `sha256:${createHash("sha256").update(canonical(value)).digest("hex")}`;
}

// The runtime hands step inputs and prior-step context in one JSON object; a
// context value may arrive unwrapped or under its step id.
function pick(input, ...names) {
  for (const n of names) {
    const direct = input[n];
    if (direct && typeof direct === "object") return direct;
    const ctx = input.context && input.context[n];
    if (ctx && typeof ctx === "object") return ctx;
  }
  return null;
}

const SENTENCE_SPLIT = /(?<=[.!?])\s+|\n+/;
const HEDGE = /\b(might|maybe|not sure|unsure|i suspect|suspect|possibly|perhaps|could be|unclear|probably|i think|seems like)\b|\?\s*$/i;
const CAUSAL = /\b(caused by|introduced|root cause|due to|because of|triggered by|regression from|broke|broken by)\b/i;
const MITIGATION = /\b(rolled back|rollback|reverted|mitigat|restored|recovered|resolved|back to (normal|baseline)|fix(ed)? deployed)\b/i;
const IMPACT = /\b(\d[\d,.]*\s*(%|percent|users|customers|requests|errors|minutes|orders)|error rate|latency|outage|downtime|degraded|failed)\b/i;
const ACTION = /\b(action item|action:|todo|follow[- ]up|we should|we need to|next step|owner:)\b/i;
const TIME_HINT = /\b([01]?\d|2[0-3]):[0-5]\d\b/;

// A cause candidate is the concrete, citable thing the thread blames: either a
// component paired with a version ("checkout-api v2.4.1"), or the subject a
// causal marker points at ("caused by a null pointer in the payment path").
const VERSIONED = /\b([a-z][a-z0-9._-]{2,40}?)[\s-]+(v?\d+\.\d+(?:\.\d+)?)\b/gi;
const CAUSE_PHRASE =
  /\b(?:caused by|introduced by|due to|because of|triggered by|regression from|broken by|root cause (?:was|is|:))\s+(.{4,90}?)(?=[.,;!?]|\s+(?:and|but|so|which|that)\b|$)/i;
const STOPWORDS = new Set([
  "the", "this", "that", "these", "those", "with", "from", "into", "over", "about", "when",
  "where", "which", "what", "have", "has", "had", "was", "were", "been", "being", "our",
  "your", "their", "its", "it's", "and", "but", "for", "not", "issue", "issues", "problem",
  "bug", "error", "errors", "failure", "failures", "some", "same", "then", "than", "there",
]);

function sentences(text) {
  return String(text || "")
    .split(SENTENCE_SPLIT)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

// A timeline entry has to be a statement someone actually made. Markdown
// headings, list bullets with no prose, and stray fragments are structure, not
// evidence, so they are read for signals but never quoted as timeline entries.
function isStatement(s) {
  const stripped = s.replace(/^[#>*\-\s]+/, "").trim();
  if (/^#{1,6}\s/.test(s)) return false;
  if (stripped.length < 25) return false;
  if (stripped.split(/\s+/).length < 4) return false;
  return true;
}

function quote(s) {
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > 220 ? `${t.slice(0, 217)}...` : t;
}

function cite(event, sentence) {
  return {
    event_id: event.id,
    author: event.author,
    at: event.at,
    url: event.url,
    quote: quote(sentence),
  };
}

function significantTokens(text) {
  return [
    ...new Set(
      String(text)
        .toLowerCase()
        .split(/[^a-z0-9._-]+/)
        .filter((t) => t.length >= 4 && !STOPWORDS.has(t))
    ),
  ];
}

function candidatesIn(sentence) {
  const found = new Map();

  // Preferred shape: a component paired with a shipped version.
  let m;
  VERSIONED.lastIndex = 0;
  while ((m = VERSIONED.exec(sentence)) !== null) {
    const name = m[1].toLowerCase().replace(/[^a-z0-9._-]/g, "");
    const version = m[2].toLowerCase();
    if (!name || name.length < 3) continue;
    if (STOPWORDS.has(name)) continue;
    found.set(`${name}@${version}`, {
      key: `${name}@${version}`,
      label: `${name} ${version}`,
      component: name,
      version,
      tokens: significantTokens(`${name} ${version}`),
    });
  }
  if (found.size > 0) return [...found.values()];

  // Fallback: whatever the causal marker points at, kept as the thread wrote it.
  const phrase = sentence.match(CAUSE_PHRASE);
  if (phrase) {
    const label = phrase[1].replace(/\s+/g, " ").replace(/^(a|an|the)\s+/i, "").trim();
    const tokens = significantTokens(label);
    if (label.length >= 4 && tokens.length > 0) {
      found.set(label.toLowerCase(), {
        key: label.toLowerCase(),
        label,
        component: null,
        version: null,
        tokens,
      });
    }
  }
  return [...found.values()];
}

// Two candidates describe the same cause when their significant tokens overlap
// ("the null pointer in the payment path" / "a null pointer deref in payments").
// Without this, one cause restated in two comments would look like two competing
// candidates and wrongly block publication.
function findMergeable(causal, cand) {
  for (const [key, entry] of causal) {
    if (key === cand.key) return key;
    if (entry.candidate.tokens.some((t) => cand.tokens.includes(t))) return key;
  }
  return null;
}

function main() {
  const input = parseInput();
  const incident = pick(input, "incident", "read_incident");
  if (!incident || !Array.isArray(incident.events) || incident.events.length === 0) {
    refuse("no incident events available from the read_incident step");
  }
  const outbox = pick(input, "outbox", "read_outbox");
  if (!outbox || typeof outbox.version !== "number") {
    refuse("no outbox stream version available from the read_outbox step");
  }
  const policy = asObject(input.postmortem_policy);
  const target = asObject(input.publish_target);
  const requireConfirmed = policy.require_confirmed_root_cause !== false;

  const timeline = [];
  const unknowns = [];
  const action_items = [];
  const causal = new Map(); // candidate key -> { candidate, confirmations[], hedges[] }
  let mitigation = null;

  for (const event of incident.events) {
    for (const s of sentences(event.text)) {
      const hedged = HEDGE.test(s);
      const isCausal = CAUSAL.test(s);
      const isMitigation = MITIGATION.test(s);
      const hasImpact = IMPACT.test(s);
      const hasTime = TIME_HINT.test(s);

      if ((isCausal || isMitigation || hasImpact || hasTime) && isStatement(s)) {
        timeline.push({
          at: event.at,
          statement: quote(s),
          kind: isCausal ? "cause_claim" : isMitigation ? "mitigation" : hasImpact ? "impact" : "context",
          confidence: hedged ? "reported" : "stated",
          evidence: cite(event, s),
        });
      }

      if (isMitigation && !mitigation) mitigation = { statement: quote(s), evidence: cite(event, s) };

      if (isCausal || hedged) {
        for (const cand of candidatesIn(s)) {
          const key = findMergeable(causal, cand) || cand.key;
          if (!causal.has(key)) causal.set(key, { candidate: cand, confirmations: [], hedges: [] });
          const entry = causal.get(key);
          if (isCausal && !hedged) entry.confirmations.push(cite(event, s));
          else entry.hedges.push(cite(event, s));
        }
      }

      if (ACTION.test(s)) {
        action_items.push({
          item: quote(s),
          status: "open",
          source: "incident thread",
          evidence: cite(event, s),
        });
      }
    }
  }

  if (timeline.length === 0) {
    refuse("no timeline could be reconstructed: the thread contains no timed, impact, cause, or mitigation statements");
  }

  const confirmedCandidates = [...causal.values()].filter((c) => c.confirmations.length > 0);
  const hedgedCandidates = [...causal.values()].filter((c) => c.confirmations.length === 0 && c.hedges.length > 0);

  let root_cause;
  if (confirmedCandidates.length === 1) {
    const c = confirmedCandidates[0];
    root_cause = {
      status: "confirmed",
      statement: `${c.candidate.label} is named as the cause in an unhedged statement in the incident thread`,
      component: c.candidate.component,
      version: c.candidate.version,
      citations: c.confirmations,
      corroborated_by_mitigation: mitigation ? mitigation.evidence : null,
    };
  } else {
    root_cause = {
      status: "unconfirmed",
      statement:
        confirmedCandidates.length > 1
          ? "more than one change is named as the cause; the thread does not settle between them"
          : hedgedCandidates.length > 0
            ? "every cause statement in the thread is hedged; no change is confirmed as the cause"
            : "no statement in the thread names a cause",
      component: null,
      version: null,
      citations: confirmedCandidates.concat(hedgedCandidates).flatMap((c) => c.confirmations.concat(c.hedges)),
      corroborated_by_mitigation: mitigation ? mitigation.evidence : null,
    };
    for (const c of confirmedCandidates.concat(hedgedCandidates)) {
      unknowns.push({
        question: `Was ${c.candidate.label} the cause of this incident?`,
        why_open:
          c.confirmations.length > 0
            ? "another change is named as the cause in the same thread"
            : "it is only named in a hedged statement",
        evidence: c.confirmations.concat(c.hedges),
      });
    }
    if (unknowns.length === 0) {
      unknowns.push({
        question: "What caused this incident?",
        why_open: "no statement in the thread names a candidate change",
        evidence: [],
      });
    }
    action_items.push({
      item: "Confirm the root cause with a reproduction or a change-correlation check before publishing this postmortem",
      status: "open",
      source: "postmortem-maker (root cause unconfirmed)",
      evidence: null,
    });
  }

  const postmortem = {
    incident_ref: incident.ref,
    title: incident.title || "Untitled incident",
    // `fetched_at` deliberately stays OUT of the postmortem body: the digest has
    // to bind the content, so the same thread re-read later digests the same and
    // a republish is an idempotent replay rather than a second delivery. When
    // the read happened is still on the record — step 1 seals it in the receipt,
    // and it is returned alongside as `fetched_at`.
    source: {
      ref: incident.ref,
      read_mode: incident.read_mode,
      events_read: incident.events_read,
      source_digest: incident.source_digest,
    },
    timeline,
    root_cause,
    mitigation,
    unknowns,
    action_items,
    status: root_cause.status === "confirmed" ? "publishable" : "needs_confirmation",
  };

  const publishable = requireConfirmed ? root_cause.status === "confirmed" : timeline.length > 0;
  const content_digest = digestOf(postmortem);
  const idempotency_key = `${incident.ref}|${content_digest}`;

  // send-as shaped authority record: who speaks, to whom, through which
  // channel, over which content, under which consent basis and approval gate.
  // Sealed here, executed (or withheld) by step 4 — never reported as sent by
  // this step.
  const send_plan = {
    schema: "send-as.plan.v1",
    principal: target.principal || "incident-review-bot",
    provider: "bundled-local-outbox",
    channel: target.channel || null,
    audience: target.audience || target.aggregate_id || null,
    classification: target.classification || "internal",
    visibility: target.visibility || "internal",
    content_digest,
    consent_basis: "internal incident review distribution",
    preflight: {
      timeline_entries: timeline.length,
      root_cause_status: root_cause.status,
      unknowns: unknowns.length,
      action_items: action_items.length,
      every_timeline_entry_cited: timeline.every((t) => t.evidence && t.evidence.event_id),
    },
    approval: {
      gate: requireConfirmed ? "confirmed_root_cause_required" : "none",
      decision: publishable ? "authorized" : "withheld",
      reason: publishable
        ? "root cause confirmed against cited thread evidence"
        : "root cause unconfirmed; the postmortem publishes nothing until it is settled",
    },
    status: publishable ? "authorized" : "withheld",
  };

  seal({
    postmortem,
    fetched_at: incident.fetched_at,
    unknowns,
    action_items,
    root_cause_status: root_cause.status,
    timeline_count: timeline.length,
    publishable,
    send_plan,
    content_digest,
    idempotency_key,
    expected_version: outbox.version,
    message: {
      kind: "postmortem",
      incident_ref: incident.ref,
      title: postmortem.title,
      content_digest,
      body: postmortem,
    },
  });
}

main();
