# Bookkeeper Skill - Delivery Report (bounty #89)

## Overview
`bookkeeper` turns a messy transaction statement into clean books **without
guessing**. It reads the statement from a **real source at runtime**, categorizes
each transaction to an existing GL account, flags anomalies, and emits a
read-only `reconciliation{matched, unmatched}` which it seals as an
`append_event` audit record. It books nothing to a live ledger.

- **Real source read**: a live HTTPS web-fetch of the U.S. Treasury Fiscal Data
  "Deposits & Withdrawals of Operating Cash" (DTS) daily statement (or an inline
  batch). The dogfood reads a live statement, not a hand-pasted fixture.
- **Categorize**: every line is bound only to an existing `chart_of_accounts`
  account whose normal balance side matches the transaction direction (Deposits
  credit, Withdrawals debit) and whose keywords hit the description; each line
  carries a confidence and a reason. A line that matches nothing is left
  unmatched (an anomaly) - the skill never invents a GL account.
- **Reconcile**: read-only `reconciliation{matched, unmatched}` with credit/debit
  totals, net cash movement, and a prior-period variance.
- **Seal**: the reconciliation is sealed as one `append_event` onto a governed
  `bookkeeping_reconciliation` audit aggregate (a consumed effect), never a
  general-ledger posting (`ledger_mutation_performed: false`).

## Package
- **Skill**: `bookkeeper` | **Owner**: `automerchlab` | **Version**: `1.0.2`
- **Registry ref**: `automerchlab/bookkeeper@1.0.2` (runx registry read automerchlab/bookkeeper@1.0.2 --json resolves metadata + digests)
- **public_url**: https://runx.ai/x/automerchlab/bookkeeper@1.0.2
- **pr_url**: https://github.com/runxhq/runx/pull/346
- **source_url**: https://github.com/automerchlab/runx/tree/a8b7a396d13251261589fb510f8e279ae29b3342
- **raw X.yaml**: https://raw.githubusercontent.com/automerchlab/runx/a8b7a396d13251261589fb510f8e279ae29b3342/skills/bookkeeper/X.yaml
- **raw SKILL.md**: https://raw.githubusercontent.com/automerchlab/runx/a8b7a396d13251261589fb510f8e279ae29b3342/skills/bookkeeper/SKILL.md
- **verification_json**: https://raw.githubusercontent.com/automerchlab/runx/a8b7a396d13251261589fb510f8e279ae29b3342/verification.json

## runx CLI
- `runx --version` -> **runx-cli 0.6.14** (>= 0.6.14 floor). Used for publish, install, dogfood, and verify.

## Publish & install
- Publish: `runx login --provider github --for publish`, then
  `runx registry publish ./skills/bookkeeper/SKILL.md --registry https://api.runx.ai --version 1.0.2`.
- Clean install: `runx add automerchlab/bookkeeper@1.0.2 --registry https://api.runx.ai` -> source=remote, status=installed
  (digest sha256:0be65d28c3fed29d4c18b71d799cb5b27bc5e12fcbd56cee2aa7875a676081b4).

## Harness
- Local harness: `runx harness ./skills/bookkeeper` -> **3/3 cases, 0 assertion errors** (WSL Linux local).
- Cases: bookkeeper-clean-batch-sealed (sealed), bookkeeper-ambiguous-needs-review (needs_review (sealed refusal)), bookkeeper-booking-refused (refused (sealed refusal)).
  - **bookkeeper-clean-batch-sealed** - a clean inline transactions[] batch is categorized and reconciled
    (one unmappable line flagged as an anomaly); the graph seals with an append_event audit record.
    Deterministic and egress-free, so it seals identically wherever the harness runs; the live Treasury
    web-fetch is exercised by the dogfood.
  - **bookkeeper-ambiguous-needs-review** - an unmappable batch exceeds the review threshold; the skill
    refuses with verdict needs_review and returns a review_queue (sealed refusal).
  - **bookkeeper-booking-refused** - book=true engages the read-only refusal.
