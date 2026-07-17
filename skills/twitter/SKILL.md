---
name: twitter
description: >-
  Govern X (Twitter) account work through three lanes: evidence reads, typed
  act plans for posting and account hygiene, and gated execution with per-act
  provider evidence. Nothing reaches a live timeline or mutates an account
  without an explicit approval recorded in the receipt.
runx:
  category: growth
---

# Twitter

One account, three lanes: read evidence, plan typed acts, execute an approved
plan.

This is the public branded X (Twitter) catalog skill for the `send-as` action
family. Its core invariant: the agent may read, audit, draft, and plan freely,
but every act that publishes to a public timeline or mutates the account stops
at a human approval gate, and the sealed receipt proves which acts ran, against
which plan digest, with which provider evidence.

Write for the operator who owns the account, the reviewer who approves a live
act, and downstream skills that consume the resulting packets. Keep each plan
to the smallest evidence-backed act set that satisfies the objective. Existing
post and user ids must come from supplied evidence, never memory or guesswork;
name missing evidence as a blocker.

## What this skill does

Four runners:

- `read`: collect account evidence from the live API or, preferred for bulk
  history, an X archive export file (`tweets.js`, `following.js`). Queries:
  `snapshot`, `posts`, `mentions`, `search`, `following`, `followers`. Emits
  `twitter.evidence.v1`. Read-only; no gate.
- `plan`: turn one bounded objective plus evidence into `twitter.plan.v1`, an
  explicit list of typed acts with rationale, using agent judgment. This is the
  curated lane, for dozens of acts that each deserve a reason. A plan is a
  draft; it delivers nothing.
- `select`: the bulk lane. Apply a deterministic predicate to an archive export
  and emit a compact plan of delete acts, no agent judgment. Use it when the
  criterion is mechanical (an author, a date range, an engagement threshold)
  and the match set runs to thousands, where a per-item rationale would be
  wrong and would exceed the runtime output limit. Emits `twitter.selection.v1`
  carrying a digest-bound `twitter_plan`.
- `execute`: run an approved plan through the X API behind an approval gate,
  act by act, sealing per-act provider evidence into `twitter.execution.v1` and
  appending the batch outcome to a durable execution ledger. Rate-limit and
  batch stops are clean: the packet names the remaining act ids, and the ledger
  holds what completed.

### Resume and the execution ledger

`execute` is stateless per turn; the state lives in the ledger, not the
process. Each turn appends a `twitter_execution` event (its `executed_act_ids`)
to a `data-store` stream keyed by the plan digest, under optimistic-concurrency
version control. A bulk purge of thousands of posts is therefore many gated
batches sized to the provider rate window, advanced by an external driver: the
driver reads the ledger, folds the executed set and the stream version, calls
`execute` for one batch, and repeats until the plan is empty. Resume is
automatic, an interrupted purge continues from the ledger with no manual
bookkeeping, because rerunning the driver re-reads the same durable state. The
driver is a caller's concern, not part of this skill. The operator binds
`data_source_ref` to durable local SQLite (the default `data-store` adapter);
no state is invented in the skill.

Each runner emits exactly one packet for its lane: typed evidence with
provenance and a content digest, a curated or bulk act plan bound by digest, or
provider outcomes bound to the executed plan digest and appended to the ledger.

The act vocabulary, with its consequence class:

| kind | consequence | params |
| --- | --- | --- |
| `post` | public_send | `text` |
| `reply` | public_send | `text`, `in_reply_to` |
| `quote` | public_send | `text`, `quote_of` |
| `thread` | public_send | `texts` (ordered, max 25) |
| `repost` | public_send | `post_id` |
| `delete_post` | live_mutation | `post_id` |
| `unfollow` | live_mutation | `target_user_id` |
| `mute` | live_mutation | `target_user_id` |
| `block` | live_mutation | `target_user_id` |
| `follow` | live_mutation | `target_user_id` |
| `like` | live_mutation | `post_id` |

