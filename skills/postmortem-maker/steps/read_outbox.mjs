// postmortem-maker / step 2 of 5: read the publication outbox stream.
//
// WHAT THIS TRANSPORT IS, STATED PLAINLY: an append-only outbox log bundled
// with this skill, not a hosted provider and not the runx data-store. The
// canonical `runx/send-as` skill is a planning and authority layer that, by its
// own description, "never delivers" and refers delivery to a provider adapter;
// and the runtime's native `data.*` tools are not in the execution closure of a
// package installed from the registry. So this package ships its own provider
// adapter: a sealed local transport that really executes the delivery, keeps it
// durable across runs, and can be read back independently.
//
// This step returns the current stream version, which step 4 uses as its
// compare-and-set guard and step 5 uses as the "before" side of its
// delivery / no-delivery assertions.
//
// Helpers below are deliberately duplicated in steps/deliver.mjs and
// steps/verify.mjs: every file the runner executes must be self-contained,
// since only the files X.yaml names are guaranteed to ship in the package.

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

// Must match steps/deliver.mjs and steps/verify.mjs exactly: same addressing,
// same directory. Hashed so a target ref never escapes the working directory.
function outboxPath(target) {
  const key = `${target.data_source_ref}|${target.channel}|${target.aggregate_id}`;
  const id = createHash("sha256").update(key).digest("hex").slice(0, 16);
  const root = process.env.RUNX_CWD || process.cwd();
  return resolve(join(root, ".postmortem-outbox", `${id}.jsonl`));
}

function main() {
  const input = parseInput();
  const target = asObject(input.publish_target);
  for (const f of ["data_source_ref", "channel", "aggregate_id"]) {
    if (typeof target[f] !== "string" || target[f].length === 0) {
      return refuse(`publish_target.${f} is required`);
    }
  }

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

  seal({
    outbox: {
      provider: PROVIDER,
      data_source_ref: target.data_source_ref,
      channel: target.channel,
      aggregate_id: target.aggregate_id,
      version: records.length,
      store_path: path,
      message_refs: records.map((r) => r.message_ref),
    },
  });
}

main();
