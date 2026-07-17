import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

import { describe, expect, it } from "vitest";

const execFileAsync = promisify(execFile);
const nativeRunx = path.resolve("crates", "target", "debug", process.platform === "win32" ? "runx.exe" : "runx");

describe("hello-world example", () => {
  it("runs through the native CLI and writes a receipt", async () => {
    const tempDir = await mkdtemp(path.join(os.tmpdir(), "runx-hello-world-example-"));

    try {
      const { stdout, stderr } = await execFileAsync(
        requireNativeRunx(),
        [
          "skill",
          "examples/hello-world",
          "--message",
          "hello from docs",
          "--non-interactive",
          "--json",
        ],
        {
          cwd: path.resolve("."),
          env: {
            ...process.env,
            NO_COLOR: "1",
            RUNX_HOME: path.join(tempDir, "home"),
            RUNX_RECEIPT_DIR: path.join(tempDir, "receipts"),
          },
        },
      );

      expect(stderr).toContain("Prepared run");
      const result = JSON.parse(stdout) as {
        readonly status: string;
        readonly execution?: { readonly stdout?: string };
        readonly receipt?: { readonly schema?: string; readonly seal?: { readonly disposition?: string } };
      };
      expect(result.status).toBe("sealed");
      expect(JSON.parse(result.execution?.stdout ?? "null")).toEqual({ message: "hello from docs" });
      expect(result.receipt?.schema).toBe("runx.receipt.v1");
      expect(result.receipt?.seal?.disposition).toBe("closed");
    } finally {
      await rm(tempDir, { recursive: true, force: true });
    }
  });
});

function requireNativeRunx(): string {
  if (!existsSync(nativeRunx)) {
    throw new Error(`native example tests require a built runx binary at ${nativeRunx}`);
  }
  return nativeRunx;
}
