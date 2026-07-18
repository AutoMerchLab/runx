// bookkeeper / reconcile step.
//
// Reads a REAL transaction statement from an external source AT RUNTIME (never a
// bundled fixture): the U.S. Treasury Fiscal Data "Deposits & Withdrawals of
// Operating Cash" (DTS) API, a public, authoritative daily cash statement. Each
// DTS line is a real transaction — Deposits are receipts (credit), Withdrawals
// are payments (debit), transaction_catg is the line description, and
// transaction_today_amt is the amount (USD millions).
//
// It then does honest double-entry bookkeeping over that statement: it
// categorizes every transaction to an EXISTING account in chart_of_accounts
// (with a confidence and a reason, never inventing a GL account), flags
// anomalies, and emits a read-only reconciliation{matched, unmatched} against
// the prior_period baseline. It books nothing to a live ledger.
//
// Design notes:
//   - Deterministic (keyword + amount-sign rules over the fetched statement, no
//     LLM), so a run over an immutable historical statement date seals
//     reproducibly in the harness.
//   - A transaction is categorized only to an account whose normal balance side
//     matches the transaction direction (Deposits->credit, Withdrawals->debit)
//     AND whose keyword set hits the line description. No match -> the line is
//     left UNMATCHED (an anomaly), never forced onto an invented account.
//   - When the ambiguous share of the batch is too high to book safely, the step
//     seals a refusal receipt whose verdict is needs_review, listing the lines a
//     human must resolve. That is the "ambiguous -> needs_review" path.

const https = require("https");
const { readFileSync, writeFileSync } = require("fs");

// ---- io helpers -------------------------------------------------------------

function parseInput() {
  let raw = process.env.RUNX_INPUTS_JSON;
  if (!raw && process.env.RUNX_INPUTS_PATH) {
    try { raw = readFileSync(process.env.RUNX_INPUTS_PATH, "utf8"); } catch { /* fall through */ }
  }
  if (!raw) return {};
  try { return JSON.parse(raw); } catch { return {}; }
}

function seal(data) { console.log(JSON.stringify(data)); process.exit(0); }
// A governed refusal (read-only violation or an ambiguous batch that cannot be
// booked with confidence) is a SEALED refusal, exit code 78 — the skill
// deliberately declined to book, and that decision is itself the receipt.
function sealRefusal(data) { console.log(JSON.stringify(data)); process.exitCode = 78; }
function hardError(msg) { console.error(msg); process.exit(1); }

// Inputs may arrive native (typed runner) or JSON-string (harness inline). Both
// are normalized so the same code path serves harness and dogfood.
function asArray(v) {
  if (Array.isArray(v)) return v;
  if (typeof v === "string" && v.trim()) { try { const p = JSON.parse(v); return Array.isArray(p) ? p : []; } catch { return []; } }
  return [];
}
function asObject(v) {
  if (v && typeof v === "object" && !Array.isArray(v)) return v;
  if (typeof v === "string" && v.trim()) { try { const p = JSON.parse(v); return (p && typeof p === "object" && !Array.isArray(p)) ? p : {}; } catch { return {}; } }
  return {};
}

function getJson(url) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { "user-agent": "runx-bookkeeper" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        return getJson(res.headers.location).then(resolve, reject);
      }
      if (res.statusCode !== 200) { reject(new Error("source_http_" + res.statusCode)); return; }
      let body = "";
      res.on("data", (c) => (body += c));
      res.on("end", () => { try { resolve(JSON.parse(body)); } catch (e) { reject(e); } });
    }).on("error", reject);
  });
}

// ---- default chart of accounts ---------------------------------------------
//
// A default double-entry chart for a U.S. Treasury operating-cash statement.
// `side` is the account's normal balance: credit accounts take receipts
// (Deposits), debit accounts take payments (Withdrawals). Every account here is
// a real GL line; categorization may only bind a transaction to one of these.
// Agency tokens shared by the two broad "any department" accounts (one per
// balance side). Kept short on purpose: a more specific line (e.g. a benefit or
// tax keyword) outscores these by matched-keyword length, so departmental misc
// lines land here while program lines route to their specific account.
const AGENCY_TOKENS = [
  "agriculture", "usda", "commerce", "(doc)", "education", "(ed)", "energy", "doe",
  "interior", "doi", "labor", "(dol)", "transportation", "dot", "housing", "hud",
  "justice", "doj", "of state", "(dos)", "defense", "dod", "homeland", "dhs",
  "health & human", "hhs", "epa", "environmental", "general services", "gsa",
  "personnel mgmt", "opm", "social security admin", "postal", "usps",
  "communications", "fcc", "deposit insurance", "fdic", "securities and exchange",
  "sec)", "small business", "sba", "science foundation", "nsf", "credit union",
  "ncua", "railroad retirement board", "assistance programs", "iap", "treasury",
  "treas", "mint", "engraving", "financing bank", "nasa", "monetary fund", "imf",
  "army corps", "district of columbia", "judicial", "legislative", "library of congress",
  "independent agencies", "thrift savings", "export-import", "trade commission", "ftc",
  "development finance", "dfc", "emergency mgmt", "fema", "seized assets",
  "universal service", "reserve earnings", "gse proceeds", "comptroller",
];

