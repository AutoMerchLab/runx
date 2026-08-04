// crm-cleanup / step 2 of 3: reconcile.
//
// Reads the CRM records that step 1 (`data.read_projection`) pulled out of the
// governed data store, extracts the field values asserted by the transcript,
// and emits ONLY the fields whose asserted value actually differs from what the
// store already holds. The decision this step returns is what step 3
// (`data.append_event`) executes under compare-and-set, so a no-op reconcile
// means no event is appended at all.
//
// Extraction is deterministic (regex heuristics plus small synonym
// vocabularies), never an LLM, so every run seals reproducibly. A line that
// clearly refers to a schema field and asserts a change but yields no readable
// value is reported under `needs_review` rather than being silently dropped.

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

function parseSchema(schemaRaw) {
  if (schemaRaw && typeof schemaRaw === "object") return schemaRaw;
  try {
    return JSON.parse(schemaRaw);
  } catch (e) {
    return refuse("Invalid crm_schema JSON");
  }
}

// The runtime hands step inputs and prior-step context in one JSON object; the
// projection may arrive either as the step id or already unwrapped, so look in
// the documented places rather than assuming one shape.
function pickProjection(input) {
  const candidates = [
    input.projection,
    input.read_records,
    input.context && input.context.projection,
    input.context && input.context.read_records,
  ];
  for (const c of candidates) {
    if (c && typeof c === "object") return c;
  }
  return {};
}

// Step 1 returns { version, record, ... }: the projected field values and the
// stream version the write step will compare-and-set against. A stream that
// does not exist yet projects to version 0 and an empty record, which is a
// legitimate first-write case rather than an error.
function readRecords(projection) {
  const version =
    typeof projection.version === "number"
      ? projection.version
      : typeof projection.after_version === "number"
        ? projection.after_version
        : 0;

  let record = {};
  if (projection.record && typeof projection.record === "object") {
    record = { ...projection.record };
  }
  // Tolerate an event-list projection shape as well, so the reconcile does not
  // silently treat stored fields as new if the read step is ever swapped out.
  const events = Array.isArray(projection.events) ? projection.events : [];
  for (const entry of events) {
    const payload =
      (entry && entry.payload) || (entry && entry.event && entry.event.payload) || {};
    const fields = payload.fields || null;
    if (fields && typeof fields === "object") record = { ...record, ...fields };
  }
  return { version, record };
}

// ---- text helpers ----

