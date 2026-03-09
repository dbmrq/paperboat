# Cursor ACP Transport

> **⚠️ Known Issue: MCP Tools Not Working**
>
> Cursor's ACP mode (`agent acp`) currently has a known issue where MCP servers are not loaded. This means MCP tools configured in `~/.cursor/mcp.json` will not be available to agents.
>
> **Recommended workaround:** Use the CLI transport instead:
> ```bash
> paperboat --backend cursor:cli "your task"
> ```
>
> The CLI transport (`agent --print`) correctly loads MCP servers and tools. This documentation is preserved for when Cursor fixes the issue.

## Overview

Cursor CLI supports ACP (Agent Client Protocol) for advanced integrations. You can run agent acp and connect a custom client over stdio using JSON-RPC.

Learn more in the official Agent Client Protocol docs.

ACP is intended for building custom clients and integrations. For normal terminal workflows, use the interactive CLI with agent.
Start ACP server

Start Cursor CLI in ACP mode:


agent acp
Transport and message format

Transport: stdio
Protocol envelope: JSON-RPC 2.0
Framing: newline-delimited JSON (one message per line)
Direction:
Client writes requests/notifications to stdin
Cursor CLI writes responses/notifications to stdout
Logs may be written to stderr
Request flow

Typical ACP session flow:

initialize
authenticate with methodId: "cursor_login"
session/new (or session/load)
session/prompt
Handle session/update notifications while the model streams output
Handle session/request_permission by returning a decision
Optionally send session/cancel
Authentication

Cursor CLI advertises cursor_login as the ACP auth method. In practice, you can pre-authenticate before startup using existing CLI auth paths:

agent login
--api-key (or CURSOR_API_KEY)
--auth-token (or CURSOR_AUTH_TOKEN)
You can also pass endpoint and TLS options from the root CLI command:


agent --api-key "$CURSOR_API_KEY" acp
agent -e https://api2.cursor.sh acp
agent -k acp
Sessions, modes, and permissions

Sessions

Create a session with session/new
Resume an existing conversation with session/load
Modes

ACP sessions support the same core modes as CLI:

agent (full tool access)
plan (planning, read-only behavior)
ask (Q&A/read-only behavior)
Permissions

When tools need approval, Cursor sends session/request_permission. Clients should return one of:

allow-once
allow-always
reject-once
If your client does not answer permission requests, tool execution can block.

Cursor extension methods

Cursor also sends ACP extension methods for richer client UX:

Method	Use
cursor/ask_question	Ask users multiple-choice questions
cursor/create_plan	Request explicit plan approval
cursor/update_todos	Notify client about todo state updates
cursor/task	Notify client about subagent task completion
cursor/generate_image	Notify client about generated image output
Clients can implement these methods to match Cursor-native interaction behavior.

Minimal Node.js client

This example shows the minimum control flow for a custom ACP client:


import { spawn } from "node:child_process";
import readline from "node:readline";
const agent = spawn("agent", ["acp"], { stdio: ["pipe", "pipe", "inherit"] });
let nextId = 1;
const pending = new Map();
function send(method, params) {
  const id = nextId++;
  agent.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}
function respond(id, result) {
  agent.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\n");
}
const rl = readline.createInterface({ input: agent.stdout });
rl.on("line", line => {
  const msg = JSON.parse(line);
  if (msg.id && (msg.result || msg.error)) {
    const waiter = pending.get(msg.id);
    if (!waiter) return;
    pending.delete(msg.id);
    msg.error ? waiter.reject(msg.error) : waiter.resolve(msg.result);
    return;
  }
  if (msg.method === "session/update") {
    const update = msg.params?.update;
    if (update?.sessionUpdate === "agent_message_chunk" && update.content?.text) {
      process.stdout.write(update.content.text);
    }
    return;
  }
  if (msg.method === "session/request_permission") {
    respond(msg.id, { outcome: { outcome: "selected", optionId: "allow-once" } });
  }
});
const init = async () => {
  await send("initialize", {
    protocolVersion: 1,
    clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
    clientInfo: { name: "acp-minimal-client", version: "0.1.0" }
  });
  await send("authenticate", { methodId: "cursor_login" });
  const { sessionId } = await send("session/new", { cwd: process.cwd(), mcpServers: [] });
  const result = await send("session/prompt", {
    sessionId,
    prompt: [{ type: "text", text: "Say hello in one sentence." }]
  });
  console.log(`\n\n[stopReason=${result.stopReason}]`);
};
init().finally(() => {
  agent.stdin.end();
  agent.kill();
});
IDE integrations

ACP enables Cursor's AI agent to work with editors beyond the Cursor desktop app. Build or use third-party integrations for your preferred development environment.

Example use cases

JetBrains IDEs — Connect IntelliJ IDEA, WebStorm, PyCharm, or other JetBrains IDEs to Cursor's agent. See the JetBrains integration guide for setup instructions.

Neovim (avante.nvim) — Use avante.nvim to connect Neovim to Cursor's agent through ACP. See Neovim setup below.

Zed — Integrate with Zed's modern editor by spawning agent acp and communicating over stdio. Zed extensions can implement the ACP client protocol to route AI requests to Cursor.

Custom editors — Any editor with extension support can implement an ACP client. Spawn the agent process, send JSON-RPC messages over stdio, and handle responses in your editor's UI.

Neovim (avante.nvim)

avante.nvim is a Neovim plugin that provides an AI-powered coding assistant. It supports ACP, so you can connect it to Cursor's agent for agentic coding inside Neovim.

Add the following to your lazy.nvim plugin configuration (e.g., ~/.config/nvim/lua/plugins/avante.lua):


return {
  {
    "yetone/avante.nvim",
    event = "VeryLazy",
    version = false,
    build = "make",
    opts = {
      provider = "cursor",
      mode = "agentic",
      acp_providers = {
        cursor = {
          command = os.getenv("HOME") .. "/.local/bin/agent",
          args = { "acp" },
          auth_method = "cursor_login",
          env = {
            HOME = os.getenv("HOME"),
            PATH = os.getenv("PATH"),
          },
        },
      },
    },
    dependencies = {
      "nvim-lua/plenary.nvim",
      "MunifTanjim/nui.nvim",
      "nvim-tree/nvim-web-devicons",
      {
        "MeanderingProgrammer/render-markdown.nvim",
        opts = {
          file_types = { "markdown", "Avante" },
        },
        ft = { "markdown", "Avante" },
      },
    },
  },
}
Key settings:

provider: Set to "cursor" to route requests through Cursor's agent.
mode: Set to "agentic" for full tool access (file edits, terminal commands). Use "normal" for chat-only mode.
command: Points to the agent binary. The default install path is ~/.local/bin/agent. Adjust if you installed it elsewhere.
auth_method: Uses "cursor_login". Run agent login in your terminal first to authenticate.
Building an integration

Spawn agent acp as a child process
Communicate over stdin/stdout using JSON-RPC
Handle session/update notifications to display streaming responses
Respond to session/request_permission when tools need approval
Optionally implement Cursor extension methods for richer UX
See the minimal Node.js client above for a working reference implementation.