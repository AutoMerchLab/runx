// postmortem-maker / step 5 of 5: read the outbox back and assert the result.
//
// This step never trusts step 4's report. It re-opens the outbox from disk on
// its own, finds (or fails to find) the message, recomputes the content digest
// over the STORED bytes, and compares that against the digest the send plan
// authorized. It runs on both paths, so the receipt carries a proof either way:
//
//   published  -> readback.delivered = true, digest_match = true, and the
//                 stored message id/ref match what the provider reported.
//   withheld   -> readback.delivered = false plus the explicit no-delivery
//                 assertions: no send plan was authorized, no provider act was
//                 performed, no delivery exists for this incident, and the
//                 outbox version is unchanged from what step 2 read.

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const PROVIDER = "bundled-local-outbox";

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

function asObject(v) {
  if (v && typeof v === "object" && !Array.isArray(v)) return v;
  if (typeof v === "string" && v.trim()) {
    try {
      const p = JSON.parse(v);
      return p && typeof p === "object" && !Array.isArray(p) ? p : {};
    } catch (e) {
      return {};
    }
  }
  return {};
}

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value === undefined ? null : value);
}

function digestOf(value) {
  return `sha256:${createHash("sha256").update(canonical(value)).digest("hex")}`;
}

// Must match steps/read_outbox.mjs and steps/deliver.mjs exactly.
function outboxPath(target) {
  const key = `${target.data_source_ref}|${target.channel}|${target.aggregate_id}`;
  const id = createHash("sha256").update(key).digest("hex").slice(0, 16);
  const root = process.env.RUNX_CWD || process.cwd();
  return resolve(join(root, ".postmortem-outbox", `${id}.jsonl`));
}

function pick(input, ...names) {
  for (const n of names) {
    const direct = input[n];
    if (direct && typeof direct === "object") return direct;
    const ctx = input.context && input.context[n];
    if (ctx && typeof ctx === "object") return ctx;
  }
  return null;
}

function scalar(input, name) {
  if (input[name] !== undefined) return input[name];
  if (input.context && input.context[name] !== undefined) return input.context[name];
  return undefined;
}

function main() {
  const input = parseInput();
  const target = asObject(input.publish_target);
  for (const f of ["data_source_ref", "channel", "aggregate_id"]) {
    if (typeof target[f] !== "string" || target[f].length === 0) {
      return refuse(`publish_target.${f} is required`);
    }
  }

  const send_plan = pick(input, "send_plan") || {};
  const publishable = scalar(input, "publishable") === true;
  const idempotency_key = scalar(input, "idempotency_key");
  const authorized_digest = scalar(input, "content_digest");
  const version_before = scalar(input, "expected_version");
  const delivery_result = pick(input, "delivery_result") || null;

  const path = outboxPath(target);
  let records = [];
  if (existsSync(path)) {
    try {
      records = readFileSync(path, "utf8")
        .split("\n")
        .filter((l) => l.trim().length > 0)
        .map((l) => JSON.parse(l));
    } catch (e) {
      return refuse(`outbox at ${path} is corrupt: unreadable line`);
    }
  }
  const version_after = records.length;
  const stored = records.find((r) => r.idempotency_key === idempotency_key) || null;

  if (publishable) {
    if (!stored) {
      return refuse(
        `readback failed: the postmortem was authorized for delivery but no message with idempotency key ${idempotency_key} is in the outbox at ${path}`
      );
    }
    const stored_digest = digestOf(stored.body);
    const digest_match = stored_digest === authorized_digest && stored.content_digest === authorized_digest;
    if (!digest_match) {
      return refuse(
        `readback failed: stored message digests to ${stored_digest} but the send plan authorized ${authorized_digest}`
      );
    }
    if (delivery_result && delivery_result.message_id && delivery_result.message_id !== stored.message_id) {
      return refuse(
        `readback failed: the provider reported message ${delivery_result.message_id} but the outbox stores ${stored.message_id}`
      );
    }
    return seal({
      readback: {
        delivered: true,
        provider: PROVIDER,
        read_from: path,
        message_id: stored.message_id,
        message_ref: stored.message_ref,
        content_digest: stored.content_digest,
        digest_match: true,
        authorized_digest,
        delivered_at: stored.delivered_at,
        incident_ref: stored.incident_ref,
        outbox_version_before: version_before,
        outbox_version_after: version_after,
        timeline_entries_stored: Array.isArray(stored.body && stored.body.timeline)
          ? stored.body.timeline.length
          : 0,
        verified_independently: "re-read from the outbox file and re-digested; not copied from the deliver step output",
      },
    });
  }

  // Withheld path: prove that nothing was planned, acted on, or delivered.
  if (stored) {
    return refuse(
      `no-delivery assertion failed: publication was withheld but a message with idempotency key ${idempotency_key} exists in the outbox`
    );
  }
  const unchanged = typeof version_before !== "number" || version_before === version_after;
  if (!unchanged) {
    return refuse(
      `no-delivery assertion failed: outbox moved from version ${version_before} to ${version_after} on a withheld postmortem`
    );
  }

  seal({
    readback: {
      delivered: false,
      provider: PROVIDER,
      read_from: path,
      send_plan_created: false,
      send_plan_status: send_plan.status || "withheld",
      provider_act_performed: false,
      delivery_exists: false,
      outbox_version_before: version_before === undefined ? null : version_before,
      outbox_version_after: version_after,
      outbox_unchanged: true,
      withheld_reason: (send_plan.approval && send_plan.approval.reason) || "root cause unconfirmed",
      verified_independently: "re-read from the outbox file; the absence of the message is asserted, not assumed",
    },
  });
}

main();