const DEFAULT_COA = [
  // --- credit side: receipts (Deposits) -----------------------------------
  { code: "4010", name: "Tax Receipts", type: "revenue", side: "credit",
    keywords: ["taxes -", "tax refunds", "withheld individual", "fica", "seca",
      "futa", "federal unemployment", "excise", "estate and gift", "estate, gift",
      "corporate income", "railroad retirement", "irs collected"] },
  { code: "4015", name: "Customs & Border Receipts", type: "revenue", side: "credit",
    keywords: ["customs duties", "customs & border", "border protection", "(cbp)"] },
  { code: "4040", name: "Public Debt Cash Issues", type: "liability", side: "credit",
    keywords: ["public debt cash issue", "debt cash issue", "cash issues"] },
  { code: "4020", name: "Agency Program Receipts", type: "revenue", side: "credit",
    keywords: AGENCY_TOKENS.concat(["marketplace receipts", "medicare premiums",
      "veterans affairs", "(va)", "foreign military sales", "state unemployment",
      "pension benefit guaranty", "gas and oil lease", "land and minerals",
      "fish and wildlife", "water and science"]) },
  // --- debit side: payments (Withdrawals) ---------------------------------
  { code: "5010", name: "Benefit & Entitlement Payments", type: "expense", side: "debit",
    keywords: ["benefits payments", "ssa - benefits", "supplemental security",
      "va - benefits", "veterans affairs", "medicaid", "medicare", "prescription drugs",
      "marketplace payments", "hospital insr", "med insr trust", "supp nutrition",
      "child nutrition", "(snap)", "(wic)", "temp assistance", "needy families",
      "children and families", "unemployment benefits", "military retirement",
      "civil serv retirement", "rrb - benefit", "indian health", "health resources",
      "national institutes", "disease control", "grants to states"] },
  { code: "5015", name: "Salaries & Payroll", type: "expense", side: "debit",
    keywords: ["federal salaries", "military active duty pay", "active duty"] },
  { code: "5030", name: "Debt Service & Redemptions", type: "expense", side: "debit",
    keywords: ["interest on treasury", "public debt cash redemp", "debt cash redemp",
      "resolution funding"] },
  { code: "5040", name: "Tax Refunds Payable", type: "expense", side: "debit",
    keywords: ["individual tax refunds", "business tax refunds", "tax refunds",
      "irs refunds", "claims judgments"] },
  { code: "5020", name: "Agency Operating Payments", type: "expense", side: "debit",
    keywords: AGENCY_TOKENS.concat(["federal aviation", "federal highway",
      "federal transit", "federal railroad", "economic recovery", "military active",
      "insurance payment", "aviation administration"]) },
];

// ---- statement normalization ------------------------------------------------

// Turn one fetched DTS row into a normalized transaction. Deposits credit the
// books (money in); Withdrawals debit them (money out).
function normalizeDtsRow(row, i) {
  const type = String(row.transaction_type || "").trim();
  const direction = /deposit/i.test(type) ? "credit" : /withdraw/i.test(type) ? "debit" : "unknown";
  const amount = Number(row.transaction_today_amt);
  return {
    line: i,
    date: row.record_date || null,
    description: String(row.transaction_catg || row.transaction_catg_desc || "").trim(),
    direction,
    amount: Number.isFinite(amount) ? amount : null,
    raw_type: type,
  };
}

// Accept an already-normalized inline transactions[] array too, so a caller can
// pass a batch directly. Each item may be {description, direction|type, amount}.
function normalizeInlineTx(t, i) {
  const type = String(t.type || t.transaction_type || "").trim();
  let direction = t.direction || (/deposit|credit|receipt/i.test(type) ? "credit"
    : /withdraw|debit|payment/i.test(type) ? "debit" : "unknown");
  const amount = Number(t.amount != null ? t.amount : t.transaction_today_amt);
  return {
    line: i,
    date: t.date || t.record_date || null,
    description: String(t.description || t.transaction_catg || t.memo || "").trim(),
    direction,
    amount: Number.isFinite(amount) ? amount : null,
    raw_type: type || direction,
  };
}

