import { describe, expect, it } from "vitest";
import { genres } from "./library";
import type { Track } from "./types";

const track = (genre: string) => ({ genre }) as Track;

describe("genres", () => {
  it("gives every genre its own name, even when one track lists several", () => {
    const list = genres([track("Rock; Pop"), track("rock"), track("Pop/Jazz")]);
    const names = list.map((g) => g.name);
    expect(new Set(names).size).toBe(names.length);
    expect(names.sort()).toEqual(["Jazz", "Pop", "Rock"]);
  });

  it("counts a track once when it repeats a genre", () => {
    const list = genres([track("Rock, rock; ROCK")]);
    expect(list.length).toBe(1);
    expect(list[0].tracks.length).toBe(1);
  });

  it("counts a track under each of its genres", () => {
    const list = genres([track("Rock; Pop"), track("rock")]);
    expect(list.find((g) => g.name === "Rock")?.tracks.length).toBe(2);
    expect(list.find((g) => g.name === "Pop")?.tracks.length).toBe(1);
  });
});
