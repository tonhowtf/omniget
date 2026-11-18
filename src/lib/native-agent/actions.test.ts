import { describe, expect, it } from "vitest";
import { agentActions, allowedActions } from "./actions";

const button = () => ({ matches: (sel: string) => sel.includes("button") }) as unknown as HTMLElement;

describe("agent actions", () => {
  it("lets an unguarded agent click an ordinary button", () => {
    expect(allowedActions(button(), true)).toContain("click");
  });

  it("an approval button marked with no actions cannot be clicked even unguarded", () => {
    const approve = button();
    const mark = agentActions(approve, []);
    expect(allowedActions(approve, true)).toEqual([]);
    expect(allowedActions(approve, false)).toEqual([]);
    mark.destroy();
    expect(allowedActions(approve, true)).toContain("click");
  });
});
