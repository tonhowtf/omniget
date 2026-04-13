import { describe, it, expect } from "vitest";
import { translateBackendError } from "./error-translate";

const mockT = (key: string) => `T[${key}]`;

describe("translateBackendError", () => {
  it("returns common.unknown_error for empty input", () => {
    expect(translateBackendError("", mockT)).toBe("T[common.unknown_error]");
  });

  it("maps known backend strings exactly", () => {
    expect(
      translateBackendError("Video unavailable or removed.", mockT),
    ).toBe("T[errors.video_unavailable]");
    expect(translateBackendError("This video is private.", mockT)).toBe(
      "T[errors.video_private]",
    );
    expect(
      translateBackendError(
        "Server returned error 429 (too many requests). Try again later.",
        mockT,
      ),
    ).toBe("T[errors.rate_limited]");
  });

  it("strips the 'Failed to get formats:' prefix before matching", () => {
    expect(
      translateBackendError(
        "Failed to get formats: Video unavailable or removed.",
        mockT,
      ),
    ).toBe("T[errors.video_unavailable]");
  });

  it("falls back to heuristic cookie_database match", () => {
    expect(
      translateBackendError("Could not copy Chrome cookie database", mockT),
    ).toBe("T[errors.cookie_database]");
  });

  it("falls back to heuristic size_mismatch match", () => {
    expect(
      translateBackendError("Got a size mismatch on chunk 3", mockT),
    ).toBe("T[errors.size_mismatch]");
  });

  it("falls back to heuristic disk_full match", () => {
    expect(translateBackendError("Disk full on volume D:", mockT)).toBe(
      "T[errors.disk_full]",
    );
    expect(
      translateBackendError("write error when flushing buffer", mockT),
    ).toBe("T[errors.disk_full]");
  });

  it("falls back to heuristic tiktok_blocked match", () => {
    expect(
      translateBackendError("TikTok is blocking requests from this IP", mockT),
    ).toBe("T[errors.tiktok_blocked]");
  });

  it("returns the original stripped message when no mapping matches", () => {
    expect(translateBackendError("some unknown error detail", mockT)).toBe(
      "some unknown error detail",
    );
  });

  it("preserves punctuation in unmapped messages after strip", () => {
    expect(
      translateBackendError(
        "Failed to get formats: weird backend msg: x=1",
        mockT,
      ),
    ).toBe("weird backend msg: x=1");
  });
});
