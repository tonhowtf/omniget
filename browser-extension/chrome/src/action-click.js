const DEFAULT_LAUNCH_FAILED_CODE = "LAUNCH_FAILED";
const PROTOCOL_VERSION = 1;

export async function handleSupportedActionClick({
  tabId,
  url,
  platform,
  getCookies,
  sendNativeMessage,
  clearBadge = async () => {},
  showSuccessBadge = async () => {},
  openErrorPage,
  mapChromeErrorCode,
  openSchemeUrl = async () => ({ ok: false, reason: "not-wired" }),
}) {
  if (tabId !== undefined && tabId !== null) {
    await clearBadge(tabId);
  }

  let cookies = null;
  try {
    if (getCookies && platform) {
      cookies = await getCookies(platform);
    }
  } catch {
    // Cookie extraction failed — continue without cookies
  }

  try {
    const message = { type: "enqueue", url, protocolVersion: PROTOCOL_VERSION };
    if (cookies && cookies.length > 0) {
      message.cookies = cookies;
    }

    const response = await sendNativeMessage(message);

    if (!response?.ok) {
      await openErrorPage({
        code: response?.code ?? DEFAULT_LAUNCH_FAILED_CODE,
        message: response?.message ?? "",
        url,
      });
      return false;
    }

    if (tabId !== undefined && tabId !== null) {
      await showSuccessBadge(tabId);
    }

    return true;
  } catch (error) {
    const schemeResult = await openSchemeUrl(url);
    if (schemeResult?.ok) {
      if (tabId !== undefined && tabId !== null) {
        await showSuccessBadge(tabId);
      }
      return true;
    }

    await openErrorPage({
      code: mapChromeErrorCode(error),
      message: error instanceof Error ? error.message : String(error),
      url,
    });
    return false;
  }
}
