const DEFAULT_LAUNCH_FAILED_CODE = "LAUNCH_FAILED";

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
    const message = { type: "enqueue", url };
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
    await openErrorPage({
      code: mapChromeErrorCode(error),
      message: error instanceof Error ? error.message : String(error),
      url,
    });
    return false;
  }
}
