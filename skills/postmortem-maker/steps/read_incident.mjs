// postmortem-maker / step 1 of 5: read the incident record from a real source.
//
// The dogfood path fetches a REAL incident thread over HTTPS at run time (a
// GitHub issue and its comments), so the sealed receipt records an actual
// source read rather than a hand-pasted fixture argument. Every event returned
// here carries the upstream id and URL, which the compose step cites as
// evidence for each timeline entry and root-cause claim.
//
// The harness path (`kind: "inline"`) replays a bundled thread instead, so the
// harness cases stay deterministic and egress-free wherever they run.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { get } from "node:https";

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

// Plain HTTPS GET. The global fetch() is not reliably available in the runner's
// node context, so this uses node:https directly (same approach the published
// bookkeeper graph uses for its runtime source read).
function getJson(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    const req = get(
      url,
      { headers: { "user-agent": "runx-postmortem-maker", accept: "application/vnd.github+json" } },
      (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location && redirects < 4) {
          res.resume();
          return getJson(res.headers.location, redirects + 1).then(resolve, reject);
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`source_http_${res.statusCode}`));
        }
        let body = "";
        res.on("data", (c) => (body += c));
        res.on("end", () => {
          try {
            resolve(JSON.parse(body));
          } catch (e) {
            reject(new Error("source returned non-JSON body"));
          }
        });
      }
    );
    req.on("error", reject);
    req.setTimeout(20000, () => req.destroy(new Error("source fetch timed out")));
  });
}

function normalizeEvent(e, i, fallbackUrl) {
  const text = String(e.text || e.body || "").trim();
  return {
    seq: i,
    id: String(e.id != null ? e.id : `event-${i}`),
    at: e.at || e.created_at || null,
    author: (e.author && String(e.author)) || (e.user && e.user.login) || "unknown",
    text,
    url: e.url || e.html_url || fallbackUrl || null,
  };
}

async function main() {
  const input = parseInput();
  const source = asObject(input.incident_source);
  const kind = String(source.kind || (source.ref ? "github_issue" : "")).trim();
  if (!kind) refuse("incident_source.kind is required (github_issue | inline)");

  let title = "";
  let ref = source.ref || null;
  let events = [];
  let read_mode = "";

  if (kind === "inline") {
    // Harness path: a bundled thread, replayed verbatim. Never used by the
    // dogfood, which must prove a real source read.
    const thread = asObject(source.thread);
    title = String(thread.title || "").trim();
    ref = ref || String(thread.url || "inline:incident-thread");
    const raw = Array.isArray(thread.events) ? thread.events : [];
    events = raw.map((e, i) => normalizeEvent(e, i, ref));
    read_mode = "inline-fixture";
  } else if (kind === "github_issue") {
    if (typeof ref !== "string" || !ref.startsWith("https://")) {
      refuse("incident_source.ref must be an https URL to a GitHub issue API resource");
    }
    let issue;
    try {
      issue = await getJson(ref);
    } catch (e) {
      refuse(`incident source fetch failed: ${e.message}`);
    }
    title = String(issue.title || "").trim();
    const opened = normalizeEvent(
      {
        id: issue.id,
        at: issue.created_at,
        author: issue.user && issue.user.login,
        text: issue.body,
        url: issue.html_url,
      },
      0,
      ref
    );
    let comments = [];
    const commentsUrl = issue.comments_url || `${ref.replace(/\/$/, "")}/comments`;
    try {
      const fetched = await getJson(`${commentsUrl}?per_page=100`);
      if (Array.isArray(fetched)) comments = fetched;
    } catch (e) {
      refuse(`incident comments fetch failed: ${e.message}`);
    }
    events = [opened].concat(
      comments.map((c, i) =>
        normalizeEvent(
          { id: c.id, at: c.created_at, author: c.user && c.user.login, text: c.body, url: c.html_url },
          i + 1,
          ref
        )
      )
    );
    read_mode = "runtime-web-fetch";
  } else {
    refuse(`unsupported incident_source.kind "${kind}" (expected github_issue | inline)`);
  }

  events = events.filter((e) => e.text.length > 0);
  if (events.length === 0) {
    refuse("incident source returned no readable events; nothing to reconstruct a postmortem from");
  }

  const incident = {
    ref,
    kind,
    read_mode,
    title,
    fetched_at: new Date().toISOString(),
    events_read: events.length,
    events,
    source_digest: digestOf(events.map((e) => ({ id: e.id, at: e.at, author: e.author, text: e.text }))),
  };

  seal({ incident });
}

main().catch((e) => refuse(`read_incident error: ${e.message}`));
