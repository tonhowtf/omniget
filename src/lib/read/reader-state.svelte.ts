export const reader = $state({
  kind: "" as "" | "ebook" | "comic",
  title: "",
  location: "",
  progress: 0,
  chapter: "",
  selection: "",
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
