import { createHash } from "node:crypto";

let input = Buffer.alloc(0);

process.stdin.on("data", (chunk) => {
  input = Buffer.concat([input, chunk]);
  parseAvailableMessages();
});

function parseAvailableMessages() {
  while (true) {
    const headerEnd = input.indexOf("\r\n\r\n");
    if (headerEnd === -1) {
      return;
    }
    const header = input.subarray(0, headerEnd).toString("utf8");
    const match = /Content-Length:\s*(\d+)/i.exec(header);
    if (!match) {
      return;
    }
    const bodyStart = headerEnd + 4;
    const bodyEnd = bodyStart + Number(match[1]);
    if (input.length < bodyEnd) {
      return;
    }
    const body = input.subarray(bodyStart, bodyEnd).toString("utf8");
    input = input.subarray(bodyEnd);
    handle(JSON.parse(body));
  }
}

function handle(request) {
  if (request.id === undefined) {
    return;
  }
  if (request.method === "initialize") {
    respond(request.id, {
      protocolVersion: "2025-06-18",
      capabilities: { tools: {} },
      serverInfo: { name: "runx-github-mcp-fixture", version: "0.0.0" },
    });
    return;
  }
  if (request.method === "tools/list") {
    respond(request.id, { tools: githubTools() });
    return;
  }
  if (request.method === "tools/call") {
    handleToolCall(request.id, request.params);
    return;
  }
  respondError(request.id, -32601, "method not found");
}

function githubTools() {
  return [
    tool("github_issue_read", "Return a deterministic GitHub issue fixture.", {
      repository: "string",
      number: "string",
    }, ["repository", "number"]),
    tool("github_issue_comment", "Return a deterministic GitHub issue comment fixture.", {
      repository: "string",
      number: "string",
      body: "string",
    }, ["repository", "number", "body"]),
    tool("github_pr_review_note", "Return a deterministic GitHub PR review-note fixture.", {
      repository: "string",
      number: "string",
      body: "string",
    }, ["repository", "number", "body"]),
    tool("github_pr_merge", "Return a deterministic GitHub PR merge fixture.", {
      repository: "string",
      number: "string",
    }, ["repository", "number"]),
  ];
}

function tool(name, description, properties, required) {
  return {
    name,
    description,
    inputSchema: {
      type: "object",
      properties: Object.fromEntries(
        Object.entries(properties).map(([key, type]) => [key, { type, description: `${key}.` }]),
      ),
      required,
      additionalProperties: false,
    },
  };
}

function handleToolCall(id, params) {
  if (!isRecord(params) || typeof params.name !== "string") {
    respondError(id, -32602, "invalid tool call");
    return;
  }
  const args = isRecord(params.arguments) ? params.arguments : {};
  if (params.name === "github_issue_read") {
    respondText(id, {
      repository: String(args.repository ?? ""),
      number: String(args.number ?? ""),
      title: "Governed MCP fixture issue",
      state: "open",
      body: "A deterministic issue snapshot served through the MCP fixture.",
    });
    return;
  }
  if (params.name === "github_issue_comment") {
    respondText(id, {
      repository: String(args.repository ?? ""),
      number: String(args.number ?? ""),
      comment_id: "issue-comment-fixture-001",
      body_sha256: sha256(String(args.body ?? "")),
    });
    return;
  }
  if (params.name === "github_pr_review_note") {
    respondText(id, {
      repository: String(args.repository ?? ""),
      number: String(args.number ?? ""),
      review_note_id: "github-pr-comment-fixture-001",
      body_sha256: sha256(String(args.body ?? "")),
    });
    return;
  }
  if (params.name === "github_pr_merge") {
    respondText(id, {
      repository: String(args.repository ?? ""),
      number: String(args.number ?? ""),
      merge_commit_sha: "0000000000000000000000000000000000000000",
    });
    return;
  }
  respondError(id, -32601, "tool not found");
}

function respondText(id, value) {
  respond(id, {
    content: [{ type: "text", text: JSON.stringify(value) }],
  });
}

function respond(id, result) {
  write({ jsonrpc: "2.0", id, result });
}

function respondError(id, code, message) {
  write({ jsonrpc: "2.0", id, error: { code, message } });
}

function write(message) {
  const body = JSON.stringify(message);
  process.stdout.write(`Content-Length: ${Buffer.byteLength(body, "utf8")}\r\n\r\n${body}`);
}

function sha256(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
