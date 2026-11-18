import { describe, expect, it } from "vitest";
import { errDetail, errKey, NET, tool } from "./api";

describe("social api helpers", () => {
  it("builds tool names for the six networks", () => {
    expect(tool(NET.forums, "feed")).toBe("rd_feed");
    expect(tool(NET.pinboard, "user")).toBe("pin_user");
    expect(tool(NET.codehub, "search")).toBe("gh_search");
    expect(tool(NET.shortpost, "feed")).toBe("x_feed");
  });

  it("reads the message of a JSON error and keeps plain ones", () => {
    expect(errDetail('TOKEN_REQUIRED: {"message":"Code search needs a Code Hub token","retryable":false}')).toBe("Code search needs a Code Hub token");
    expect(errDetail("RATE_LIMITED: wait a few minutes")).toBe("wait a few minutes");
    expect(errDetail("REJECTED: {broken")).toBe("{broken");
    expect(errKey("RATE_LIMITED: x")).toBe("rate_limited");
    expect(errKey("TOKEN_REQUIRED: {}")).toBe("unknown");
  });
});
