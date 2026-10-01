// Curated set of long-lived public BitTorrent trackers. Many magnet links
// carry few or dead trackers; injecting these stable open trackers materially
// improves peer discovery so downloads actually start. Kept as a static list
// (no runtime fetch) to avoid an external dependency, scheduler, and the
// SSRF/privacy surface of pulling a remote list at runtime.
const PUBLIC_TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.tracker.cl:1337/announce",
    "udp://tracker.openbittorrent.com:6969/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://tracker.dler.org:6969/announce",
    "udp://explodie.org:6969/announce",
    "https://tracker.tamersunion.org:443/announce",
    "udp://opentracker.io:6969/announce",
];

pub fn extra_trackers() -> Vec<String> {
    PUBLIC_TRACKERS.iter().map(|s| s.to_string()).collect()
}
