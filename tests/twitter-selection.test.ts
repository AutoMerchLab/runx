import { describe, expect, it } from "vitest";

import { selectArchivePage } from "../skills/twitter/twitter-selection.mjs";

describe("twitter archive selection", () => {
  it("binds plans to stable archive content rather than ephemeral artifact handles", () => {
    const first = select("runx:local-artifact:first");
    const second = select("runx:local-artifact:second");

    expect(first).toEqual(second);
    expect(first.evidence_refs).toEqual(["sha256:archive-content"]);
  });
});

function select(artifactRef: string) {
  return selectArchivePage({
    objective: "Delete old reposts",
    principal: "account:@fixture",
    max_acts: 10,
    selection_plan: {
      target: "posts",
      predicate: {
        is_retweet: true,
        before_year: 2026,
      },
      blockers: [],
    },
    runx_page: {
      artifact_ref: artifactRef,
      whole_digest: "sha256:archive-content",
      records: [
        JSON.stringify({
          tweet: {
            id_str: "123",
            full_text: "RT @example: archived",
            created_at: "Wed Jan 01 00:00:00 +0000 2025",
          },
        }),
      ],
      state: null,
      eof: true,
    },
  }).twitter_selection_draft.twitter_plan;
}
