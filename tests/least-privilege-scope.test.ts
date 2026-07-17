import { spawnSync } from "node:child_process";
import path from "node:path";

import { describe, expect, it } from "vitest";

const root = process.cwd();

describe("least-privilege scope normalization", () => {
  it("reads canonical Runx scopes as resource:verb", () => {
    const result = spawnSync(process.execPath, [path.join(root, "skills/least-privilege/run.mjs")], {
      cwd: root,
      env: {
        ...process.env,
        RUNX_INPUTS_JSON: JSON.stringify({
          subject: "growth/lifecycle-campaign-send",
          granted_scopes: ["email:send", "repo:write", "payment:spend"],
          receipt_ids: ["rcpt_campaign_1"],
          ledger_evidence: {
            matched_receipts: [{ receipt_id: "rcpt_campaign_1" }],
            receipt_details: [{
              id: "rcpt_campaign_1",
              authority: { exercised_scopes: [{ scope: "email:send" }, { scope: "repo:read" }] },
            }],
          },
        }),
      },
      encoding: "utf8",
    });

    expect(result.status).toBe(0);
    const report = JSON.parse(result.stdout).audit_report;
    expect(report.scope_diff.map(({ normalized }: { normalized: unknown }) => normalized)).toEqual([
      { verb: "send", resource: "email", conditions: null },
      { verb: "write", resource: "repo", conditions: null },
      { verb: "spend", resource: "payment", conditions: null },
    ]);
    expect(report.narrowed_scopes).toEqual([{ from: "repo:write", to: "repo:read" }]);
  });
});
