export function summarizeCookies(cookies) {
  if (!Array.isArray(cookies) || cookies.length === 0) return null;

  const counts = new Map();
  for (const cookie of cookies) {
    const rawDomain = cookie && typeof cookie.domain === "string" ? cookie.domain : "";
    const domain = rawDomain.replace(/^\./, "").toLowerCase();
    if (!domain) continue;
    counts.set(domain, (counts.get(domain) || 0) + 1);
  }

  if (counts.size === 0) return null;

  let primaryDomain = null;
  let primaryCount = -1;
  for (const [domain, count] of counts) {
    if (count > primaryCount || (count === primaryCount && domain < primaryDomain)) {
      primaryCount = count;
      primaryDomain = domain;
    }
  }

  return {
    count: cookies.length,
    primaryDomain,
    domainCount: counts.size,
  };
}

export function formatCookieSummary(summary) {
  if (!summary) return "";
  const { count, primaryDomain, domainCount } = summary;
  if (!count || count <= 0) return "";
  if (domainCount <= 1 && primaryDomain) {
    return `${count} cookies from ${primaryDomain} sent`;
  }
  if (primaryDomain) {
    return `${count} cookies from ${primaryDomain} and ${domainCount - 1} other domain${domainCount - 1 === 1 ? "" : "s"} sent`;
  }
  return `${count} cookies sent`;
}
