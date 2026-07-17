import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
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

  it("fails closed when native lint rejects the authored policy", () => {
    const invalidPolicy = JSON.parse(readFileSync(
      path.join(root, "fixtures/operational-policy/invalid-secret-field.json"),
      "utf8",
    ));
    const result = spawnSync(process.execPath, [path.join(root, "skills/policy-author/validate_policy.mjs")], {
      cwd: root,
      env: {
        ...process.env,
        PATH: [path.join(root, "crates/target/debug"), process.env.PATH].filter(Boolean).join(path.delimiter),
        RUNX_CWD: root,
        RUNX_INPUTS_JSON: JSON.stringify({
          policy_proposal: {
            decision: "ready",
            policy: invalidPolicy,
            rationale: "Exercise the native rejection path.",
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
    expect(proposal.validation).toMatchObject({
      status: "fail",
      engine: "runx policy lint",
      readback: null,
    });
    expect(proposal.validation.findings).toContainEqual({
      code: "policy.native_lint.invalid",
      path: "$",
      message: "The proposal could not be parsed or validated by the native policy engine.",
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
