//! Ad and tracker blocking for the Global Downloader browser, applied per tab
//! with `Network.setBlockedURLs`. The list is short on purpose: the large ad
//! and tracking networks, never a login, captcha, CDN or player host, so
//! blocking never breaks a sign-in. A person can switch it off.

const HOSTS: &[&str] = &[
    // Ad networks
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "adservice.google.com",
    "pagead2.googlesyndication.com",
    "amazon-adsystem.com",
    "adnxs.com",
    "adsrvr.org",
    "criteo.com",
    "criteo.net",
    "taboola.com",
    "outbrain.com",
    "pubmatic.com",
    "rubiconproject.com",
    "openx.net",
    "casalemedia.com",
    "smartadserver.com",
    "moatads.com",
    "adform.net",
    "yieldmo.com",
    "sharethrough.com",
    "media.net",
    "bidswitch.net",
    "3lift.com",
    "teads.tv",
    "zemanta.com",
    "revcontent.com",
    "mgid.com",
    "popads.net",
    "propellerads.com",
    "exoclick.com",
    "adsterra.com",
    // Trackers and analytics
    "google-analytics.com",
    "googletagmanager.com",
    "analytics.google.com",
    "connect.facebook.net",
    "facebook.com/tr",
    "hotjar.com",
    "hotjar.io",
    "fullstory.com",
    "mouseflow.com",
    "clarity.ms",
    "scorecardresearch.com",
    "quantserve.com",
    "quantcount.com",
    "chartbeat.com",
    "chartbeat.net",
    "newrelic.com",
    "nr-data.net",
    "segment.io",
    "segment.com/analytics",
    "mixpanel.com",
    "amplitude.com",
    "heap.io",
    "heapanalytics.com",
    "branch.io",
    "adjust.com",
    "appsflyer.com",
    "kochava.com",
    "bat.bing.com",
    "ads.linkedin.com",
    "px.ads.linkedin.com",
    "snap.licdn.com",
    "analytics.tiktok.com",
    "ads.tiktok.com",
    "analytics.twitter.com",
    "static.ads-twitter.com",
    "ct.pinterest.com",
    "sc-static.net",
    "tr.snapchat.com",
    "yandex.ru/metrika",
    "mc.yandex.ru",
    "hs-analytics.net",
    "hs-banner.com",
    "optimizely.com",
    "crazyegg.com",
    "inspectlet.com",
    "luckyorange.com",
    "luckyorange.net",
    "matomo.cloud",
    "bugsnag.com",
];

/// `Network.setBlockedURLs` patterns: every scheme and subdomain of each host.
pub fn patterns() -> Vec<String> {
    let mut out = Vec::with_capacity(HOSTS.len() * 2);
    for h in HOSTS {
        if h.contains('/') {
            out.push(format!("*://*.{h}*"));
            out.push(format!("*://{h}*"));
        } else {
            out.push(format!("*://*.{h}/*"));
            out.push(format!("*://{h}/*"));
        }
    }
    out
}

pub fn host_count() -> usize {
    HOSTS.len()
}
