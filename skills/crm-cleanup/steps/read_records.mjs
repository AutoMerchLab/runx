// crm-cleanup / step 1 of 3: read the account's current CRM records.
//
// Replays the append-only event log for (data_source_ref, resource,
// aggregate_id) and returns the projected record plus the stream version that
// step 3 uses as its compare-and-set guard.
//
// The records returned here are whatever earlier appends left in the log — they
// are NOT a fixture bundled with this package. On a stream that does not exist
// yet this returns version 0 and an empty record, which is a legitimate
// first-write case, and the reconcile step then reports every extracted value
// as new rather than as a change.

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const PROVIDER = "bundled-local-event-log";

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

// Must match steps/append_event.mjs exactly: same addressing, same directory.
function storePath(handle) {
  const key = `${handle.data_source_ref}|${handle.resource}|${handle.aggregate_id}`;
  const id = createHash("sha256").update(key).digest("hex").slice(0, 16);
  const root = process.env.RUNX_CWD || process.cwd();
  return resolve(join(root, ".crm-store", `${id}.jsonl`));
}

function main() {
  const input = parseInput();
  const handle = input.source_handle && typeof input.source_handle === "object" ? input.source_handle : null;
  if (!handle) return refuse("source_handle is required");
  for (const f of ["data_source_ref", "resource", "aggregate_id"]) {
    if (typeof handle[f] !== "string" || handle[f].length === 0) {
      return refuse(`source_handle.${f} is required`);
    }
  }

  const path = storePath(handle);
  let version = 0;
  let record = {};
  const event_refs = [];

  if (existsSync(path)) {
    const lines = readFileSync(path, "utf8").split("\n").filter((l) => l.trim().length > 0);
    for (const line of lines) {
      let entry;
      try {
        entry = JSON.parse(line);
      } catch (e) {
        return refuse(`Event log at ${path} is corrupt: unreadable line`);
      }
      version = typeof entry.version === "number" ? entry.version : version + 1;
      if (entry.event_ref) event_refs.push(entry.event_ref);
      const fields =
        (entry.event && entry.event.payload && entry.event.payload.fields) || null;
      if (fields && typeof fields === "object") record = { ...record, ...fields };
    }
  }

  seal({
    projection: {
      provider: PROVIDER,
      operation: "read_projection",
      data_source_ref: handle.data_source_ref,
      resource: handle.resource,
      aggregate_id: handle.aggregate_id,
      version,
      record,
      event_count: event_refs.length,
      last_event_ref: event_refs.length > 0 ? event_refs[event_refs.length - 1] : null,
      store_path: path,
      exists: existsSync(path),
    },
  });
}

main();
