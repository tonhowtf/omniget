let draftUrl = $state("");

export function getOmniboxDraftUrl(): string {
  return draftUrl;
}

export function setOmniboxDraftUrl(url: string) {
  draftUrl = url;
}

export function clearOmniboxDraftUrl() {
  draftUrl = "";
}
