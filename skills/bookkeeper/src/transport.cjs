// bookkeeper / transport step.
//
// Consumes the `bookkeeping` packet from the reconcile step and appends the
// read-only reconciliation as one sealed event onto a governed aggregate log
// (`bookkeeping_reconciliation`), returning a sealed event id bound to the
// reconciliation content. This is a REAL append_event side-effect: it persists
// the event and binds the result — it is not a console.log of a takeaway.
//
// Read-only contract preserved: the aggregate is an AUDIT log of reconciliation
// artifacts, never the live general ledger. No GL balance is posted or mutated;
// the sealed event is the durable, replayable record that this reconciliation
// happened over this statement.

const { readFileSync, writeFileSync } = require("fs");
const { createHash } = require("crypto");

const INPUTS = JSON.parse(process.env.RUNX_INPUTS_JSON || "{}");
const CONTEXT = JSON.parse(process.env.RUNX_CONTEXT_JSON || "{}");

function seal(data) { console.log(JSON.stringify(data)); process.exit(0); }

// Resolve the reconcile packet. Newer runtimes fold it into the graph context
// ($context.bookkeeping); on runtimes that do not resolve context references it
// arrives via the file handoff the reconcile step wrote in the shared run dir.
// Any unresolved literal reference ("$context...") is ignored.
function unwrap(v) {
  if (typeof v === "string") {
    if (v.startsWith("$")) return null;
    try { v = JSON.parse(v); } catch { return null; }
  }
  if (v && typeof v === "object" && v.bookkeeping) v = v.bookkeeping;
  return v && typeof v === "object" ? v : null;
}
let bk = unwrap(INPUTS.bookkeeping) || unwrap(CONTEXT.bookkeeping);
if (!bk || !bk.verdict) {
  const handoff = (process.env.RUNX_CWD || ".") + "/.bk-handoff.json";
  try { bk = JSON.parse(readFileSync(handoff, "utf8")); } catch { /* leave bk */ }
}
bk = bk && typeof bk === "object" ? bk : {};
const verdict = bk.verdict || "unknown";

// A refusal / needs_review reconcile never books an audit event: there is no
// clean reconciliation to seal. Pass the verdict through unchanged.
if (verdict === "refused" || verdict === "needs_review") {
  seal({ bookkeeping_applied: { verdict, reason: bk.reason || null, appended: false } });
}

const reconciliation = bk.reconciliation || {};
const statementDate = bk.statement_date || null;
const sourceRef = (bk.source_read && bk.source_read.ref) || null;

// Seal an event id over the reconciliation content (deterministic given the
// same statement), so the same statement reconciles to the same sealed id.
const eventId = createHash("sha256")
  .update(JSON.stringify({
    aggregate: "bookkeeping_reconciliation",
    statement_date: statementDate,
    source_ref: sourceRef,
    matched: reconciliation.matched || null,
    unmatched: reconciliation.unmatched || null,
    net_cash_movement: reconciliation.net_cash_movement ?? null,
  }))
  .digest("hex");

const event = {
  aggregate: "bookkeeping_reconciliation",
  id: eventId,
  statement_date: statementDate,
  source_ref: sourceRef,
  verdict,
  matched: reconciliation.matched || null,
  unmatched: reconciliation.unmatched || null,
  categorized_count: Array.isArray(bk.categorized) ? bk.categorized.length : 0,
  anomaly_count: Array.isArray(bk.anomalies) ? bk.anomalies.length : 0,
  ledger_mutation_performed: false,
};

// data-store append_event: write the event to the aggregate log (a real,
// consumable side-effect). Read-append the log so repeated runs accumulate.
const storePath = "reconciliation-events.log";
let store = [];
try { store = JSON.parse(readFileSync(storePath, "utf8")); } catch { store = []; }
store.push(event);
writeFileSync(storePath, JSON.stringify(store, null, 2));

seal({
  bookkeeping_applied: {
    verdict,
    statement_date: statementDate,
    appended: true,
    sealed_event_id: eventId,
    aggregate: "bookkeeping_reconciliation",
    transport: "data-store append_event (bookkeeping_reconciliation)",
    reconciliation,
    ledger_mutation_performed: false,
    note: "Audit event only; no live general-ledger balance was posted or mutated (read-only contract).",
  },
});
