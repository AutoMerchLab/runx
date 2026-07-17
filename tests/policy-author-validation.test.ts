import { spawnSync } from "node:child_process";
import path from "node:path";

import { describe, expect, it } from "vitest";

const root = process.cwd();

describe("policy-author deterministic validation", () => {
  it("rejects authority widening before native lint", () => {
    const existing = policy(["issue-intake"]);
    const proposed = policy(["issue-intake", "issue-to-pr"]);
    const result = spawnSync(process.execPath, [path.join(root, "skills/policy-author/validate_policy.mjs")], {
      cwd: root,
      env: {
        ...process.env,
        RUNX_INPUTS_JSON: JSON.stringify({
          existing_policy: existing,
          policy_proposal: {
            decision: "ready",
            policy: proposed,
            rationale: "Add issue-to-pr.",
            blockers: [],
            needs_input: [],
            success_checkpoint: {},
          },
        }),
      },
      encoding: "utf8",
    });

    expect(result.status).toBe(0);
    const proposal = JSON.parse(result.stdout).policy_proposal;
    expect(proposal.decision).toBe("reject");
    expect(proposal.validation.status).toBe("fail");
    expect(proposal.validation.findings).toContainEqual({
      code: "policy.attenuation.widened",
      path: "source_id.github-issues.allowed_actions.issue-to-pr",
      message: "The tightening lane cannot add or widen this authority.",
    });
  });
});

function policy(actions: string[]) {
  return {
    sources: [{ source_id: "github-issues", allowed_locators: ["github://acme/acme/issues"], allowed_actions: actions }],
    runners: [{ runner_id: "local-review", allowed_actions: actions, target_repos: ["acme/acme"] }],
    targets: [{ repo: "acme/acme", allowed_actions: actions, runner_ids: ["local-review"] }],
    permissions: { auto_merge: false, mutate_target_repo: true, require_human_merge_gate: true },
  };
}
