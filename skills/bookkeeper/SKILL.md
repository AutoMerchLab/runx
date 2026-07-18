---
name: bookkeeper
description: >-
  Reads a real transaction statement from an external source at runtime (a live
  web-fetch of the U.S. Treasury Fiscal Data operating-cash statement, or an
  inline batch), categorizes every transaction to an existing account in
  chart_of_accounts with a confidence and a reason, flags anomalies, and emits a
  read-only reconciliation{matched, unmatched} sealed as an append_event audit
  record. Books nothing to a live ledger; refuses ambiguous batches with
  needs_review and any live-booking framing.
runx.category: data
---

# bookkeeper

Bookkeeper turns a messy transaction statement into clean books without
guessing. It reads the statement from a **real source at runtime**, categorizes
each transaction to an existing GL account, flags anomalies, and emits a
read-only reconciliation artifact — the whole read → categorize → reconcile →
seal loop is proven in one sealed dogfood run. It books nothing to a live
ledger.

## What it does

1. **Read** the statement from a real source at runtime — a live HTTPS
   `web-fetch` of the U.S. Treasury Fiscal Data "Deposits & Withdrawals of
   Operating Cash" (DTS) daily statement (`statement_url` + `statement_date`),
   or an inline `transactions[]` batch. The dogfood run reads a live statement,
   not a hand-pasted fixture. Deposits are receipts (credit); Withdrawals are
   payments (debit).
2. **Categorize** every transaction to an existing account in
   `chart_of_accounts`. A line is bound only to an account whose normal balance
   side matches the transaction direction AND whose keyword set hits the line
   description; the best match wins on hit count then keyword specificity. Each
   categorized line carries a `confidence` and a `reason` that names the matched
   keyword and account. The skill **never invents a GL account** — a line that
   matches nothing is left unmatched (an anomaly).
3. **Flag anomalies** — unmapped lines, unknown direction, and unparseable
   amounts, each with the reason it was flagged.
4. **Reconcile** into a read-only `reconciliation{matched, unmatched}` with
   credit/debit totals, net cash movement, and a prior-period variance against
   the supplied `prior_period` baseline. No balance is posted.
5. **Seal** the reconciliation as one `append_event` onto a governed
   `bookkeeping_reconciliation` audit aggregate, returning a sealed event id
   bound to the reconciliation content — a real, replayable side-effect that is
   an audit record, not a general-ledger posting.

## Read-only contract

Bookkeeper is a **reconciliation/preview** skill. It emits and seals a
read-only reconciliation artifact but does **not** post to or mutate a live
general ledger (`ledger_mutation_performed: false`). If `book: true` (or
`mutate` / `post_to_ledger`) is passed, the `review` runner refuses with verdict
`refused`. When too large a share of a batch cannot be booked with confidence,
the skill refuses with verdict `needs_review` and returns the `review_queue` of
lines a human must resolve, rather than forcing them onto invented accounts.

## Inputs

| input | type | required | notes |
|-------|------|----------|-------|
| `statement_url` | string | no | Real statement source fetched over HTTPS at runtime. Defaults to the Treasury DTS statement. |
| `statement_date` | string | no | Immutable statement date (YYYY-MM-DD) pinning the DTS fetch for reproducibility. |
| `transactions` | json | no | Inline `transactions[]` batch used when no `statement_url` is given. Each item `{description, type\|direction, amount}`. |
| `chart_of_accounts` | json | no | GL account allowlist `{code, name, type, side, keywords[]}`. A built-in Treasury chart is used when omitted. |
| `prior_period` | json | no | Read-only baseline `{opening_balance, prior_net}` for the reconciliation. |
| `review_threshold` | number | no | Ambiguous/unmappable share above which the batch is refused with `needs_review` (default 0.5). |
| `book` | boolean | no | Live-booking framing; when true the skill refuses (read-only). |

## Outputs

- `bookkeeping` — `verdict` (`reconciled` / `reconciled_with_exceptions` /
  `needs_review` / `refused`), `source_read`, `categorized[]` (each
  `{line, date, description, amount, direction, account{code,name,type},
  confidence, reason}`), `anomalies[]`, and `reconciliation{matched, unmatched,
  net_cash_movement, prior_period, balanced}`. `ledger_mutation_performed:
  false`.
- `bookkeeping_applied` — the sealed audit record: `verdict`, `sealed_event_id`,
  `aggregate`, the `reconciliation`, and `ledger_mutation_performed: false`.

## Harness

- `bookkeeper-clean-batch-sealed` — a clean `transactions[]` batch is
  categorized and reconciled (one unmappable line flagged as an anomaly); the
  graph seals with an `append_event` audit record. Deterministic and egress-free
  so it seals identically wherever the harness runs; the live Treasury web-fetch
  is exercised by the dogfood.
- `bookkeeper-ambiguous-needs-review` — an unmappable batch exceeds the review
  threshold; the skill refuses with verdict `needs_review` (sealed refusal).
- `bookkeeper-booking-refused` — `book: true` engages the read-only refusal.

## Install, run, verify

```bash
# Install from the registry
runx add automerchlab/bookkeeper@1.0.2 --registry https://api.runx.ai

# Run over a real, live statement (Treasury DTS for a pinned date)
runx skill automerchlab/bookkeeper@1.0.2 --registry https://api.runx.ai --json \
  -i statement_url='https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v1/accounting/dts/deposits_withdrawals_operating_cash?fields=record_date,account_type,transaction_type,transaction_catg,transaction_today_amt&sort=transaction_catg&page[size]=500' \
  -i statement_date=2026-07-16 \
  --input-json prior_period='{"opening_balance":800000,"prior_net":12000}' \
  -R ./receipts

# Verify the sealed receipt (production signature)
runx verify --receipt ./receipts/<receipt-file>.json --json
# expects valid=true, signature_mode=production
```

## Limitations

- Categorization is deterministic keyword matching over the line description and
  the transaction direction; a line whose description is outside the chart's
  keyword vocabulary is left unmatched (an anomaly), never guessed onto an
  account.
- The default chart is scoped to the U.S. Treasury operating-cash statement;
  supply your own `chart_of_accounts` for another statement shape.
- The skill posts nothing to a live ledger. `bookkeeping_applied` is an audit
  event on the `bookkeeping_reconciliation` aggregate, and the reconciliation is
  a read-only artifact.
