import { describe, expect, it } from "vitest";

import { finalizeExecution } from "../skills/twitter/twitter-execution-result.mjs";

describe("twitter execution provider outcomes", () => {
  it("treats following:false as a completed unfollow", () => {
    const execution = finalize("unfollow", { following: false });

    expect(execution).toMatchObject({
      decision: "executed",
      next_act_index: 1,
      remaining_count: 0,
      results: [
        {
          kind: "unfollow",
          status: "done",
          provider_ref: "target-1",
          detail: null,
        },
      ],
    });
  });

  it("requires each typed mutation to reach its requested state", () => {
    expect(finalize("unfollow", { following: true })).toMatchObject({
      decision: "partial",
      next_act_index: 0,
      results: [{ status: "failed" }],
    });
    expect(finalize("follow", { following: true })).toMatchObject({
      decision: "executed",
      next_act_index: 1,
      results: [{ status: "done" }],
    });
    expect(finalize("follow", { following: false })).toMatchObject({
      decision: "partial",
      next_act_index: 0,
      results: [{ status: "failed" }],
    });
    expect(finalize("delete_post", { deleted: false })).toMatchObject({
      decision: "partial",
      next_act_index: 0,
      results: [{ status: "failed" }],
    });
  });
});

function finalize(kind: string, data: Record<string, unknown>) {
  return finalizeExecution({
    execution_plan: {
      decision: "ready",
      plan_digest: "sha256:fixture",
      principal: "account:@fixture",
      start_act_index: 0,
      next_act_index: 0,
      total_act_count: 1,
      remaining_count: 1,
      act_groups: [
        {
          act_id: "act-001",
          act_index: 0,
          kind,
          consequence: "live_mutation",
          fallback_provider_ref: "target-1",
          request_ids: ["act:act-001"],
        },
      ],
    },
    http_execution: {
      responses: [
        {
          id: "act:act-001",
          ok: true,
          status: 200,
          json: { data },
        },
      ],
    },
  }).twitter_execution;
}