// ---- categorization ---------------------------------------------------------

function countHits(desc, keywords) {
  const d = desc.toLowerCase();
  let best = "";
  let hits = 0;
  for (const kw of keywords) {
    if (d.includes(kw)) { hits += 1; if (kw.length > best.length) best = kw; }
  }
  return { hits, best };
}

// Categorize one transaction against the chart. Only accounts on the SAME
// balance side as the transaction direction are eligible, so a receipt never
// lands on an expense account. Returns null (leave unmatched) when nothing hits
// — the skill never invents an account.
function categorize(tx, coa) {
  if (tx.direction === "unknown") return null;
  const eligible = coa.filter((a) => a.side === tx.direction);
  // Score every eligible account: more keyword hits win, ties broken by the
  // longest (most specific) matched keyword. A short agency token like "hhs"
  // therefore loses to a specific phrase like "grants to states" / "medicaid".
  const scored = [];
  for (const acct of eligible) {
    const { hits, best: kw } = countHits(tx.description, acct.keywords);
    if (hits === 0) continue;
    scored.push({ acct, hits, kw, score: hits * 100 + kw.length });
  }
  if (scored.length === 0) return null;
  scored.sort((a, b) => b.score - a.score);
  const best = scored[0];
  const runnerUp = scored[1] || null;
  // Genuinely ambiguous only on a near-tie: the runner-up matches as many
  // keywords AND its longest match is within one char of the winner's.
  const ambiguous = !!runnerUp && runnerUp.hits === best.hits
    && Math.abs(runnerUp.kw.length - best.kw.length) <= 1;
  let confidence = Math.min(0.98, 0.62 + 0.1 * best.hits + best.kw.length / 120);
  if (ambiguous) confidence = Math.min(confidence, 0.5);
  return {
    account: { code: best.acct.code, name: best.acct.name, type: best.acct.type },
    direction: tx.direction,
    confidence: Number(confidence.toFixed(2)),
    reason: `matched keyword "${best.kw}" in "${tx.description}" -> ${best.acct.code} ${best.acct.name} (${tx.direction})`,
    ambiguous,
  };
}

// ---- main -------------------------------------------------------------------

