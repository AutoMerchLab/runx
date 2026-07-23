import { appendFileSync } from "node:fs";

let input = Buffer.alloc(0);
const startMarkerPath = process.env.RUNX_MCP_START_MARKER;
if (typeof startMarkerPath === "string" && startMarkerPath.length > 0) {
  appendLifecycle(startMarkerPath, "start");
}

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
      capabilities: {
        tools: {},
      },
      serverInfo: {
        name: "runx-rust-mcp-fixture",
        version: "0.0.0",
      },
    });
    return;
  }

  if (request.method === "tools/list") {
    respond(request.id, {
      tools: [
        {
          name: "echo",
          description: "Echo a message through the fixture MCP server.",
          inputSchema: {
            type: "object",
            properties: {
              message: {
                type: "string",
                description: "Message to echo.",
              },
            },
            required: ["message"],
            additionalProperties: false,
          },
        },
        {
          name: "fail",
          description: "Return a fixture MCP error for testing.",
          inputSchema: {
            type: "object",
            properties: {
              message: {
                type: "string",
              },
            },
            additionalProperties: false,
          },
        },
        {
          name: "sleep",
          description: "Never respond, for timeout testing.",
          inputSchema: {
            type: "object",
            properties: {},
            additionalProperties: false,
          },
        },
        {
          name: "env",
          description: "Return a single fixture server environment variable.",
          inputSchema: {
            type: "object",
            properties: {
              name: {
                type: "string",
              },
            },
            required: ["name"],
            additionalProperties: false,
          },
        },
      ],
    });
    return;
  }

  if (request.method === "tools/call") {
    handleToolCall(request.id, request.params);
    return;
  }

  respondError(request.id, -32601, "method not found");
}

function handleToolCall(id, params) {
  if (!isRecord(params) || typeof params.name !== "string") {
    respondError(id, -32602, "invalid tool call");
    return;
  }

  const args = isRecord(params.arguments) ? params.arguments : {};

  if (params.name === "sleep") {
    startLifecycleHeartbeat(args.markerPath);
    return;
  }

  if (params.name === "env") {
    respond(id, {
      content: [
        {
          type: "text",
          text: String(process.env[String(args.name ?? "")] ?? ""),
        },
      ],
    });
    return;
  }

  if (params.name === "fail") {
    respondError(id, -32000, `fixture failure: ${String(args.message ?? "")}`);
    return;
  }

  if (params.name !== "echo") {
    respondError(id, -32601, "tool not found");
    return;
  }

  respond(id, {
    content: [
      {
        type: "text",
        text: String(args.message ?? ""),
      },
    ],
  });
}

function respond(id, result) {
  write({
    jsonrpc: "2.0",
    id,
    result,
  });
}

function respondError(id, code, message) {
  write({
    jsonrpc: "2.0",
    id,
    error: {
      code,
      message,
    },
  });
}

function write(message) {
  const body = JSON.stringify(message);
  writeRaw(Buffer.byteLength(body, "utf8"), body);
}

function writeRaw(contentLength, body) {
  process.stdout.write(`Content-Length: ${contentLength}\r\n\r\n${body}`);
}

function startLifecycleHeartbeat(markerPath) {
  if (typeof markerPath !== "string" || markerPath.length === 0) {
    return;
  }
  appendLifecycle(markerPath, "sleep-start");
  setInterval(() => appendLifecycle(markerPath, "heartbeat"), 25);
}

function appendLifecycle(markerPath, event) {
  appendFileSync(markerPath, `${event} ${process.pid} ${Date.now()}\n`);
}

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
