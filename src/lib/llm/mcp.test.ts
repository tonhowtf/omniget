import { describe, expect, it } from "vitest";
import type { AgentDef, ToolGrant } from "./types";
import {
  MCP_REGISTRY,
  applyGrant,
  blankServer,
  formatArgs,
  formatPairs,
  grantModeOf,
  grantedCount,
  isSecretRef,
  maskSnippet,
  maskToken,
  mcpErrorKey,
  parseArgs,
  parseClientConfig,
  parsePairs,
  slugifyId,
  templateOf,
  toolSignature,
  validateServer,
  type McpServerConfig,
} from "./mcp";

function stdio(over: Partial<McpServerConfig> = {}): McpServerConfig {
  return {
    id: "filesystem",
    name: "Filesystem",
    enabled: true,
    transport: { kind: "stdio", command: "npx", args: [], env: {}, cwd: null },
    ...over,
  };
}

function http(url: string): McpServerConfig {
  return {
    id: "context7",
    name: "Context7",
    enabled: true,
    transport: { kind: "http", url, headers: {} },
  };
}

function agent(tools: ToolGrant[]): AgentDef {
  return {
    id: "omni",
    name: "Omni",
    role: "worker",
    system_prompt: "",
    model: { policy: "fixed", model: { provider: "openai", model: "gpt-5" } },
    tools,
    skills: [],
    budget: {},
    runtime: { kind: "native" },
  };
}

describe("validateServer", () => {
  it("accepts a plain stdio server", () => {
    expect(validateServer(stdio())).toEqual([]);
  });

  it("refuses a command that is only whitespace", () => {
    const config = stdio();
    (config.transport as { command: string }).command = "  ";
    expect(validateServer(config)).toEqual(["llm.mcp.invalid.command"]);
  });

  it("accepts exactly the ids core::mcp::types::valid_id accepts", () => {
    for (const id of ["filesystem", "github-mcp-2", "file_system", "-lead"]) {
      expect(validateServer(stdio({ id }))).toEqual([]);
    }
  });

  it("refuses ids the backend would refuse too", () => {
    for (const id of ["", "Filesystem", "file system", "a.b", "a/b", "a".repeat(65)]) {
      expect(validateServer(stdio({ id }))).toContain("llm.mcp.invalid.id");
    }
  });

  it("only accepts http(s) urls", () => {
    expect(validateServer(http("https://mcp.context7.com/mcp"))).toEqual([]);
    expect(validateServer(http("http://127.0.0.1:9000/mcp"))).toEqual([]);
    for (const url of ["", "mcp.context7.com", "ftp://x/y", "https://"]) {
      expect(validateServer(http(url))).toContain("llm.mcp.invalid.url");
    }
  });

  it("reports every problem at once, in field order", () => {
    const config = stdio({ id: "BAD", name: " " });
    (config.transport as { command: string }).command = "";
    expect(validateServer(config)).toEqual([
      "llm.mcp.invalid.id",
      "llm.mcp.invalid.name",
      "llm.mcp.invalid.command",
    ]);
  });
});

describe("slugifyId", () => {
  it("makes a valid id out of a display name", () => {
    expect(slugifyId("GitHub MCP")).toBe("github-mcp");
    expect(slugifyId("Contexto 7 — Ção")).toBe("contexto-7-cao");
  });

  it("never returns an empty id", () => {
    expect(slugifyId("!!!")).toBe("server");
    expect(slugifyId("")).toBe("server");
  });
});

describe("parseArgs", () => {
  it("splits on whitespace", () => {
    expect(parseArgs("-y @scope/server /tmp")).toEqual(["-y", "@scope/server", "/tmp"]);
  });

  it("keeps a quoted path in one argument", () => {
    expect(parseArgs('--root "/Users/me/My Files" -v')).toEqual([
      "--root",
      "/Users/me/My Files",
      "-v",
    ]);
    expect(parseArgs("--root '/a b/c'")).toEqual(["--root", "/a b/c"]);
  });

  it("keeps an empty quoted argument", () => {
    expect(parseArgs('--flag ""')).toEqual(["--flag", ""]);
  });

  it("is empty for blank input", () => {
    expect(parseArgs("   ")).toEqual([]);
  });

  it("round-trips through formatArgs", () => {
    const args = ["--root", "/Users/me/My Files", "-v"];
    expect(parseArgs(formatArgs(args))).toEqual(args);
  });
});

