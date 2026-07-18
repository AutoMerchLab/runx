#!/usr/bin/env python3
"""Assemble skills/bookkeeper/X.yaml from the readable step sources.

The two graph steps (src/reconcile.cjs, src/transport.cjs) are base64-inlined
into a `node -e "eval(Buffer.from('<b64>'))"` command so the published registry
package is a single self-contained X.yaml (the same shape the runx registry
accepts for a remote graph). The readable sources stay in src/ for review and
are the single source of truth; run this script after editing them.
"""
import base64
import pathlib

HERE = pathlib.Path(__file__).parent
VERSION = "1.0.2"


def inline(path):
    src = (HERE / "src" / path).read_bytes()
    b64 = base64.b64encode(src).decode("ascii")
    return "eval(Buffer.from('%s','base64').toString('utf8'))" % b64


RECONCILE = inline("reconcile.cjs")
TRANSPORT = inline("transport.cjs")

# Ambiguous / unmappable batch for the needs_review harness case: none of these
# descriptions hit any default chart_of_accounts keyword, so the batch cannot be
# booked with confidence and the skill refuses with verdict needs_review.
AMBIGUOUS_TX = (
    '[{"description":"Miscellaneous vendor settlement 4471","type":"withdrawal","amount":120},'
    '{"description":"Unknown counterparty adjustment","type":"deposit","amount":90},'
    '{"description":"Sundry clearing entry","type":"withdrawal","amount":55},'
    '{"description":"Unreconciled suspense item","type":"deposit","amount":210}]'
)

STATEMENT_URL = (
    "https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v1/accounting/"
    "dts/deposits_withdrawals_operating_cash?fields=record_date,account_type,"
    "transaction_type,transaction_catg,transaction_today_amt&sort=transaction_catg&page[size]=500"
)

# Clean transactions[] batch for the sealed happy case. Deterministic and
# egress-free (so the case seals identically wherever the harness runs, local or
# hosted): a representative statement batch that categorizes to several GL
# accounts, leaves one unmappable line (Unclassified) as a flagged anomaly, and
# reconciles matched vs unmatched against a prior_period baseline. The live
# runtime web-fetch of the real Treasury statement is exercised by the dogfood.
CLEAN_BATCH = (
    '['
    '{"description":"Taxes - Withheld Individual/FICA","type":"deposit","amount":18240},'
    '{"description":"Taxes - Corporate Income","type":"deposit","amount":2600},'
    '{"description":"DHS - Customs Duties, Taxes, and Fees","type":"deposit","amount":310},'
    '{"description":"Dept of Agriculture (USDA) - misc","type":"deposit","amount":140},'
    '{"description":"Public Debt Cash Issues (Table IIIB)","type":"deposit","amount":85000},'
    '{"description":"SSA - Benefits Payments","type":"withdrawal","amount":1650},'
    '{"description":"HHS - Grants to States for Medicaid","type":"withdrawal","amount":2200},'
    '{"description":"Federal Salaries (EFT)","type":"withdrawal","amount":410},'
    '{"description":"Interest on Treasury Securities","type":"withdrawal","amount":900},'
    '{"description":"Unclassified","type":"deposit","amount":75}'
    ']'
)

X_YAML = f"""skill: bookkeeper
version: "{VERSION}"

catalog:
  kind: graph
  audience: operator
  visibility: public
  role: context

emits:
  - name: bookkeeping
    packet: runx.bookkeeping.v1
  - name: bookkeeping_applied
    packet: runx.bookkeeping.applied.v1

runners:
  bookkeep:
    default: true
    type: graph
    inputs:
      statement_url:
        type: string
        required: false
        description: >-
          Real transaction statement source, fetched over HTTPS at runtime (never a
          bundled fixture). Defaults to the U.S. Treasury Fiscal Data DTS deposits &
          withdrawals of operating cash statement when omitted.
      statement_date:
        type: string
        required: false
        description: Immutable statement date (YYYY-MM-DD) to pin the DTS fetch, so a historical run reconciles reproducibly.
      transactions:
        type: json
        required: false
        description: >-
          Optional inline transactions[] batch used when no statement_url is given.
          Each item is {{description, type|direction (deposit/credit or withdrawal/debit), amount}}.
        default: []
      chart_of_accounts:
        type: json
        required: false
        description: GL account allowlist; each account is {{code, name, type, side (debit/credit), keywords[]}}. A built-in Treasury chart is used when omitted. A line is bound only to an existing account, never an invented one.
        default: []
      prior_period:
        type: json
        required: false
        description: Read-only prior-period baseline {{opening_balance, prior_net}} for the reconciliation; no balance is posted.
        default: {{}}
      review_threshold:
        type: number
        required: false
        description: Ambiguous/unmappable share above which the batch is refused with verdict needs_review (default 0.5).
      book:
        type: boolean
        required: false
        description: Live-ledger booking framing. When true the skill refuses (read-only contract).
    graph:
      name: bookkeeper
      steps:
        - id: reconcile
          run:
            type: cli-tool
            command: node
            args:
              - -e
              - "{RECONCILE}"
          inputs:
            statement_url: "$input.statement_url"
            statement_date: "$input.statement_date"
            transactions: "$input.transactions"
            chart_of_accounts: "$input.chart_of_accounts"
            prior_period: "$input.prior_period"
            review_threshold: "$input.review_threshold"
            book: "$input.book"
          artifacts:
            named_emits:
              bookkeeping: reconcile.output.bookkeeping
            packets:
              bookkeeping: runx.bookkeeping.v1
        - id: transport
          run:
            type: cli-tool
            command: node
            args:
              - -e
              - "{TRANSPORT}"
          inputs:
            bookkeeping: "$context.bookkeeping"
          artifacts:
            named_emits:
              bookkeeping_applied: transport.output.bookkeeping_applied
            packets:
              bookkeeping_applied: runx.bookkeeping.applied.v1
  review:
    default: false
    type: cli-tool
    command: node
    args:
      - -e
      - "{RECONCILE}"
    inputs:
      statement_url:
        type: string
        required: false
      statement_date:
        type: string
        required: false
      transactions:
        type: json
        required: false
        default: []
      chart_of_accounts:
        type: json
        required: false
        default: []
      prior_period:
        type: json
        required: false
        default: {{}}
      review_threshold:
        type: number
        required: false
      book:
        type: boolean
        required: false
    outputs:
      bookkeeping: object

harness:
  cases:
    - name: bookkeeper-clean-batch-sealed
      runner: bookkeep
      inputs:
        transactions: '{CLEAN_BATCH}'
        prior_period: {{ opening_balance: 800000, prior_net: 12000 }}
      expect:
        status: sealed
        receipt:
          schema: runx.receipt.v1
          state: sealed
          disposition: closed
    - name: bookkeeper-ambiguous-needs-review
      runner: review
      inputs:
        transactions: '{AMBIGUOUS_TX}'
        review_threshold: 0.4
      expect:
        status: failure
        receipt:
          schema: runx.receipt.v1
          state: sealed
          disposition: failed
    - name: bookkeeper-booking-refused
      runner: review
      inputs:
        transactions: '[{{"description":"Dept of Energy (DOE)","type":"withdrawal","amount":11}}]'
        book: true
      expect:
        status: failure
        receipt:
          schema: runx.receipt.v1
          state: sealed
          disposition: failed
"""

(HERE / "X.yaml").write_text(X_YAML, encoding="utf-8", newline="\n")
print("wrote", (HERE / "X.yaml"), "version", VERSION)
print("reconcile b64 bytes:", len(RECONCILE), "transport b64 bytes:", len(TRANSPORT))
