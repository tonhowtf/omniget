import { describe, expect, it } from "vitest";
import { translateBackendError } from "./error-translate";
import { classifyError } from "./home/friendly-error";

const t = (key: string) => key;
const withValues = (key: string, vars: Record<string, string | number>) => `${key}:${JSON.stringify(vars)}`;

describe("translateBackendError", () => {
  it("names the failing step and what to change", () => {
    const out = translateBackendError(
      "Post-processing failed at the merge step (open_input): Error opening input files: Result too large",
      t,
      withValues,
    );
    expect(out).toContain("errors.postprocess_step:");
    expect(out).toContain("errors.postprocess_step_name.merge");
    expect(out).toContain("errors.postprocess_advice.open_input");
    expect(out).toContain("Result too large");
  });

  it("finds the step message inside the wider queue error", () => {
    expect(
      translateBackendError("x (Post-processing failed at the embed_thumbnail step (generic): boom)", t),
    ).toContain("errors.postprocess_step_name.embed_thumbnail");
  });

  it("reports the line of an invalid archive file", () => {
    expect(translateBackendError("ArchiveInvalidLine|4|hello there you", t, withValues)).toContain('"line":"4"');
  });

  it("translates the Facebook login message", () => {
    expect(
      translateBackendError(
        "Facebook did not show this post without a login. Import cookies for this site in Settings → Cookies, then retry.",
        t,
      ),
    ).toBe("errors.facebook_login");
  });
});

describe("classifyError", () => {
  it("treats login wording from Facebook and Instagram as a login problem", () => {
    expect(classifyError("You must be logged in to view this")).toBe("login");
    expect(classifyError("This video is only available for registered users")).toBe("login");
    expect(classifyError("Facebook did not show this post without a login.")).toBe("login");
  });
});
