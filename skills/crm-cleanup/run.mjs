// crm-cleanup: extract structured CRM field updates from a natural-language
// meeting transcript, and flag actionable-but-unextractable lines for review.
//
// Design notes:
//   - Extraction is deterministic (heuristics over regex + small synonym
//     vocabularies), not an LLM, so harness runs seal reproducibly.
//   - A field is matched by strategies chosen from its inferred type
//     (money / enum / action / generic), each of which understands several
//     natural phrasings, not only the literal "<field> is <value>" template.
//   - When a sentence clearly refers to a schema field and asserts a change
//     but no value can be extracted, the line is reported in `needs_review`
//     instead of being silently dropped, so a downstream human can triage it.

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

function parseSchema(schemaRaw) {
  if (schemaRaw && typeof schemaRaw === "object") return schemaRaw;
  try {
    return JSON.parse(schemaRaw);
  } catch (e) {
    return refuse("Invalid crm_schema JSON");
  }
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

// Does this sentence talk about `field` at all (name, humanized, or topic word)?
function referencesField(sentence, field, type) {
  const lower = sentence.toLowerCase();
  if (lower.includes(humanize(field))) return true;
  if (lower.includes(field.toLowerCase())) return true;
  if (type === "money" && /\b(budget|amount|price|cost|deal|revenue|spend|worth|quota)\b/.test(lower)) return true;
  if (type === "enum" && /\b(status|stage|priority|tier|phase|pipeline)\b/.test(lower)) return true;
  if (type === "action" && /\b(next step|next steps|follow ?up|action item|to-?do)\b/.test(lower)) return true;
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

  const allowedFields = crm_schema.fields;
  if (!Array.isArray(allowedFields)) return refuse("crm_schema.fields must be an array");
  if (allowedFields.length === 0) return refuse("No fields defined in schema");

  const sentences = splitSentences(transcript);

  const takeaways = [];
  const field_updates = {};
  const needs_review = [];
  const seenReview = new Set();

  for (const field of allowedFields) {
    const type = fieldType(field);
    const human = humanize(field);

    let value = null;
    // 1) Prefer a sentence that both references the field and yields a value.
    for (const s of sentences) {
      if (!referencesField(s, field, type)) continue;
      const v = extractForType(s, field, type);
      if (v) { value = v; break; }
    }
    // 2) Fall back to any sentence yielding a value for this field type
    //    (e.g. a bare money mention without the word "budget").
    if (!value) {
      for (const s of sentences) {
        const v = extractForType(s, field, type);
        if (v) { value = v; break; }
      }
    }

    if (value) {
      field_updates[field] = value;
      takeaways.push(`Identified ${human}: ${value}`);
      continue;
    }

    // 3) No value: flag actionable-but-unextractable references for review.
    for (const s of sentences) {
      if (!referencesField(s, field, type)) continue;
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

  const write_proposal = Object.keys(field_updates).length > 0;

  seal({ takeaways, field_updates, needs_review, write_proposal });
}

main();
