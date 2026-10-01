const HOSTS: &[&str] = &[
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

const AD_MEDIA_HOSTS: &[&str] = &["2mdn.net", "imasdk.googleapis.com"];

const AD_MEDIA_SEGMENTS: &[&str] = &[
    "nicoad",
    "instream",
    "preroll",
    "midroll",
    "vast",
    "adcreative",
];

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