Every kind is consequential, so every plan sets
`gates.human_approval_required: true`. `follow`, `like`, and `repost` are
engagement acts and share a hard cap of 10 per execution. Direct messages are
outside this skill.

## When to use this skill

- Promote a release, publish a thread, or reply to a mention on behalf of a
  named principal, with content bound verbatim in the plan.
- Audit an account's own post history or following list and prune it: bulk
  delete old posts, unfollow low-value accounts, against operator-stated
  criteria.
- Gather timeline, mention, or search evidence for downstream research,
  triage, or lead skills that consume `twitter.evidence.v1`.
- Route a `send-as` plan onto X as its provider delivery lane.

## When not to use this skill

- To send direct messages or manage DM conversations.
- To farm engagement: follow-churn, mass liking, coordinated reposting, or any
  volume pattern designed to game visibility. The engagement cap is not a
  budget to fill.
- To mention, reply to, or dogpile users who have not engaged with the
  principal, at volume.
- To operate an account the operator does not control, or to automate consumer
  account creation.
- To bypass the approval gate, the act caps, or the operator's spending limit
  on the pay-per-use API.

## Procedure

For the `plan` runner, build `twitter_plan` this way:

1. Hold one bounded objective. If the ask bundles unrelated jobs (a promo
   thread plus a follow purge), return `needs_input` with the split.
2. Ground every reference. Ids for `delete_post`, `unfollow`, `reply`,
   `quote`, `like`, `mute`, and `block` must come from `evidence_json` or an
   explicit operator-supplied id. If the evidence is missing or stale, name it
   as a blocker; never invent or approximate an id.
3. Write public content into the act params verbatim: the exact `text` or
   `texts` to publish, shaped by `brand_context` when supplied. Do not plan a
   summary of what will be written; approval binds the exact words.
4. Give every act a stable `act_id` (`act-001` onward), its `kind`, `params`,
   its `consequence` from the table, and a one-line `rationale` tied to the
   objective or the operator's stated criteria.
5. Apply `operator_policy` narrowly. A prune criterion like "no engagement and
   older than 2023" selects only posts the evidence shows meet it; borderline
   items go to `open_questions`, not into the act list.
6. Keep the plan small: at most 50 acts, at most 10 engagement acts, threads
   at most 25 segments. A larger job becomes staged plans.
7. Set `decision`: `ready` when acts are grounded and complete; `needs_input`
   for missing objective, principal, evidence, or content; `reject` for asks
   outside the vocabulary or inside the abuse boundaries.
8. Never place credentials, bearer tokens, raw API dumps, or third-party
   personal data beyond public ids and handles in the plan.

## Edge cases and stop conditions

- **Mixed objective:** return `needs_input` with the runner or plan split.
- **Id not in evidence:** blocker; the plan stays `needs_input`.
- **Prune criteria ambiguous:** select the unambiguous items, list the rest in
  `open_questions`.
- **Rate limit during read:** the evidence packet returns what it collected
  with `stop_conditions: ["rate_limited"]` and the reset time.
- **Rate limit during execute:** the execution packet lists
  `remaining_act_ids`; re-run with `already_executed_act_ids` after the reset.
- **Plan digest mismatch on execute:** the apply step refuses and executes
  nothing; the ledger records a no-op refusal event, and the driver, which
  re-reads the stream version each turn, folds no completed acts and advances.
- **Approval missing or denied:** the execute graph stops at the gate.
- **Credentials missing:** clean `needs_input` stop naming the environment
  variables; never a half-configured call.
- **Fully autonomous live posting requested:** `reject`; the gate is the
  contract.

## Output schema

- `twitter.evidence.v1`: `decision`, `source` (`live` or `archive`), `query`,
  `account`, `items[]` (typed post or user records with metrics), `item_count`,
  `truncated`, `provenance` (`retrieved_via`, `request_count`,
  `content_digest`), `rate`, `blockers[]`, `stop_conditions[]`.
