// Light or dark, read from the app theme (`color-scheme` on <html>, set by the
// theme CSS) and kept up to date when the theme changes.
export const look = $state({ scheme: "dark" as "dark" | "light" });

let started = false;

function read() {
  const cs = getComputedStyle(document.documentElement).colorScheme || "";
  if (/dark/.test(cs) && !/light/.test(cs)) look.scheme = "dark";
  else if (/light/.test(cs) && !/dark/.test(cs)) look.scheme = "light";
  else look.scheme = matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function watchScheme() {
  if (started || typeof document === "undefined") return;
  started = true;
  read();
  new MutationObserver(() => read()).observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme", "class", "style"],
  });
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", read);
}