- Harness evidence committed in the PR: `skills/bookkeeper/harness/harness_out.json` and the sealed
  harness receipts under `skills/bookkeeper/harness/receipts/`.

## Dogfood (post-publish, real, against the PUBLISHED package)
- Command: `runx skill automerchlab/bookkeeper@1.0.2 bookkeep --registry https://api.runx.ai -j -i "statement_url=https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v1/accounting/dts/deposits_withdrawals_operating_cash?fields=record_date,account_type,transaction_type,transaction_catg,transaction_today_amt&sort=transaction_catg&page[size]=500" -i statement_date=2026-07-16 --input-json prior_period='{"opening_balance": 800000, "prior_net": 12000}' -R ./receipts`
- The receipt's registry provenance proves the published package was run: registry_source=remote https://api.runx.ai, skill_id=automerchlab/bookkeeper, version=1.0.2, digest=sha256:0be65d28c3fed29d4c18b71d799cb5b27bc5e12fcbd56cee2aa7875a676081b4, trust_state=trusted, trust_tier=community.
- The run read **180 live statement lines** from the U.S. Treasury DTS operating-cash statement
  for 2026-07-16, categorized **173** to GL accounts (each with a confidence and a reason), flagged
  **7** anomalies (left unmatched, never booked to an invented account), reconciled
  **matched count=173 amount=569963** vs **unmatched count=7**, computed the prior-period
  variance, and sealed the reconciliation as append_event **8f9ccb1edfef121df79aa70ac9a3ed25a8d3729564f9e7bbca1b529c4238f464** on the
  `bookkeeping_reconciliation` aggregate (`ledger_mutation_performed: false`).
- Receipt: `runx:receipt:sha256:4f70d13a66e231ebcff413498d74d667de8728dda2fe69351123991c25e78ce7` (graph receipt; two child step receipts linked via lineage).
- `runx verify --receipt dogfood_receipt.json --json` -> **valid: true, signature_mode: production, signature: valid**.

## Provenance (single source revision)
- source_url, raw X.yaml, raw SKILL.md and verification.json all resolve at one source revision:
  commit `a8b7a396d13251261589fb510f8e279ae29b3342` on the `automerchlab/runx` `bookkeeper` branch (the PR head lineage).
- The committed skill files are the files published as `automerchlab/bookkeeper@1.0.2` and the dogfood ran that published
  package from the remote registry - not a local path (receipt registry_provenance above).
- This report and evidence.json are committed as the direct child of `a8b7a396d13251261589fb510f8e279ae29b3342` and describe that same
  revision; the recorded receipt_ref is the post-publish dogfood run, not a harness fixture seal.

## Read-only contract
The reconciliation is a read-only artifact and the append_event writes to an audit aggregate, not the
general ledger. `book: true` (or `mutate` / `post_to_ledger`) engages the read-only refusal, and an
ambiguous batch is refused with `needs_review` rather than being booked to invented accounts.

## How a new user installs, runs, verifies (no private context)
1. `runx add automerchlab/bookkeeper@1.0.2 --registry https://api.runx.ai`
2. `runx skill automerchlab/bookkeeper@1.0.2 bookkeep --registry https://api.runx.ai -j -i "statement_url=https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v1/accounting/dts/deposits_withdrawals_operating_cash?fields=record_date,account_type,transaction_type,transaction_catg,transaction_today_amt&sort=transaction_catg&page[size]=500" -i statement_date=2026-07-16 --input-json prior_period='{"opening_balance": 800000, "prior_net": 12000}' -R ./receipts`
3. `runx verify --receipt-dir ./receipts --json` -> valid=true, signature_mode=production.

## What to inspect first
1. `runx verify --receipt dogfood_receipt.json --json` (valid=true, production).
2. `evidence.json` dogfood.output (source_read via runtime web-fetch, categorized sample with
   confidence+reason, anomalies, reconciliation matched/unmatched, and the append_event audit record).
3. Raw X.yaml / SKILL.md / verification.json at source revision `a8b7a396d13251261589fb510f8e279ae29b3342`.
4. The bookkeeper-ambiguous-needs-review harness case: an unmappable batch refused with needs_review.
