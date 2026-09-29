import { describe, expect, it } from "vitest";
import { errorCode, hunkAt, noteFromComment, noteRange, viewerFiles, type HunksView } from "./job-review";

const view: HunksView = {
  job_id: "j1",
  state: "review",
  complete: true,
  clipped: false,
  mode: "worktree",
  actionable: true,
  hunks: [
    { id: "a.txt:11111111", path: "a.txt", index: 0, header: "@@ -1 +1 @@", old_start: 1, old_lines: 1, new_start: 1, new_lines: 1, additions: 1, deletions: 1, patch: "p0", complete: true },
    { id: "a.txt:22222222", path: "a.txt", index: 1, header: "@@ -9 +9 @@", old_start: 9, old_lines: 1, new_start: 9, new_lines: 1, additions: 1, deletions: 1, patch: "p1", complete: true },
  ],
  files: [{ path: "a.txt", patch: "diff --git a/a.txt b/a.txt\n" }],
};

describe("job review", () => {
  it("maps the job's files to the viewer with each file's own patch", () => {
    const files = viewerFiles(
      [
        { path: "a.txt", status: "M", additions: 2, deletions: 2, binary: false },
        { path: "img.png", status: "A", additions: null, deletions: null, binary: true },
        { path: "big.txt", status: "A", additions: 9000, deletions: 0, binary: false },
      ],
      view,
    );
    expect(files[0]).toMatchObject({ status: "modified", patch: "diff --git a/a.txt b/a.txt\n", truncated: false });
    expect(files[1]).toMatchObject({ status: "binary", truncated: false });
    // Past the diff's size limit: no patch, said so.
    expect(files[2]).toMatchObject({ status: "added", patch: "", truncated: true });
  });

  it("finds the hunk the viewer shows by file and position", () => {
    expect(hunkAt(view, "a.txt", 1)?.id).toBe("a.txt:22222222");
    expect(hunkAt(view, "a.txt", 2)).toBeNull();
    expect(hunkAt(view, "b.txt", 0)).toBeNull();
  });

  it("turns a selection into a note on new lines, or on the file for removed lines", () => {
    const base = { path: "a.txt", rangeLabel: "+3 to +5", diff: "", language: "text" };
    expect(noteFromComment({ ...base, side: "new", start: 3, end: 5, text: " rename x " })).toEqual({
      path: "a.txt",
      start_line: 3,
      end_line: 5,
      body: "rename x",
    });
    const old = noteFromComment({ ...base, side: "old", start: 4, end: 4, rangeLabel: "-4", text: "why?" });
    expect(old).toEqual({ path: "a.txt", start_line: null, end_line: null, body: "(on removed lines -4) why?" });
  });

  it("labels a note's lines and reads error codes", () => {
    expect(noteRange({ start_line: null, end_line: null })).toBe("file");
    expect(noteRange({ start_line: 4, end_line: 4 })).toBe("L4");
    expect(noteRange({ start_line: 4, end_line: 9 })).toBe("L4–9");
    expect(errorCode("ERR_HUNK_STALE: refresh")).toBe("ERR_HUNK_STALE");
    expect(errorCode("oops")).toBe("");
  });
});