describe("parsePairs", () => {
  it("reads env lines", () => {
    expect(parsePairs("TOKEN=abc\nNO_PROXY=1")).toEqual({ TOKEN: "abc", NO_PROXY: "1" });
  });

  it("reads header lines", () => {
    expect(parsePairs("Authorization: Bearer x\nX-Trace: 1")).toEqual({
      Authorization: "Bearer x",
      "X-Trace": "1",
    });
  });

  it("keeps the rest of a value that carries the other separator", () => {
    expect(parsePairs("Authorization: secret:gh")).toEqual({ Authorization: "secret:gh" });
    expect(parsePairs("URL=https://a/b")).toEqual({ URL: "https://a/b" });
  });

  it("drops blank lines, comments and keyless lines", () => {
    expect(parsePairs("\n# a comment\n=orphan\nA=1\n")).toEqual({ A: "1" });
  });

  it("round-trips through formatPairs", () => {
    const pairs = { TOKEN: "abc", NO_PROXY: "1" };
    expect(parsePairs(formatPairs(pairs))).toEqual(pairs);
    expect(parsePairs(formatPairs(pairs, ": "))).toEqual(pairs);
  });
});

describe("parseClientConfig", () => {
  it("reads the mcpServers block other clients use", () => {
    const text = JSON.stringify({
      mcpServers: {
        fetch: { command: "uvx", args: ["mcp-server-fetch"] },
        context7: { url: "https://mcp.context7.com/mcp", headers: { A: "b" } },
      },
    });
    const parsed = parseClientConfig(text);
    expect(parsed).toHaveLength(2);
    expect(parsed[0]).toEqual({
      id: "fetch",
      name: "fetch",
      enabled: true,
      transport: {
        kind: "stdio",
        command: "uvx",
        args: ["mcp-server-fetch"],
        env: {},
        cwd: null,
      },
    });
    expect(parsed[1].transport).toEqual({
      kind: "http",
      url: "https://mcp.context7.com/mcp",
      headers: { A: "b" },
    });
    for (const config of parsed) expect(validateServer(config)).toEqual([]);
  });

  it("accepts the bare map and slugifies keys that are not ids", () => {
    const parsed = parseClientConfig('{"GitHub MCP": {"command": "gh-mcp"}}');
    expect(parsed).toHaveLength(1);
    expect(parsed[0].id).toBe("github-mcp");
    expect(parsed[0].name).toBe("GitHub MCP");
  });

  it("honours `disabled`", () => {
    const parsed = parseClientConfig('{"mcpServers":{"a":{"command":"x","disabled":true}}}');
    expect(parsed[0].enabled).toBe(false);
  });

  it("returns nothing instead of throwing on junk", () => {
    expect(parseClientConfig("not json")).toEqual([]);
    expect(parseClientConfig("[]")).toEqual([]);
    expect(parseClientConfig('{"mcpServers":{"a":{"nothing":1}}}')).toEqual([]);
  });
});

describe("the server half helpers", () => {
  it("masks a token but keeps its shape", () => {
    expect(maskToken("abcdefghijkl")).toBe("abcd…ijkl");
    expect(maskToken("short")).toBe("••••");
  });

  it("masks every occurrence in a snippet, and none when revealed", () => {
    const snippet = 'claude mcp add --header "Authorization: Bearer tok-123456789" tok-123456789';
    const masked = maskSnippet(snippet, "tok-123456789", false);
    expect(masked).not.toContain("tok-123456789");
    expect(masked.match(/tok-…6789/g)).toHaveLength(2);
    expect(maskSnippet(snippet, "tok-123456789", true)).toBe(snippet);
  });

  it("marks optional arguments with a question mark", () => {
    expect(
      toolSignature({ properties: { url: {}, quality: {} }, required: ["url"] }),
    ).toBe("url, quality?");
    expect(toolSignature({})).toBe("");
    expect(toolSignature(null)).toBe("");
  });
});

