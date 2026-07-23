import { spawnSync } from "node:child_process";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { beforeAll, describe, expect, it } from "vitest";

import { ensureRunxBinary, kernelTestEnv, runxBinary } from "./host-protocol-test-utils.js";
import { appendLedgerEntries, createRunEventEntry } from "./ledger-fixtures.js";

interface CommandMatrix {
  readonly exitCodes: readonly number[];
  readonly commands: readonly CommandEntry[];
}

interface CommandEntry {
  readonly id: string;
  readonly topLevelUsage: readonly string[];
  readonly usage: readonly string[];
  readonly notes: readonly string[];
  readonly options: readonly string[];
  readonly exitCodes: readonly number[];
  readonly parity: {
    readonly humanOutput: "semantic" | "none";
    readonly jsonOutput: "schema-exact" | "none";
    readonly receipt: "schema-exact" | "none";
    readonly sideEffect: string;
    readonly surfaces: readonly string[];
  };
  readonly cases: readonly string[];
}

interface RuntimeSurfaces {
  readonly surfaces: readonly {
    readonly id: string;
    readonly owner: string;
    readonly parityClass: string;
    readonly coveredBy: readonly string[];
  }[];
}

interface OracleCases {
  readonly cases: readonly OracleCase[];
}

interface OracleCase {
  readonly id: string;
  readonly commandId: string;
  readonly mode: "execute" | "validate";
  readonly argv?: readonly string[];
  readonly expectedExitCode?: number;
  readonly expectJson?: boolean;
  readonly stdoutIncludes?: readonly string[];
  readonly stderrIncludes?: readonly string[];
  readonly proves: readonly string[];
}

describe("CLI feature parity matrix", () => {
  beforeAll(() => {
    ensureRunxBinary();
  });

  it("covers every command with at least one oracle case", async () => {
    const matrix = await readJson<CommandMatrix>("fixtures/cli-parity/commands.json");
    const oracle = await readOracleCases();
    const casesByCommand = new Map<string, OracleCase[]>();

    for (const testCase of oracle) {
      const cases = casesByCommand.get(testCase.commandId) ?? [];
      cases.push(testCase);
      casesByCommand.set(testCase.commandId, cases);
    }

    expect(matrix.exitCodes).toEqual([0, 1, 2, 3, 64]);
    for (const command of matrix.commands) {
      expect(command.exitCodes).toEqual(matrix.exitCodes);
      expect(command.parity.surfaces.length).toBeGreaterThan(0);
      expect(casesByCommand.get(command.id)?.length ?? 0).toBeGreaterThan(0);
    }
  });

  it("connects every runtime surface to a command and oracle case", async () => {
    const matrix = await readJson<CommandMatrix>("fixtures/cli-parity/commands.json");
    const runtime = await readJson<RuntimeSurfaces>("fixtures/cli-parity/runtime-surfaces.json");
    const oracle = await readOracleCases();
    const commandIds = new Set(matrix.commands.map((command) => command.id));
    const provenSurfaces = new Set(oracle.flatMap((testCase) => testCase.proves));

    for (const surface of runtime.surfaces) {
      expect(surface.coveredBy.length).toBeGreaterThan(0);
      for (const commandId of surface.coveredBy) {
        expect(commandIds.has(commandId)).toBe(true);
      }
      expect(provenSurfaces.has(surface.id)).toBe(true);
    }
  });

  it("executes deterministic oracle cases against the native CLI", async () => {
    const executableCases = (await readOracleCases()).filter((testCase) => testCase.mode === "execute");

    for (const testCase of executableCases) {
      const tempDir = await mkdtemp(path.join(os.tmpdir(), `runx-cli-parity-${testCase.id}-`));

      try {
        const receiptDir = path.join(tempDir, "receipts");
        await prepareOracleFixtures(testCase, receiptDir);
        const argv = (testCase.argv ?? []).map((arg) =>
          arg === "$FIXTURE_RECEIPTS" ? receiptDir : arg,
        );
        const result = spawnSync(runxBinary, argv, {
          cwd: process.cwd(),
          encoding: "utf8",
          env: {
            ...kernelTestEnv(process.env),
            RUNX_CWD: process.cwd(),
            RUNX_HOME: path.join(tempDir, "home"),
            RUNX_RECEIPT_DIR: receiptDir,
            RUNX_BANNER: "0",
          },
        });
        if (result.error) {
          throw result.error;
        }
        const stdout = result.stdout ?? "";
        const stderr = result.stderr ?? "";
        const exitCode = result.status ?? 1;

        expect(exitCode, testCase.id).toBe(testCase.expectedExitCode);
        for (const expected of testCase.stdoutIncludes ?? []) {
          expect(stdout, testCase.id).toContain(expected);
        }
        for (const expected of testCase.stderrIncludes ?? []) {
          expect(stderr, testCase.id).toContain(expected);
        }
        if (testCase.expectJson) {
          expect(() => JSON.parse(stdout), testCase.id).not.toThrow();
        }
      } finally {
        await rm(tempDir, { recursive: true, force: true });
      }
    }
  }, 20_000);
});

async function prepareOracleFixtures(testCase: OracleCase, receiptDir: string): Promise<void> {
  if (!testCase.argv?.includes("$FIXTURE_RECEIPTS")) {
    return;
  }
  if (testCase.id === "history.execute") {
    await appendLedgerEntries({
      receiptDir,
      runId: "gx_needs_agent_oracle",
      entries: [
        createRunEventEntry({
          runId: "gx_needs_agent_oracle",
          producer: { skill: "sourcey", runner: "graph" },
          kind: "run_started",
          status: "started",
          createdAt: "2026-04-28T01:00:00.000Z",
        }),
        createRunEventEntry({
          runId: "gx_needs_agent_oracle",
          stepId: "discover",
          producer: { skill: "sourcey", runner: "graph" },
          kind: "step_waiting_resolution",
          status: "waiting",
          detail: {
            request_ids: ["agent_task.test-step.output"],
            resolution_kinds: ["agent_act"],
            step_ids: ["discover"],
            step_labels: ["inspect repo"],
            inputs: {},
            selected_runner: "agent-task",
          },
          createdAt: "2026-04-28T01:00:00.000Z",
        }),
      ],
    });
  }
}

async function readOracleCases(): Promise<readonly OracleCase[]> {
  const directory = "fixtures/cli-parity/cases";
  const names = (await readdir(directory)).filter((name) => name.endsWith(".json"));
  const files = await Promise.all(names.map((name) => readJson<OracleCases>(path.join(directory, name))));
  return files.flatMap((file) => file.cases);
}

async function readJson<T>(filePath: string): Promise<T> {
  return JSON.parse(await readFile(filePath, "utf8")) as T;
}
