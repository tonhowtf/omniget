import { describe, expect, it } from "vitest";
import { hardcodedStrings, missingKeys } from "../../../scripts/i18n-scan.mjs";

describe("i18n build check", () => {
  it("fails on a key the code uses that en.json does not have", () => {
    const keys = new Set(["a.b"]);
    expect(missingKeys('{$t("a.b")} {$t("a.c")}', keys)).toEqual(["a.c"]);
  });

  it("ignores keys built at runtime", () => {
    expect(missingKeys("$t(`a.${b}`)", new Set())).toEqual([]);
  });

  it("finds prose written straight into a template", () => {
    expect(hardcodedStrings('<button>Selecionar tudo</button><input placeholder="Type here" />')).toEqual(["Selecionar tudo", "Type here"]);
  });

  it("leaves scripts, code samples and one-word brand names alone", () => {
    expect(hardcodedStrings('<script>const a = "<b>Not this one</b>"</script><span>Telegram</span><code>Sample code here</code>')).toEqual([]);
  });
});