function splitSentences(text) {
  return text
    .split(/(?<=[.!?])\s+|\n+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function humanize(field) {
  return field.replace(/[_\-]+/g, " ").trim().toLowerCase();
}

// Escape regex metacharacters so an untrusted crm_schema field name can never
// crash the RegExp constructor or inject an unintended pattern.
function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Infer how to read a field from its name so the skill stays generic over an
// arbitrary crm_schema while still handling common CRM field shapes well.
function fieldType(field) {
  const f = field.toLowerCase();
  if (/(budget|amount|value|price|cost|revenue|deal.?size|spend|mrr|arr|acv|quota|contract)/.test(f)) return "money";
  if (/(status|stage|priority|tier|state|phase|disposition)/.test(f)) return "enum";
  if (/(next.?step|action|follow.?up|task|todo|next)/.test(f)) return "action";
  return "generic";
}

const STATUS_VOCAB = [
  "closed won", "closed lost", "on hold", "on-hold",
  "new", "contacted", "engaged", "qualified", "unqualified", "disqualified",
  "proposal", "negotiation", "demo", "trial", "won", "lost", "churned",
  "active", "inactive", "nurture", "pending", "committed", "stalled",
];

const CHANGE_CUE =
  /\b(is|are|was|were|be|=|:|set|sets|setting|move|moved|moving|mark|marked|marking|now|update|updated|updating|change|changed|changing|bump|bumped|need to|needs to|should|will|going to|gonna|next step|next steps|follow ?up|action item|let'?s|plan to|planning to|schedule|send|put)\b/i;

const NEGATION =
  /\b(nothing (?:changed|to update)|no (?:change|update|updates)|didn'?t change|unchanged|on track|no news)\b/i;

// ---- money ----

function extractMoney(sentence) {
  // $50k, $50,000, 50k, 50 grand, USD 50000, 50 thousand, 1.2m
  const re =
    /(?:\$|usd|eur|gbp)?\s?(\d[\d,]*(?:\.\d+)?)\s?(k|m|thousand|thousands|grand|million|millions|mm)?\b/gi;
  let best = null;
  let m;
  while ((m = re.exec(sentence)) !== null) {
    const num = m[1];
    let suffix = (m[2] || "").toLowerCase();
    // Ignore bare small integers with no currency/suffix context (e.g. "2 calls").
    const hadCurrency = /\$|usd|eur|gbp/i.test(m[0]);
    if (!suffix && !hadCurrency) {
      // require an explicit money cue near the number
      const around = sentence.slice(Math.max(0, m.index - 24), m.index + m[0].length + 8).toLowerCase();
      if (!/(budget|dollar|price|cost|deal|revenue|spend|worth|contract|value)/.test(around)) continue;
    }
    let normalized;
    if (/^(k|thousand|thousands|grand)$/.test(suffix)) normalized = `$${num}k`;
    else if (/^(m|million|millions|mm)$/.test(suffix)) normalized = `$${num}m`;
    else normalized = `$${num}`;
    // Prefer the first / most explicit money mention.
    if (!best) best = normalized;
  }
  return best;
}

// ---- enum / status ----

function extractEnum(sentence) {
  const lower = sentence.toLowerCase();
  for (const term of STATUS_VOCAB) {
    const t = term.replace(/[-\/\\^$*+?.()|[\]{}]/g, "\\$&");
    // Require an assignment/transition cue right before the status term so a
    // temporal mention ("after the demo") is not misread as a status value.
    const cued = new RegExp(
      `\\b(?:to|as|into|now|status(?:\\s+is|:)?|stage(?:\\s+is|:)?|` +
        `mark(?:ed)?(?:\\s+as)?|mov\\w+(?:\\s+\\w+){0,3}?\\s+to|set(?:\\s+\\w+){0,3}?\\s+to|=|:)` +
        `\\s+${t}\\b`,
      "i"
    );
    if (cued.test(lower)) return term;
  }
  return null;
}

// ---- action / next step ----

function extractAction(sentence) {
  const patterns = [
    /next steps?\s*(?:is|are|will be|:|-)?\s*(?:to\s+)?(.+)/i,
    /(?:the\s+)?(?:agreed|plan)\s+(?:is|was)?\s*(?:to\s+)?(.+)/i,
    /follow[\s-]?up\s+(?:with\s+|by\s+)?(.+)/i,
    /action items?\s*(?::|-)?\s*(.+)/i,
    /(?:i'?ll|we'?ll|i will|we will|i'?m going to|we'?re going to|let'?s|need to|needs to|going to|plan to|planning to)\s+(.+)/i,
    /(?:schedule|send|book|set up|arrange|prepare|draft)\s+(.+)/i,
  ];
  for (const re of patterns) {
    const m = sentence.match(re);
    if (m && m[1]) {
      // Trim at a hard clause break; keep it readable.
      let v = m[1].trim().replace(/[.]+$/, "");
      v = v.split(/\s+(?:and then|then again|;)\s+/i)[0].trim();
      if (v.length >= 2) return v;
    }
  }
  return null;
}

// ---- generic "<field> is/of/: <value>" ----

function extractGeneric(sentence, fieldName) {
  // Escape first, then allow the field's word separators to match flexibly.
  const name = escapeRe(fieldName).replace(/[-_]/g, "[\\s_-]?");
  const patterns = [
    new RegExp(`set\\s+${name}\\s+to\\s+\\$?([^.;,]+)`, "i"),
    new RegExp(`${name}\\s*(?:is|are|=|:)\\s*\\$?([^.;,]+)`, "i"),
    new RegExp(`${name}\\s+of\\s+\\$?([^.;,]+)`, "i"),
  ];
  for (const re of patterns) {
    const m = sentence.match(re);
    if (m && m[1]) {
      // Stop the value at a conjunction so it does not swallow the next clause.
      const v = m[1].trim().split(/\s+(?:and|but|however|;)\s+/i)[0].trim();
      if (v.length >= 1) return v;
    }
  }
  return null;
}

// Does the sentence name this specific field (own name or humanized form)?
// Word-boundary matched so a short field like "arr" does not hit "narrow".
function mentionsName(sentence, field) {
  return [humanize(field), field.toLowerCase()].some(
    (c) => c && new RegExp(`\\b${escapeRe(c)}\\b`, "i").test(sentence)
  );
}

// Does the sentence mention this field TYPE's topic word (budget/status/...)?
// This is ambiguous when several fields share a type, so callers only trust it
// when the field is the sole one of its type.
function mentionsTypeTopic(sentence, type) {
  const lower = sentence.toLowerCase();
  if (type === "money") return /\b(budget|amount|price|cost|deal|revenue|spend|worth|quota)\b/.test(lower);
  if (type === "enum") return /\b(status|stage|priority|tier|phase|pipeline)\b/.test(lower);
  if (type === "action") return /\b(next step|next steps|follow ?up|action item|to-?do)\b/.test(lower);
  return false;
}

function extractForType(sentence, field, type) {
  if (type === "money") return extractMoney(sentence);
  if (type === "enum") return extractEnum(sentence);
  if (type === "action") return extractAction(sentence);
  return extractGeneric(sentence, field);
}

function main() {
  const parsed = parseInput();
  const transcript = typeof parsed.transcript === "string" ? parsed.transcript : "";
  const crm_schema = parseSchema(parsed.crm_schema != null ? parsed.crm_schema : "{}");
  const handle = (parsed.source_handle && typeof parsed.source_handle === "object") ? parsed.source_handle : {};

  const allowedFields = crm_schema.fields;
  if (!Array.isArray(allowedFields)) return refuse("crm_schema.fields must be an array");
  if (allowedFields.length === 0) return refuse("No fields defined in schema");

  const { version, record } = readRecords(pickProjection(parsed));
  const sentences = splitSentences(transcript);

  // How many fields share each inferred type? A type-topic / any-sentence match
  // is only safe to attribute when a field is the SOLE one of its type,
  // otherwise two same-type fields would both bind to the first value.
  const typeCounts = {};
  for (const f of allowedFields) {
    const t = fieldType(f);
    typeCounts[t] = (typeCounts[t] || 0) + 1;
  }

  const takeaways = [];
  const field_updates = {};
  const needs_review = [];
  const seenReview = new Set();

  for (const field of allowedFields) {
    const type = fieldType(field);
    const human = humanize(field);
    const uniqueType = typeCounts[type] === 1;
    // The field is "referred to" by a sentence that names it, or — only when it
    // is the sole field of its type — mentions the field type's topic word.
    const refers = (s) => mentionsName(s, field) || (uniqueType && mentionsTypeTopic(s, type));

    let value = null;
    // 1) A sentence that refers to the field and yields a value.
    for (const s of sentences) {
      if (!refers(s)) continue;
      const v = extractForType(s, field, type);
      if (v) { value = v; break; }
    }
    // 2) Fall back to any sentence yielding a value for this type ONLY when this
    //    field is unique for its type (e.g. a bare "$75k" with no field name).
    if (!value && uniqueType) {
      for (const s of sentences) {
        const v = extractForType(s, field, type);
        if (v) { value = v; break; }
      }
    }

    if (value) {
      const current = record[field];
      // Reconciliation, not blind extraction: a value the store already holds
      // is not an update, so it never reaches the write step.
      if (current !== undefined && String(current) === String(value)) {
        takeaways.push(`${human} already matches the record: ${value}`);
        continue;
      }
      field_updates[field] = value;
      takeaways.push(
        current === undefined
          ? `Identified ${human}: ${value} (no value on record)`
          : `Identified ${human}: ${value} (record holds ${current})`
      );
      continue;
    }

    // 3) No value: flag actionable-but-unextractable references for review.
    for (const s of sentences) {
      if (!refers(s)) continue;
      if (NEGATION.test(s)) continue;
      if (!CHANGE_CUE.test(s)) continue;
      const key = `${field}::${s}`;
      if (seenReview.has(key)) continue;
      seenReview.add(key);
      needs_review.push({
        field,
        line: s,
        reason: `References ${human} with an actionable cue but no clear value could be extracted`,
      });
    }
  }

  const changedFields = Object.keys(field_updates);
  const changed = changedFields.length > 0;

  const before = {};
  const after = {};
  for (const f of allowedFields) {
    if (record[f] !== undefined) before[f] = record[f];
  }
  Object.assign(after, before, field_updates);

  const aggregate = handle.aggregate_id || "unknown";
  // Stable retry key: same records + same decision at the same version must
  // reuse one key, a different decision must not.
  const idempotency_key = `crm-cleanup:${aggregate}:v${version}:${createHash("sha256")
    .update(JSON.stringify({ aggregate, version, field_updates }))
    .digest("hex")
    .slice(0, 16)}`;

  // The write step is guarded on `changed`, so the no-op event below is never
  // appended: it is emitted only so the receipt records, in typed form, what
  // the reconcile decided and why nothing was written.
  const event = changed
    ? {
        type: "crm.records.updated",
        effect_family: "crm",
        operation: "update",
        payload: {
          aggregate_id: aggregate,
          fields: after,
          field_updates,
          before,
          changed_fields: changedFields,
          source: "crm-cleanup",
        },
      }
    : {
        type: "crm.records.unchanged",
        effect_family: "crm",
        operation: "noop",
        payload: {
          aggregate_id: aggregate,
          fields: before,
          reason: "no transcript assertion differed from the records read from the store",
          source: "crm-cleanup",
        },
      };

  seal({
    takeaways,
    field_updates,
    needs_review,
    changed,
    expected_version: version,
    idempotency_key,
    event,
    before,
  });
}

main();
