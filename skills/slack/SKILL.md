---
name: slack
description: Read bounded Slack search and thread evidence, plan an exact reply, and deliver an approved reply through Runx Connect with stable-message readback.
runx:
  category: ops
---

# Slack

Use Slack as a governed provider boundary without turning Slack—or Runx
Cloud—into the owner of the operator's workflow. This skill owns the reusable
Slack mechanics that an agent should not have to reconstruct: bounded message
search, bounded thread hydration, digest-bound reply planning, explicit
approval, idempotent delivery, and exact-message readback.

Cloud has one narrow role in this flow. It retains the OAuth credential,
resolves the operator's grant, and executes a fixed Slack driver operation.
The skill, its procedure, approval point, retry identity, and completion rule
remain in Runx OSS. Any queue, team-specific routing rule, or durable action
state belongs in a higher-level operator skill and normally composes
`operator-inbox`.

## Runners

`search` reads one bounded Slack search page. Supply a `query` object with at
least one of `author_external_id`, `mentions_connected_subject`, or `keywords`.
It may also contain `after`, `before`, `channel_types`, `limit`, and a provider
cursor. The provider enforces exact search syntax, a maximum page of 20, and
returns normalized locators and bounded previews rather than a raw Slack
response. Continue only from `next_cursor`; one page is never proof that a
workspace scan is complete.

`read_thread` reads one bounded thread page from an exact
`slack://workspace/channel/timestamp` locator. The limit is capped at 15 because
Slack applies a low limit and, for some non-Marketplace installations, a very
low request cadence to `conversations.replies`. Hydrate only threads that are
actually needed, preserve `next_cursor`, and do not fan out speculative reads.

`plan_reply` is safe and does not call Slack. It computes the text digest with
Runx's native `data.digest` tool and passes the principal, exact thread
audience, and digest-bound content to canonical `send-as`. The emitted
`send_plan` is an authorization plan, not evidence of delivery.

`deliver_reply` accepts that exact `send_plan`, thread locator, text, and a
stable UUID idempotency key. It recomputes the digest and rejects any change to
the plan decision, Slack provider, chat channel, thread audience, content
digest, or human-approval requirement. After approval it calls the native
`provider.mutate` tool for `thread.reply`, then independently calls
`provider.read` for `thread.reply.read`. Completion requires the same workspace,
channel, thread locator, message locator, content digest, and occurrence time
from Slack. Provider acceptance alone is not completion.

Use `slack-notify` for a proactive top-level channel post. Use this skill for
search, thread context, and replies. Use `operator-inbox` when observations must
become durable local work items. A product-owned skill may add team-specific
triage and compose these skills, but must not copy their provider transport,
approval, idempotency, readback, or queue logic.

## Authority and privacy

Reads resolve a Slack Connect grant for `messages.search` or `thread.read` and
do not ask for human approval. Reply delivery requires both `thread.reply` and
`thread.reply.read`; the mutation stops at an explicit approval bound to the
unchanged plan and content digest. Runx supplies the idempotency key once, and
Cloud verifies the registered operation and its read/mutate class against the
server-side grant and OAuth binding.

The skill never receives a Slack token, constructs HTTP, or stores a raw
provider envelope. Search and thread outputs contain bounded previews because
the operator needs context. Reply receipts contain only stable locators,
timestamps, and a content digest; they do not echo the delivered message.

## Stop conditions

- Refuse a missing, ambiguous, revoked, wrong-provider, or insufficient-scope
  Connect grant. Never fall back to a token, webhook, browser, or Cloud script.
- Stop when a search has no structural selector, uses an unsupported modifier,
  or asks for more than one bounded page per turn.
- Stop on malformed or cross-workspace locators and on thread/message channel
  mismatch.
- Stop on reply-plan drift, absent or denied approval, invalid idempotency, or
  provider readback that does not match the exact delivered message.
- Do not infer that a request is resolved from Slack prose. Record resolution,
  waiting, follow-up, or dismissal explicitly in the owning operator workflow.
- Do not claim complete scan coverage while a cursor remains, or successful
  delivery from a local plan, a provider acknowledgement, or a fixture.

## Worked flow

An operator searches for direct mentions and receives one page with a next
cursor. They hydrate only the actionable thread, then pass that normalized
observation to a team-specific triage skill and `operator-inbox`. If a reply is
needed, `plan_reply` binds the proposed text to the exact thread. A changed
draft fails before approval. An approved unchanged reply is posted once, then
read back from the exact returned message locator before Runx seals the effect.
