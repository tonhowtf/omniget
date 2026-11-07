import { describe, expect, it } from "vitest";
import { columnsFor, moveIndex, rowRange, scrollToReveal } from "./grid";

describe("grid", () => {
  it("computes columns", () => {
    expect(columnsFor(600, 72, 8)).toBe(7);
    expect(columnsFor(10, 72, 8)).toBe(1);
    expect(columnsFor(0, 72, 8)).toBe(1);
  });

  it("computes the visible row window with overscan", () => {
    expect(rowRange(0, 400, 80, 100, 2)).toEqual({ first: 0, last: 7 });
    expect(rowRange(800, 400, 80, 100, 2)).toEqual({ first: 8, last: 17 });
    expect(rowRange(0, 400, 80, 3, 2)).toEqual({ first: 0, last: 2 });
    expect(rowRange(0, 400, 80, 0)).toEqual({ first: 0, last: -1 });
  });

  it("moves with the keyboard", () => {
    expect(moveIndex(0, "ArrowRight", 5, 12)).toBe(1);
    expect(moveIndex(11, "ArrowRight", 5, 12)).toBe(11);
    expect(moveIndex(3, "ArrowDown", 5, 12)).toBe(8);
    expect(moveIndex(8, "ArrowDown", 5, 12)).toBe(8);
    expect(moveIndex(2, "ArrowUp", 5, 12)).toBe(2);
    expect(moveIndex(4, "End", 5, 12)).toBe(11);
    expect(moveIndex(4, "Home", 5, 12)).toBe(0);
    expect(moveIndex(0, "PageDown", 5, 100, 4)).toBe(20);
    expect(moveIndex(0, "ArrowDown", 5, 0)).toBe(-1);
  });

  it("scrolls to reveal the active cell", () => {
    expect(scrollToReveal(0, 5, 80, 160, 400)).toBe(0);
    expect(scrollToReveal(50, 5, 80, 0, 400)).toBe(480);
    expect(scrollToReveal(10, 5, 80, 0, 400)).toBe(0);
  });
});