async function main() {
  const input = parseInput();

  // A live-ledger booking framing is refused: this skill is read-only.
  if (input.book === true || input.mutate === true || input.post_to_ledger === true) {
    sealRefusal({ bookkeeping: { verdict: "refused",
      reason: "bookkeeper is a read-only reconciliation skill; it books nothing to a live ledger. Set book=false." } });
    return;
  }

  const coa = asArray(input.chart_of_accounts).length ? asArray(input.chart_of_accounts) : DEFAULT_COA;
  const priorPeriod = asObject(input.prior_period);
  const reviewThreshold = Number.isFinite(input.review_threshold) ? input.review_threshold : 0.5;

  // --- read the statement from a real source at runtime -------------------
  let statementRef = input.statement_url || input.data_source_ref || null;
  const statementDate = input.statement_date || null;
  // Pin the fetch to one immutable statement date so a historical run seals
  // reproducibly. Only inject when the caller has not already supplied a
  // record_date filter (checked on the filter= clause, not the fields= list).
  if (statementRef && statementDate && statementRef.includes("fiscaldata.treasury.gov")
      && !/filter=[^&]*record_date/.test(statementRef)) {
    const sep = statementRef.includes("?") ? "&" : "?";
    statementRef += `${sep}filter=record_date:eq:${statementDate}`;
  }

  let transactions = [];
  let source_read = null;
  if (statementRef) {
    let payload;
    try { payload = await getJson(statementRef); }
    catch (e) { hardError("statement fetch failed: " + e.message); }
    const rows = Array.isArray(payload) ? payload : (payload.data || payload.records || []);
    transactions = rows.map(normalizeDtsRow);
    source_read = { ref: statementRef, kind: "runtime-web-fetch", records_read: transactions.length };
  } else {
    const inline = asArray(input.transactions);
    transactions = inline.map(normalizeInlineTx);
    source_read = { ref: "inline:transactions", kind: "inline-batch", records_read: transactions.length };
  }

  if (transactions.length === 0) hardError("no transactions read from statement source");

  // --- categorize + collect anomalies ------------------------------------
  const categorized = [];
  const anomalies = [];
  let ambiguousCount = 0;

  for (const tx of transactions) {
    if (tx.amount == null) {
      anomalies.push({ type: "unparseable_amount", line: tx.line, description: tx.description,
        reason: "transaction amount is missing or non-numeric" });
    }
    if (tx.direction === "unknown") {
      anomalies.push({ type: "unknown_direction", line: tx.line, description: tx.description,
        reason: `transaction type "${tx.raw_type}" is neither a deposit nor a withdrawal` });
    }
    const cat = categorize(tx, coa);
    if (!cat) {
      anomalies.push({ type: "unmapped", line: tx.line, description: tx.description, direction: tx.direction,
        reason: "no chart_of_accounts account on the matching balance side has a keyword for this line; left unmatched rather than booked to an invented account" });
      continue;
    }
    if (cat.ambiguous) ambiguousCount += 1;
    categorized.push({
      line: tx.line, date: tx.date, description: tx.description, amount: tx.amount,
      direction: cat.direction, account: cat.account, confidence: cat.confidence, reason: cat.reason,
    });
  }

  // --- ambiguous batch -> needs_review -----------------------------------
  const ambiguousShare = transactions.length ? (ambiguousCount + anomalies.filter((a) => a.type === "unmapped").length) / transactions.length : 0;
  if (ambiguousShare > reviewThreshold) {
    const review_queue = anomalies.filter((a) => a.type === "unmapped")
      .concat(categorized.filter((c) => c.confidence <= 0.55).map((c) => ({ type: "low_confidence", line: c.line, description: c.description, reason: c.reason })));
    sealRefusal({ bookkeeping: {
      verdict: "needs_review",
      reason: `ambiguous share ${(ambiguousShare * 100).toFixed(0)}% exceeds review_threshold ${(reviewThreshold * 100).toFixed(0)}%; too many lines cannot be booked with confidence`,
      source_read,
      review_queue,
      transactions_read: transactions.length,
    } });
    return;
  }

  // --- reconciliation ----------------------------------------------------
  const sum = (arr) => arr.reduce((s, x) => s + (Number(x.amount) || 0), 0);
  const unmatchedLines = anomalies.filter((a) => a.type === "unmapped");
  const matchedAmount = sum(categorized);
  const totalCredits = sum(categorized.filter((c) => c.direction === "credit"));
  const totalDebits = sum(categorized.filter((c) => c.direction === "debit"));

  // Prior-period reconciliation: compare net cash movement to a supplied
  // baseline (opening_balance / prior_net) when present; read-only, no posting.
  const net = totalCredits - totalDebits;
  const priorNet = Number.isFinite(priorPeriod.prior_net) ? priorPeriod.prior_net : null;
  const opening = Number.isFinite(priorPeriod.opening_balance) ? priorPeriod.opening_balance : null;

  const reconciliation = {
    matched: { count: categorized.length, amount: Number(matchedAmount.toFixed(2)),
      credits: Number(totalCredits.toFixed(2)), debits: Number(totalDebits.toFixed(2)) },
    unmatched: { count: unmatchedLines.length,
      amount: Number(sum(transactions.filter((t) => unmatchedLines.some((u) => u.line === t.line))).toFixed(2)),
      lines: unmatchedLines.map((u) => u.line) },
    net_cash_movement: Number(net.toFixed(2)),
    prior_period: priorNet == null && opening == null ? null : {
      opening_balance: opening,
      prior_net: priorNet,
      variance_vs_prior_net: priorNet == null ? null : Number((net - priorNet).toFixed(2)),
      closing_balance: opening == null ? null : Number((opening + net).toFixed(2)),
    },
    balanced: unmatchedLines.length === 0,
  };

  const bkResult = {
    verdict: unmatchedLines.length === 0 ? "reconciled" : "reconciled_with_exceptions",
    statement_date: statementDate || (transactions[0] && transactions[0].date) || null,
    source_read,
    transactions_read: transactions.length,
    categorized,
    anomalies,
    reconciliation,
    ledger_mutation_performed: false,
  };

  // Hand the reconciliation off to the downstream transport step through the
  // shared run dir, so the append_event binds the real result even on runtimes
  // that do not resolve $context references.
  try {
    const handoff = (process.env.RUNX_CWD || ".") + "/.bk-handoff.json";
    writeFileSync(handoff, JSON.stringify(bkResult));
  } catch { /* transport falls back to context/input */ }

  seal({ bookkeeping: bkResult });
}

main().catch((e) => hardError("reconcile error: " + e.message));
