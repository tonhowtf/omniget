import { describe, expect, it } from "vitest";
import screens from "./catalog.json";
import { SUPERPOWERS } from "$lib/superpowers/catalog";

describe("native agent screen catalog", () => {
  it("lets agents navigate to every superpower", () => {
    const routes = new Set(screens.map((screen) => screen.route));
    const missing = SUPERPOWERS.map((power) => power.href).filter((href) => !routes.has(href));
    expect(missing).toEqual([]);
  });
});
