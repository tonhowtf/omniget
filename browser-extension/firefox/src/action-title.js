const ACTION_TITLE_MESSAGES = Object.freeze({
  action_title_supported: "Send this media page to OmniGet",
  action_title_unsupported: "No supported media detected on this page",
});

function defaultGetMessage(name) {
  if (typeof chrome === "undefined" || !chrome.i18n?.getMessage) {
    return "";
  }
  return chrome.i18n.getMessage(name);
}

export function resolveActionTitle(supported, getMessage = defaultGetMessage) {
  const key = supported ? "action_title_supported" : "action_title_unsupported";
  return getMessage(key) || ACTION_TITLE_MESSAGES[key];
}