describe("grants", () => {
  it("adds, replaces and clears one grant", () => {
    let tools = applyGrant([], "filesystem", "read_file", "auto");
    expect(tools).toEqual([
      { source: "mcp", server: "filesystem", tool: "read_file", mode: "auto" },
    ]);
    tools = applyGrant(tools, "filesystem", "read_file", "ask");
    expect(tools).toHaveLength(1);
    expect(tools[0].mode).toBe("ask");
    expect(applyGrant(tools, "filesystem", "read_file", null)).toEqual([]);
  });

  it("does not confuse the same tool name on another server", () => {
    const tools = applyGrant(
      applyGrant([], "filesystem", "read_file", "auto"),
      "github",
      "read_file",
      "deny",
    );
    expect(tools).toHaveLength(2);
    expect(grantModeOf(agent(tools), "filesystem", "read_file")).toBe("auto");
    expect(grantModeOf(agent(tools), "github", "read_file")).toBe("deny");
  });

  it("leaves the internal grants alone", () => {
    const internal: ToolGrant = { source: "internal", name: "download_url", mode: "auto" };
    const tools = applyGrant([internal], "filesystem", "read_file", "auto");
    expect(tools[0]).toBe(internal);
  });

  it("answers null for a tool that was never granted", () => {
    expect(grantModeOf(agent([]), "filesystem", "read_file")).toBeNull();
    expect(grantModeOf(null, "filesystem", "read_file")).toBeNull();
  });

  it("counts only what the agent may actually run", () => {
    const tools: ToolGrant[] = [
      { source: "mcp", server: "fs", tool: "a", mode: "auto" },
      { source: "mcp", server: "fs", tool: "b", mode: "ask" },
      { source: "mcp", server: "fs", tool: "c", mode: "deny" },
      { source: "mcp", server: "gh", tool: "d", mode: "auto" },
    ];
    expect(grantedCount(agent(tools), "fs")).toBe(2);
  });
});

describe("the registry shown in the tab", () => {
  it("carries the five servers the core records fixtures for, then Microsoft 365", () => {
    expect(MCP_REGISTRY.map((e) => e.id)).toEqual([
      "filesystem",
      "github",
      "fetch",
      "playwright",
      "context7",
      "microsoft365",
    ]);
  });

  it("only offers templates that pass validation", () => {
    for (const entry of MCP_REGISTRY) {
      expect(validateServer(entry.template)).toEqual([]);
    }
  });

  it("hands out a copy, so editing the form never touches the registry", () => {
    const entry = MCP_REGISTRY[0];
    const copy = templateOf(entry);
    copy.name = "edited";
    (copy.transport as { command: string }).command = "edited";
    expect(entry.template.name).toBe("Filesystem");
    expect((entry.template.transport as { command: string }).command).toBe("npx");
  });

  it("uses a secret reference instead of a token for the GitHub header", () => {
    const github = MCP_REGISTRY.find((e) => e.id === "github")!;
    const headers = (github.template.transport as { headers: Record<string, string> }).headers;
    expect(isSecretRef(headers.Authorization)).toBe(true);
    expect(isSecretRef("Bearer real-token")).toBe(false);
  });
});

describe("errors", () => {
  it("maps every documented code to its own key", () => {
    expect(mcpErrorKey("ERR_MCP_SPAWN: no such file")).toBe("llm.mcp.err.spawn");
    expect(mcpErrorKey("ERR_MCP_TIMEOUT")).toBe("llm.mcp.err.timeout");
    expect(mcpErrorKey("ERR_MCP_CONFIG: url")).toBe("llm.mcp.err.config");
    expect(mcpErrorKey("boom")).toBe("llm.mcp.err.generic");
    expect(mcpErrorKey(null)).toBe("llm.mcp.err.generic");
  });
});

describe("blankServer", () => {
  it("starts on stdio and fails validation until it is filled in", () => {
    const blank = blankServer();
    expect(blank.transport.kind).toBe("stdio");
    expect(validateServer(blank)).toEqual([
      "llm.mcp.invalid.id",
      "llm.mcp.invalid.name",
      "llm.mcp.invalid.command",
    ]);
  });
});
