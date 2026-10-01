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
    // Outstream video ads and the header bidders seen next to them
    // (w3schools.com, 2026-10): the ad plays as HLS and lands in the
    // sniffer as if it were the page's media.
    "viously.com",
    "springserve.com",
    "spotxchange.com",
    "spotx.tv",
    "innovid.com",
    "tremorhub.com",
    "aniview.com",
    "vidazoo.com",
    "primis.tech",
    "sparteo.com",
    "4dex.io",
    "bricks-co.com",
    "e-planning.net",
    "a-mx.com",
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

/// Hosts that only serve ad creatives. They are not blocked on the network:
/// players load the ad SDK from the same hosts (Google IMA's `client.js` is
/// on 2mdn.net) and break without it. Their media is only kept out of the
/// sniffed list.
const AD_MEDIA_HOSTS: &[&str] = &["2mdn.net", "imasdk.googleapis.com"];

/// Path segments that mark an ad creative on a site's own CDN, where the host
/// cannot be blocked (Niconico serves its pre-rolls from
/// `cdn.nimg.jp/nicoad/instream/video/…`, next to the real thumbnails).
/// Whole segments only, so `/videos/instreamer/` stays.
const AD_MEDIA_SEGMENTS: &[&str] = &[
    "nicoad",
    "instream",
    "preroll",
    "midroll",
    "vast",
    "adcreative",
];

/// A sniffed response that is an ad, not the page's media: blocked hosts
/// (when blocking is off they still load), ad-only hosts and ad paths.
pub fn is_ad_media(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let on = |h: &&str| !h.contains('/') && super::host_matches(&host, h);
    if HOSTS.iter().any(on) || AD_MEDIA_HOSTS.iter().any(on) {
        return true;
    }
    u.path_segments().is_some_and(|mut segs| {
        segs.any(|seg| {
            AD_MEDIA_SEGMENTS
                .iter()
                .any(|a| seg.eq_ignore_ascii_case(a))
        })
    })
}
