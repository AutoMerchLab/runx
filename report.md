# postmortem-maker 2.0.1 — Frantic #83 delivery report

**Package**: `automerchlab/postmortem-maker@2.0.1` · **Owner**: `automerchlab` · **Registry digest**: `2c38cb8403134a4c71e4f647e411c2146dfc97232b517a35d66cdd217083392f`
**Public URL**: https://runx.ai/x/automerchlab/postmortem-maker@2.0.1
**PR**: https://github.com/runxhq/runx/pull/320
**Source revision (X)**: https://github.com/automerchlab/runx/tree/a12043512818cb4d6e27636f964cbaab92eecab9
**Receipt**: `runx:receipt:sha256:5429edb88481e299e8a75e94316eae3e84f7e48f0fac95bb67a2189bd4a6e89c` · **runx verify**: valid=True, signature_mode=production, kid=key1
**runx CLI**: runx-cli 0.8.2

## What changed against the previous delivery

The previous revision constructed send-as shaped objects locally and reported the
result as sent. This revision does not report a send — it performs one, and then
proves it from the other side:

1. `read_incident` fetches the incident thread over HTTPS at run time.
2. `read_outbox` reads the publication stream and its version.
3. `compose` reconstructs the timeline (every entry citing the event id, author,
   URL and quoted line), confirms a root cause only when exactly one candidate is
   named in an unhedged statement, and authorizes or withholds publication.
4. `deliver` runs **only** when publication is authorized: it appends the
   postmortem to the outbox under compare-and-set, assigns a provider message id
   and makes it durable.
5. `verify` re-opens the outbox itself, re-digests the **stored** message and
   matches it against the digest the send plan authorized — or, on the withheld
   path, asserts that no send plan, no provider act and no delivery exist and the
   outbox version has not moved.

## What the transport is

An append-only outbox log **bundled with the package** — not a hosted provider
and not the runx data-store. Canonical `runx/send-as` describes itself as a
planning and authority layer that "never delivers" and refers delivery to a
provider adapter; and the runtime's native `data.*` tools are not in the
execution closure of a registry-installed package. So the package ships its own
adapter, with compare-and-set, idempotency and independent readback. Nothing in
this packet claims a hosted provider delivered anything.

## Dogfood — the published package against a live incident thread

| run | command | result |
|-----|---------|--------|
| 1 (publish) | `runx skill automerchlab/postmortem-maker@2.0.1 --registry https://api.runx.ai --json --input-json incident_source='{"kind":"github_issue","ref":"https://api.github.com/repos/nltk/nltk/issues/3733"}' --input-json publish_target='{"data_source_ref":"local://runx-postmortems/dogfood-2026-08-04","channel":"incident-review","aggregate_id":"nltk-3733","principal":"incident-review-bot","audience":"incident-review"}' -R ./receipts2` | root cause **confirmed**, 1 timeline entry cited, delivery **delivered** `ef49e3e7306b44dd7f8ac8f3`, outbox 0 → 1, readback digest_match **True** |
| 2 (replay) | same incident, `-R ./receipts2_replay` | **replayed=True**, same message id `ef49e3e7306b44dd7f8ac8f3`, outbox stays at version 1 |
| 3 (withheld) | `https://api.github.com/repos/Goz3rr/vscode-glualint/issues/24` | refused, nothing delivered; outbox still holds exactly 1 message |

Source read: `https://api.github.com/repos/nltk/nltk/issues/3733` — runtime-web-fetch, 2 events, source_digest `sha256:d0ed54d0a816c02d42a4f23670f670971eaf93a97dfed0a9bb61148051994f1e`.
Root cause: #3722 is named as the cause in an unhedged statement in the incident thread
Citations: https://github.com/nltk/nltk/issues/3733#issuecomment-5175474175

## Harness (WSL local, before publish)

consistent-incident-published (sealed), conflicting-evidence-withheld (sealed), empty-thread-refused (refused) — passed 3/3 cases with 0 assertion errors. Receipts are
in the PR under `skills/postmortem-maker/harness/receipts/`. The
`conflicting-evidence-withheld` case is what proves the refusal path: it asserts
`send_plan_created=false`, `provider_act_performed=false`, `delivery_exists=false`
and `outbox_unchanged=true`.

## Provenance

`automerchlab/postmortem-maker@2.0.1`, the PR head commit, `source_url`, `x_yaml`, `skill_md`,
`verification_json`, `receipt_ref` and this report all describe the same package
version and the same source revision `a12043512818cb4d6e27636f964cbaab92eecab9`; `evidence.json` and this report sit
in that commit's child so the evidence can name the source revision it came from.

## Install, run, verify

```bash
runx add automerchlab/postmortem-maker@2.0.1 --registry https://api.runx.ai
runx skill automerchlab/postmortem-maker@2.0.1 --registry https://api.runx.ai --json --input-json incident_source='{"kind":"github_issue","ref":"https://api.github.com/repos/nltk/nltk/issues/3733"}' --input-json publish_target='{"data_source_ref":"local://runx-postmortems/dogfood-2026-08-04","channel":"incident-review","aggregate_id":"nltk-3733","principal":"incident-review-bot","audience":"incident-review"}' -R ./receipts2
runx verify --receipt "$(ls -t ./receipts2/sha256*.json | head -1)" --json
```
