// What the open reader shows right now; read by ReadHost for agents and by
// the reader panels.

export const reader = $state({
  kind: "" as "" | "ebook" | "comic",
  title: "",
  location: "",
  progress: 0,
  chapter: "",
  selection: "",
  /** Where opening a book got to (fetch, parse, render, ready) or the error. */
  stage: "",
});

export function resetReader() {
  reader.kind = "";
  reader.title = "";
  reader.location = "";
  reader.progress = 0;
  reader.chapter = "";
  reader.selection = "";
  reader.stage = "";
}
