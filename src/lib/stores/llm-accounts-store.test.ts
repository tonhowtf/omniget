import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
const unlisten = vi.fn();
const listen = vi.fn(async (_event: string, _cb: (ev: unknown) => void) => unlisten);

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (ev: unknown) => void) => listen(event, cb),
}));

type Store = typeof import("./llm-accounts-store.svelte");

let store: Store;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./llm-accounts-store.svelte");
});

afterEach(() => {
  invoke.mockReset();
  listen.mockClear();
  unlisten.mockClear();
  store.resetAccountsStore();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

const NOW = 1_800_000_000_000;

function account(id: string, extra: Partial<import("./llm-accounts-store.svelte").AccountView> = {}) {
  return {
    id,
    cli: "claude" as const,
    label: id,
    config_dir: `/p/${id}`,
    disabled: false,
    sandbox: "read-only" as const,
    sandbox_flag: "",
    windows: [],
    sessions: 0,
    active: false,
    last_error: null,
    ...extra,
  };
}

function win(
  window: import("./llm-accounts-store.svelte").WindowKind,
  used: number | null,
  resets_at_ms: number | null,
  account_id = "a",
): import("./llm-accounts-store.svelte").UsageWindow {
  return { account_id, window, used, used_tokens: 0, resets_at_ms, source: "reported" };
}

describe("normalizeSnapshot", () => {
  it("survives an empty payload", () => {
    const s = store.normalizeSnapshot(null);
    expect(s.accounts).toEqual([]);
    expect(s.chain).toEqual([]);
    expect(s.quota_available).toBe(false);
  });

  it("keeps rotation off unless the backend says otherwise", () => {
    expect(store.normalizeSnapshot({}).rotation.enabled).toBe(false);
    expect(store.normalizeSnapshot({ rotation: { enabled: true, threshold: 0.8, cooldown_s: 60 } }).rotation)
      .toEqual({ enabled: true, threshold: 0.8, cooldown_s: 60 });
  });

  it("clamps the used ratio and defaults the source to estimated", () => {
    const s = store.normalizeSnapshot({
      accounts: [
        account("a", {
          // Deliberately wrong on the wire: the store has to survive it.
          windows: [{ window: "five_hour", used: 3, resets_at_ms: null, source: "nope" }] as never,
        }),
      ],
    });
    expect(s.accounts[0].windows[0].used).toBe(1);
    expect(s.accounts[0].windows[0].source).toBe("estimated");
    expect(s.accounts[0].windows[0].account_id).toBe("a");
  });

  it("keeps a window with no plan ceiling as null instead of zero", () => {
    // `cli_usage` sends `used: null` when no ceiling is published: drawing 0%
    // would claim a measurement nobody made.
    const w = store.normalizeWindow({ window: "seven_day", used: null, used_tokens: 4200 });
    expect(w.used).toBeNull();
    expect(w.used_tokens).toBe(4200);
  });

  it("reads an account with no sandbox field as read-only", () => {
    // Nothing on the wire must ever be drawn as a write-enabled account.
    const { sandbox, sandbox_flag, ...bare } = account("a");
    const s = store.normalizeSnapshot({ accounts: [bare] });
    expect(s.accounts[0].sandbox).toBe("read-only");
    expect(s.accounts[0].sandbox_flag).toBe("");
  });

  it("only the exact word write is a write", () => {
    expect(store.normalizeSandbox("write")).toBe("write");
    expect(store.normalizeSandbox("full")).toBe("full");
    for (const bad of [undefined, null, "", "WRITE", "workspace-write", "danger-full-access", 1, {}]) {
      expect(store.normalizeSandbox(bad)).toBe("read-only");
    }
  });

  it("keeps the sandbox the backend reported, flag and all", () => {
    const s = store.normalizeSnapshot({
      accounts: [account("a", { sandbox: "write", sandbox_flag: "--permission-mode acceptEdits" })],
    });
    expect(s.accounts[0].sandbox).toBe("write");
    expect(s.accounts[0].sandbox_flag).toBe("--permission-mode acceptEdits");
  });

  it("drops chain ids that no longer exist and appends new accounts", () => {
    const s = store.normalizeSnapshot({
      accounts: [account("a"), account("b")],
      chain: ["ghost", "b"],
    });
    expect(s.chain).toEqual(["b", "a"]);
  });
});

describe("normalizeReport", () => {
  it("pads every heatmap row to 24 hours", () => {
    const r = store.normalizeReport({ by_tool_hour: [[1, 2, 3]] } as never);
    expect(r.by_tool_hour[0]).toHaveLength(24);
    expect(r.by_tool_hour[0][23]).toBe(0);
  });

  it("returns an empty report for garbage", () => {
    expect(store.normalizeReport(undefined).by_day).toEqual([]);
  });
});

describe("chain order", () => {
  it("orders accounts by the chain and keeps the rest", () => {
    const accounts = [account("a"), account("b"), account("c")];
    expect(store.orderByChain(accounts, ["c", "a"]).map((a) => a.id)).toEqual(["c", "a", "b"]);
  });

  it("reorders to an index and clamps out-of-range targets", () => {
    const chain = ["a", "b", "c"];
    expect(store.reorder(chain, "c", 0)).toEqual(["c", "a", "b"]);
    expect(store.reorder(chain, "a", 99)).toEqual(["b", "c", "a"]);
    expect(store.reorder(chain, "zz", 0)).toEqual(chain);
  });
});

describe("quota math", () => {
  it("picks the window under most pressure", () => {
    const w = store.worstWindow([
      win("five_hour", 0.2, null),
      win("seven_day", 0.8, null),
    ]);
    expect(w?.window).toBe("seven_day");
    expect(store.worstWindow([])).toBeNull();
  });

  it("does not let a ceiling-less window win over a measured one", () => {
    const w = store.worstWindow([win("five_hour", 0.2, null), win("seven_day", null, null)]);
    expect(w?.window).toBe("five_hour");
  });

  it("projects when the window runs out at the current pace", () => {
    // Half of the 5 h window spent with 4 h still to go: 1 h elapsed, so the
    // remaining half takes another hour.
    const at = store.exhaustsAt(win("five_hour", 0.5, NOW + 4 * 3_600_000), NOW);
    expect(at).not.toBeNull();
    expect(Math.round(((at as number) - NOW) / 60_000)).toBe(60);
  });

  it("gives no projection when it would land after the flip", () => {
    const at = store.exhaustsAt(win("five_hour", 0.1, NOW + 3_600_000), NOW);
    expect(at).toBeNull();
  });

  it("gives no projection without a reset time, a ratio, or anything spent", () => {
    expect(store.exhaustsAt(win("five_hour", 0.5, null), NOW)).toBeNull();
    expect(store.exhaustsAt(win("five_hour", 0, NOW + 3_600_000), NOW)).toBeNull();
    expect(store.exhaustsAt(win("five_hour", null, NOW + 3_600_000), NOW)).toBeNull();
  });

  it("formats a duration in hours and minutes", () => {
    expect(store.humanDuration(2 * 3_600_000 + 15 * 60_000)).toBe("2 h 15 min");
    expect(store.humanDuration(45 * 60_000)).toBe("45 min");
    expect(store.humanDuration(-1)).toBe("");
  });
});

describe("mergeWindows", () => {
  it("replaces a cached window and keeps one the sweep did not see", () => {
    const acc = [
      account("a", { windows: [win("five_hour", 0.1, null), win("seven_day", 0.2, null)] }),
      account("b"),
    ];
    const merged = store.mergeWindows(acc, [win("five_hour", 0.9, null, "a")]);
    expect(merged[0].windows.map((w) => w.used)).toEqual([0.9, 0.2]);
    expect(merged[1].windows).toEqual([]);
  });

  it("returns the accounts untouched when the sweep found nothing", () => {
    const acc = [account("a")];
    expect(store.mergeWindows(acc, [])).toBe(acc);
  });
});

describe("heatmap scale", () => {
  it("buckets a value into the five sequential steps", () => {
    expect(store.cellLevel(0, 100)).toBe(0);
    expect(store.cellLevel(10, 100)).toBe(1);
    expect(store.cellLevel(40, 100)).toBe(2);
    expect(store.cellLevel(70, 100)).toBe(3);
    expect(store.cellLevel(100, 100)).toBe(4);
    expect(store.cellLevel(5, 0)).toBe(0);
  });

  it("reads the max and the total of a grid", () => {
    const grid = [
      [1, 2, 3],
      [0, 9, 0],
    ];
    expect(store.gridMax(grid)).toBe(9);
    expect(store.gridTotal(grid)).toBe(15);
  });
});

describe("command palette entries", () => {
  it("offers one switch per enabled account plus the tab", () => {
    const items = store.accountPaletteItems(
      [account("a", { label: "Max pessoal" }), account("b", { disabled: true })],
      { group: "LLM", switchTo: (l) => `Switch to ${l}`, openTab: "Accounts" },
      { activate: () => {}, open: () => {} },
    );
    expect(items.map((i) => i.id)).toEqual(["nav-llm-accounts", "llm-account-a"]);
    expect(items[1].label).toBe("Switch to Max pessoal");
  });

  it("wires the action to the account id", () => {
    const seen: string[] = [];
    const items = store.accountPaletteItems(
      [account("a")],
      { group: "LLM", switchTo: (l) => l, openTab: "Accounts" },
      { activate: (id) => seen.push(id), open: () => {} },
    );
    items[1].action();
    expect(seen).toEqual(["a"]);
  });
});

describe("switch timeline", () => {
  it("keeps the newest first and stays bounded", () => {
    let list: import("./llm-accounts-store.svelte").SwitchRow[] = [];
    for (let i = 0; i < store.MAX_SWITCHES + 5; i++) {
      list = store.pushSwitch(list, { at_ms: i, agent: "a", from: "x", to: `y${i}`, why: "ERR_CLI_RATE" });
    }
    expect(list).toHaveLength(store.MAX_SWITCHES);
    expect(list[0].to).toBe(`y${store.MAX_SWITCHES + 4}`);
  });
});

describe("store IO", () => {
  it("loads accounts and marks them ready", async () => {
    invoke.mockResolvedValueOnce({ accounts: [account("a")], chain: ["a"], quota_available: true });
    await store.loadAccounts();
    expect(invoke).toHaveBeenCalledWith("llm_accounts_list");
    expect(store.getAccountsState()).toBe("ready");
    expect(store.getAccounts().accounts).toHaveLength(1);
  });

  it("treats a failing command as unavailable instead of crashing", async () => {
    invoke.mockRejectedValueOnce("ERR_STUB");
    await store.loadAccounts();
    expect(store.getAccountsState()).toBe("unavailable");
    expect(store.getAccountsError()).toContain("ERR_STUB");
  });

  it("renders an empty dashboard when the usage report is unavailable", async () => {
    invoke.mockRejectedValueOnce("ERR_CLI_USAGE_UNAVAILABLE");
    await store.loadUsage();
    expect(store.getUsageState()).toBe("unavailable");
    expect(store.getUsage().by_day).toEqual([]);
  });

  it("hands the sweep's windows to the accounts and flips the quota flag", async () => {
    invoke.mockResolvedValueOnce({ accounts: [account("a")], chain: ["a"] });
    await store.loadAccounts();
    invoke.mockResolvedValueOnce({
      by_tool_hour: [],
      windows: [{ account_id: "a", window: "seven_day", used: 0.4, used_tokens: 9, resets_at_ms: NOW, source: "estimated" }],
    });
    await store.loadUsage();
    expect(store.getAccounts().accounts[0].windows).toHaveLength(1);
    expect(store.getAccounts().accounts[0].windows[0].source).toBe("estimated");
    expect(store.getAccounts().quota_available).toBe(true);
  });

  it("sends the chain the user dragged", async () => {
    invoke.mockResolvedValue({ accounts: [account("a"), account("b")], chain: ["b", "a"] });
    await store.setChain(["b", "a"]);
    expect(invoke).toHaveBeenCalledWith("llm_accounts_set_chain", { chain: ["b", "a"] });
    expect(store.getAccounts().chain).toEqual(["b", "a"]);
  });

  it("sends the sandbox mode the card confirmed", async () => {
    invoke.mockResolvedValue({
      accounts: [account("a", { sandbox: "write", sandbox_flag: "--sandbox workspace-write" })],
      chain: ["a"],
    });
    await store.setSandbox("a", "write");
    expect(invoke).toHaveBeenCalledWith("llm_accounts_set_sandbox", { id: "a", sandbox: "write" });
    expect(store.getAccounts().accounts[0].sandbox).toBe("write");
    await store.setSandbox("a", "read-only");
    expect(invoke).toHaveBeenLastCalledWith("llm_accounts_set_sandbox", { id: "a", sandbox: "read-only" });
  });

  it("never invokes anything in demo mode", async () => {
    store.startAccountsDemo(NOW);
    await store.loadAccounts();
    await store.loadUsage();
    await store.activateAccount("claude-pessoal");
    expect(invoke).not.toHaveBeenCalled();
    expect(store.getAccounts().accounts.length).toBeGreaterThan(0);
    expect(store.getUsage().by_tool_hour).toHaveLength(7);
    expect(store.getUsage().windows.length).toBeGreaterThan(0);
  });

  it("subscribes to the reroute event and unsubscribes on stop", async () => {
    const stop = await store.startSwitchLog();
    expect(listen).toHaveBeenCalledWith("llm://rerouted", expect.any(Function));
    const cb = listen.mock.calls[0][1] as (ev: unknown) => void;
    cb({ payload: { agent: "coordinator", from: "a", to: "b", why: "ERR_CLI_RATE" } });
    expect(store.getSwitches()[0].to).toBe("b");
    stop();
    expect(unlisten).toHaveBeenCalled();
  });
});

describe("demo fixtures", () => {
  it("are deterministic for the same minute", () => {
    expect(store.fakeSnapshot(NOW)).toEqual(store.fakeSnapshot(NOW + 1_000));
    expect(store.fakeUsage(NOW)).toEqual(store.fakeUsage(NOW + 1_000));
  });

  it("show both quota sources so the label is exercised", () => {
    const sources = store.fakeSnapshot(NOW).accounts.flatMap((a) => a.windows.map((w) => w.source));
    expect(new Set(sources)).toEqual(new Set(["reported", "estimated"]));
  });

  it("show both sandbox modes so the card draws both states", () => {
    const modes = store.fakeSnapshot(NOW).accounts.map((a) => a.sandbox);
    expect(new Set(modes)).toEqual(new Set(["read-only", "write"]));
    expect(store.fakeSnapshot(NOW).accounts.every((a) => a.sandbox_flag.length > 0)).toBe(true);
  });

  it("include a window with no plan ceiling, so that branch is drawn too", () => {
    const windows = store.fakeSnapshot(NOW).accounts.flatMap((a) => a.windows);
    expect(windows.some((w) => w.used === null && w.used_tokens > 0)).toBe(true);
  });
});