- `twitter.plan.v1`: `decision` (`ready`, `needs_input`, `reject`),
  `objective`, `principal`, `acts[]` (`act_id`, `kind`, `params`,
  `consequence`, `rationale`), `gates`
  (`human_approval_required`, `approval_ref`), `evidence_refs[]`,
  `open_questions[]`, `blockers[]`, `success_checkpoint`.
- `twitter.selection.v1`: `decision`, `objective`, `principal`, `predicate`,
  `matched`, `scanned`, `truncated`, `twitter_plan` (a `twitter.plan.v1`),
  `plan_digest`, `blockers[]`. The `twitter_plan` is what a driver hands to
  `execute`, and `plan_digest` is bound so the approved and executed bytes are
  provably identical.
- `twitter.execution.v1`: `decision` (`executed`, `partial`, `stopped`,
  `refused`), `plan_digest`, `principal`, `results[]` (`act_id`, `kind`,
  `consequence`, `status`, `provider_ref`, `detail`), `executed_act_ids[]`
  (folded by the ledger to compute what remains), `remaining_act_ids[]`,
  `rate`, `blockers[]`, `success_checkpoint`.

## Worked example

Input: objective "delete my zero-engagement posts from before 2024",
principal `account:@example`, evidence from
`read(query: posts, archive_file: data/tweets.js)` showing three matching
posts, operator_policy "keep anything with replies".

Output: `decision: ready`; three `delete_post` acts, each carrying the post id
from the evidence and a rationale quoting its age and zero metrics;
`gates.human_approval_required: true`; one borderline post with two replies
listed in `open_questions`. Execution then stops at the approval gate, and the
sealed receipt binds the approved digest to the three deletions the provider
confirmed.

## Inputs

- `read`: `query` (required), `params`, `archive_file`, `max_items`, `auth`.
- `plan`: `objective` (required), `principal` (required), `evidence_json`,
  `operator_policy`, `brand_context`, `operator_context`.
- `select`: `objective` (required), `principal` (required), `archive_file`
  (required), `predicate` (required: `rt_of`, `is_retweet`, `text_prefix`,
  `text_contains`, `max_likes`, `max_reposts`, `before_year`, `after_year`),
  `max_acts`.
- `execute`: `plan_json` (required), `plan_digest`, `data_source_ref`
  (required), `expected_version` (required), `idempotency_key` (required),
  `already_executed_act_ids`, `max_acts`. The driver supplies the ledger inputs
  per turn.

## Credentials and cost

Credentials are delivered per run through the runx credential envelope, never
as inputs and never as receipt material. Two materials exist:

- `TWITTER_USER_AUTH`: one JSON object holding `consumer_key`,
  `consumer_secret`, `access_token`, `access_secret` (OAuth 1.0a user
  context). Required for every mutation and for own-account reads. Store it
  with `runx credential set twitter --profile twitter-user --auth-mode
  oauth1_user --from-stdin`.
- `TWITTER_BEARER_TOKEN`: the app-context bearer token, enough for `search`
  and public reads. Store it with `runx credential set twitter --profile
  twitter-app --auth-mode bearer --from-stdin`. Read-only app runs should use
  only this profile.

Use `--profile twitter-user` with `read --auth user` and every `execute` run;
use `--profile twitter-app` with `read --auth app`. The runner contract maps the
selected profile's auth mode to exactly one delivery variable. Tool sandbox
allowlists do not carry credentials. For local development, the same declared
variable can come from the process or workspace `.env`; if both Twitter
variables are set, Runx refuses the ambiguous selection.

The X API bills per request on current plans and a post containing a link
costs a large multiple of a plain one, so prefer archive exports for bulk
history, set a spending cap in the developer portal, and let `max_items` and
the act caps bound each run.
