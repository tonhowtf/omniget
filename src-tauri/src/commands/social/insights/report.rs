use std::fmt::Write;

use serde_json::Value;

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const METRICS: [(&str, &str); 5] = [
    ("likes", "Likes"),
    ("replies", "Replies"),
    ("reposts", "Reposts"),
    ("saves", "Saves"),
    ("views", "Views"),
];
pub const TOP_POSTS: usize = 10;
pub const TOP_HASHTAGS: usize = 15;

const CSS: &str = "body{font:15px/1.5 system-ui,-apple-system,Segoe UI,sans-serif;margin:0;background:#f6f6f4;color:#1c1c1a}\
main{max-width:860px;margin:0 auto;padding:32px 20px 48px}\
h1{font-size:26px;margin:0 0 4px}h2{font-size:18px;margin:36px 0 10px;padding-top:18px;border-top:1px solid #d9d9d4}\
p{margin:6px 0}.muted{color:#6b6b66}.none{color:#6b6b66;font-style:italic}\
.cards{display:flex;flex-wrap:wrap;gap:10px;margin:14px 0}.card{background:#fff;border:1px solid #d9d9d4;border-radius:8px;padding:10px 14px;min-width:120px}\
.card b{display:block;font-size:20px}.card span{color:#6b6b66;font-size:13px}\
table{border-collapse:collapse;width:100%;background:#fff;border:1px solid #d9d9d4;border-radius:8px}\
th,td{text-align:left;padding:6px 10px;border-bottom:1px solid #ecece8;vertical-align:top}th{font-size:13px;color:#6b6b66;font-weight:600}\
td.n,th.n{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}\
svg{max-width:100%;height:auto;background:#fff;border:1px solid #d9d9d4;border-radius:8px}\
a{color:#1f5fbf}footer{margin-top:40px;color:#6b6b66;font-size:13px}\
@media (prefers-color-scheme:dark){body{background:#161615;color:#ecece8}.card,table,svg{background:#1f1f1d;border-color:#3a3a36}\
h2{border-color:#3a3a36}th,td{border-color:#2c2c29}.muted,.none,.card span,th,footer{color:#a3a39c}a{color:#8ab4f8}svg text{fill:#a3a39c}}";

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn md_cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\r' | '\t' => out.push(' '),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '|' | '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '!' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => {
                let r = (f * 100.0).round() / 100.0;
                if r.fract() == 0.0 {
                    format!("{r:.0}")
                } else {
                    r.to_string()
                }
            }
            _ => n.to_string(),
        },
        _ => "–".into(),
    }
}

fn safe_url(v: &Value) -> Option<&str> {
    v.as_str().filter(|u| {
        (u.starts_with("https://") || u.starts_with("http://"))
            && !u.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

fn day_of(ts: &Value) -> String {
    ts.as_i64()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "–".into())
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn reason<'a>(d: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    d[key].as_str().unwrap_or(fallback)
}

fn trend_svg(days: &[Value]) -> Option<String> {
    let points: Vec<(String, f64)> = days
        .iter()
        .filter_map(|d| Some((d["day"].as_str()?.to_string(), d["followers"].as_f64()?)))
        .collect();
    if points.len() < 2 {
        return None;
    }
    let (w, h, pad) = (640.0, 180.0, 28.0);
    let min = points.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let max = points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let span = (max - min).max(1.0);
    let step = (w - 2.0 * pad) / (points.len() - 1) as f64;
    let coords: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            format!(
                "{:.1},{:.1}",
                pad + step * i as f64,
                h - pad - (v - min) / span * (h - 2.0 * pad)
            )
        })
        .collect();
    Some(format!(
        "<svg viewBox=\"0 0 {w} {h}\" role=\"img\" aria-label=\"Followers per day\"><polyline fill=\"none\" stroke=\"#1f5fbf\" stroke-width=\"2\" points=\"{}\"/><text x=\"{pad}\" y=\"16\" font-size=\"12\" fill=\"#6b6b66\">{max:.0}</text><text x=\"{pad}\" y=\"{}\" font-size=\"12\" fill=\"#6b6b66\">{min:.0}</text><text x=\"{pad}\" y=\"{}\" font-size=\"12\" fill=\"#6b6b66\">{}</text><text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"#6b6b66\" text-anchor=\"end\">{}</text></svg>",
        coords.join(" "),
        h - pad + 4.0,
        h - 6.0,
        esc(&points[0].0),
        w - pad,
        h - 6.0,
        esc(&points[points.len() - 1].0),
    ))
}

fn changes_svg(days: &[Value]) -> Option<String> {
    let bars: Vec<(f64, f64)> = days
        .iter()
        .map(|d| {
            (
                d["gained"].as_f64().unwrap_or(0.0),
                d["lost"].as_f64().unwrap_or(0.0),
            )
        })
        .collect();
    let top = bars.iter().map(|b| b.0.max(b.1)).fold(0.0, f64::max);
    if top == 0.0 {
        return None;
    }
    let (w, h, pad) = (640.0, 140.0, 20.0);
    let mid = h / 2.0;
    let slot = (w - 2.0 * pad) / bars.len() as f64;
    let bar = (slot * 0.7).clamp(1.0, 24.0);
    let mut out = format!("<svg viewBox=\"0 0 {w} {h}\" role=\"img\" aria-label=\"Followers gained and lost per day\"><line x1=\"{pad}\" x2=\"{}\" y1=\"{mid}\" y2=\"{mid}\" stroke=\"#b5b5ae\"/>", w - pad);
    for (i, (gained, lost)) in bars.iter().enumerate() {
        let x = pad + slot * i as f64 + (slot - bar) / 2.0;
        let up = gained / top * (mid - pad);
        let down = lost / top * (mid - pad);
        let _ = write!(out, "<rect x=\"{x:.1}\" y=\"{:.1}\" width=\"{bar:.1}\" height=\"{up:.1}\" fill=\"#2e8b57\"/><rect x=\"{x:.1}\" y=\"{mid}\" width=\"{bar:.1}\" height=\"{down:.1}\" fill=\"#c0392b\"/>", mid - up);
    }
    out.push_str("</svg>");
    Some(out)
}

fn times_svg(times: &Value) -> Option<String> {
    let cells = list(&times["cells"]);
    let top = cells
        .iter()
        .filter_map(|c| c["mean"].as_f64())
        .fold(0.0, f64::max);
    if cells.is_empty() {
        return None;
    }
    let (cw, ch, left, head) = (24.0, 20.0, 40.0, 18.0);
    let (w, h) = (left + 24.0 * cw + 4.0, head + 7.0 * ch + 4.0);
    let mut out = format!(
        "<svg viewBox=\"0 0 {w} {h}\" role=\"img\" aria-label=\"Mean engagement by weekday and hour\">"
    );
    for (i, day) in DAYS.iter().enumerate() {
        let _ = write!(
            out,
            "<text x=\"4\" y=\"{:.0}\" font-size=\"11\" fill=\"#6b6b66\">{day}</text>",
            head + ch * i as f64 + 14.0
        );
    }
    for hour in [0, 6, 12, 18, 23] {
        let _ = write!(
            out,
            "<text x=\"{:.0}\" y=\"12\" font-size=\"11\" fill=\"#6b6b66\">{hour:02}</text>",
            left + cw * f64::from(hour) + 4.0
        );
    }
    for c in cells {
        let (Some(day), Some(hour)) = (c["weekday"].as_u64(), c["hour"].as_u64()) else {
            continue;
        };
        if day > 6 || hour > 23 {
            continue;
        }
        let mean = c["mean"].as_f64().unwrap_or(0.0);
        let strength = if top > 0.0 {
            0.15 + 0.85 * mean / top
        } else {
            0.15
        };
        let dash = if c["thin"] == true {
            " stroke=\"#1f5fbf\" stroke-dasharray=\"3 2\" fill-opacity=\"0.08\""
        } else {
            ""
        };
        let _ = write!(
            out,
            "<rect x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\" rx=\"3\" fill=\"#1f5fbf\" fill-opacity=\"{strength:.2}\"{dash}><title>{} {hour:02}:00, {} posts, mean {}</title></rect>",
            left + cw * hour as f64 + 1.0,
            head + ch * day as f64 + 1.0,
            cw - 2.0,
            ch - 2.0,
            DAYS[day as usize],
            text(&c["posts"]),
            text(&c["mean"]),
        );
    }
    out.push_str("</svg>");
    Some(out)
}

fn none(out: &mut String, why: &str) {
    let _ = write!(out, "<p class=\"none\">No data: {}</p>", esc(why));
}

fn card(out: &mut String, value: &Value, label: &str) {
    if !value.is_null() {
        let _ = write!(
            out,
            "<div class=\"card\"><b>{}</b><span>{label}</span></div>",
            esc(&text(value))
        );
    }
}

pub fn html(d: &Value) -> String {
    let handle = esc(d["handle"].as_str().unwrap_or_default());
    let platform = esc(d["platform"].as_str().unwrap_or_default());
    let mut o = String::new();
    let _ = write!(o, "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{platform} report for @{handle}</title><style>{CSS}</style></head><body><main>");
    let _ = write!(o, "<h1>{platform} report for @{handle}</h1><p class=\"muted\">Generated {} on this computer, from what a signed-in browser session can see. Numbers the network hides are left out, never estimated. Text from the network is shown as written by other people.</p>", esc(d["generated"].as_str().unwrap_or_default()));

    o.push_str("<h2>Account</h2>");
    let profile = &d["profile"];
    let summary = &d["summary"];
    if profile.is_null() && summary.is_null() {
        none(
            &mut o,
            reason(d, "posts_reason", "the account could not be read."),
        );
    } else {
        if let Some(name) = profile["name"].as_str().filter(|n| !n.is_empty()) {
            let _ = write!(o, "<p>{}</p>", esc(name));
        }
        o.push_str("<div class=\"cards\">");
        card(&mut o, &profile["followers"], "followers");
        card(&mut o, &profile["following"], "following");
        card(&mut o, &profile["posts"], "posts in total");
        card(&mut o, &summary["analyzed"], "posts analyzed");
        card(
            &mut o,
            &summary["engagement"]["median"],
            "median engagement",
        );
        card(&mut o, &summary["engagement"]["mean"], "mean engagement");
        o.push_str("</div>");
        if !summary.is_null() {
            let _ = write!(o, "<p class=\"muted\">Sample from {} to {}; confidence {}. Engagement = likes + replies + reposts + saves, each where the network shows it.</p>", day_of(&summary["from"]), day_of(&summary["to"]), esc(&text(&summary["confidence"])));
        }
        if let Some(note) = d["posts_note"].as_str() {
            let _ = write!(o, "<p class=\"muted\">{}</p>", esc(note));
        }
    }

    o.push_str("<h2>Followers</h2>");
    let days = list(&d["audience"]["days"]);
    if days.is_empty() {
        none(
            &mut o,
            reason(
                d,
                "audience_reason",
                "no audience snapshot is stored yet; sync the audience twice to see changes.",
            ),
        );
    } else {
        let gained: i64 = days.iter().filter_map(|x| x["gained"].as_i64()).sum();
        let lost: i64 = days.iter().filter_map(|x| x["lost"].as_i64()).sum();
        let _ = write!(
            o,
            "<p>{} snapshot day{} from {} to {}: <b>{gained}</b> gained, <b>{lost}</b> lost.</p>",
            days.len(),
            if days.len() == 1 { "" } else { "s" },
            esc(&text(&days[0]["day"])),
            esc(&text(&days[days.len() - 1]["day"]))
        );
        match trend_svg(days) {
            Some(svg) => o.push_str(&svg),
            None => o.push_str(
                "<p class=\"none\">The trend line needs two days with a follower count.</p>",
            ),
        }
        match changes_svg(days) {
            Some(svg) => {
                o.push_str("<p class=\"muted\">Gained (up) and lost (down) per sync day.</p>");
                o.push_str(&svg);
            }
            None => o.push_str(
                "<p class=\"none\">No follower was gained or lost between the stored syncs.</p>",
            ),
        }
        o.push_str("<p class=\"muted\">History starts at the first sync; a change is dated by the sync that found it.</p>");
    }

    o.push_str("<h2>Post performance</h2>");
    if summary["analyzed"].as_u64().unwrap_or(0) == 0 {
        none(&mut o, reason(d, "posts_reason", "no post could be read."));
    } else {
        o.push_str("<table><thead><tr><th>Metric</th><th class=\"n\">Posts</th><th class=\"n\">Total</th><th class=\"n\">Mean</th><th class=\"n\">Median</th></tr></thead><tbody>");
        for (key, label) in METRICS {
            let m = &summary["metrics"][key];
            if m["available"] == true {
                let _ = write!(o, "<tr><td>{label}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>", text(&m["posts"]), text(&m["total"]), text(&m["mean"]), text(&m["median"]));
            } else {
                let _ = write!(o, "<tr><td>{label}</td><td colspan=\"4\" class=\"none\">not visible to a signed-in session</td></tr>");
            }
        }
        o.push_str("</tbody></table>");
    }

    o.push_str("<h2>Best times</h2>");
    match times_svg(&d["times"]) {
        Some(svg) => {
            o.push_str(&svg);
            let best = &d["times"]["best"];
            if let (Some(day), Some(hour)) = (
                best["weekday"].as_u64().filter(|x| *x < 7),
                best["hour"].as_u64(),
            ) {
                let _ = write!(
                    o,
                    "<p>Best: <b>{} {hour:02}:00</b>, mean engagement {} over {} posts.</p>",
                    DAYS[day as usize],
                    text(&best["mean"]),
                    text(&best["posts"])
                );
            }
            let _ = write!(o, "<p class=\"muted\">Time zone {}. Darker = higher mean engagement; dashed = fewer than {} posts in that hour.</p>", esc(&text(&d["times"]["timezone"])), text(&d["times"]["thin_below"]));
        }
        None => none(&mut o, "no dated post in the sample."),
    }

    o.push_str("<h2>Top posts</h2>");
    let top = list(&d["top"]);
    if top.is_empty() {
        none(&mut o, "no post in the sample.");
    } else {
        o.push_str("<table><thead><tr><th>Date</th><th>Post</th><th class=\"n\">Engagement</th><th class=\"n\">Likes</th><th class=\"n\">Replies</th><th class=\"n\">Reposts</th><th class=\"n\">Views</th></tr></thead><tbody>");
        for p in top.iter().take(TOP_POSTS) {
            let body = esc(p["text"]
                .as_str()
                .filter(|t| !t.is_empty())
                .unwrap_or("(no text)"));
            let cell = match safe_url(&p["url"]) {
                Some(url) => format!(
                    "<a href=\"{}\" rel=\"noreferrer noopener\">{body}</a>",
                    esc(url)
                ),
                None => body,
            };
            let _ = write!(o, "<tr><td class=\"n\">{}</td><td>{cell}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>", day_of(&p["created_at"]), text(&p["engagement"]), text(&p["likes"]), text(&p["replies"]), text(&p["reposts"]), text(&p["views"]));
        }
        o.push_str("</tbody></table>");
    }

    o.push_str("<h2>Hashtags</h2>");
    let tags = list(&d["hashtags"]);
    if tags.is_empty() {
        none(&mut o, "no hashtag in the posts of the sample.");
    } else {
        o.push_str("<table><thead><tr><th>Hashtag</th><th class=\"n\">Uses</th><th class=\"n\">Mean engagement</th><th class=\"n\">Median</th></tr></thead><tbody>");
        for t in tags.iter().take(TOP_HASHTAGS) {
            let _ = write!(o, "<tr><td>#{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>", esc(&text(&t["hashtag"])), text(&t["uses"]), text(&t["engagement_mean"]), text(&t["engagement_median"]));
        }
        o.push_str("</tbody></table>");
    }

    o.push_str("<h2>Monitors</h2>");
    let monitors = list(&d["monitors"]);
    if monitors.is_empty() {
        none(
            &mut o,
            reason(
                d,
                "monitors_reason",
                "no monitor is saved for this network.",
            ),
        );
    } else {
        o.push_str("<table><thead><tr><th>Monitor</th><th>Query</th><th class=\"n\">Unread</th><th class=\"n\">Stored</th><th class=\"n\">Last check</th></tr></thead><tbody>");
        for m in monitors {
            let checked = m["last_checked"].as_i64().map(|ms| Value::from(ms / 1000));
            let _ = write!(o, "<tr><td>{}</td><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>", esc(&text(&m["name"])), esc(&text(&m["query"])), text(&m["unread"]), text(&m["total_stored"]), checked.map_or_else(|| "never".into(), |t| day_of(&t)));
        }
        o.push_str("</tbody></table><p class=\"muted\">Monitors collect posts only when a check runs; nothing runs in the background.</p>");
    }

    o.push_str("<footer>Made on this computer. No data left it to build this file, and the file loads nothing from the internet.</footer></main></body></html>");
    o
}

pub fn markdown(d: &Value) -> String {
    let handle = md_cell(d["handle"].as_str().unwrap_or_default());
    let platform = md_cell(d["platform"].as_str().unwrap_or_default());
    let mut o = String::new();
    let _ = writeln!(o, "# {platform} report for @{handle}\n");
    let _ = writeln!(o, "Generated {} on this computer, from what a signed-in browser session can see. Numbers the network hides are left out, never estimated.\n", md_cell(d["generated"].as_str().unwrap_or_default()));
    let profile = &d["profile"];
    let summary = &d["summary"];

    o.push_str("## Account\n\n");
    if profile.is_null() && summary.is_null() {
        let _ = writeln!(
            o,
            "No data: {}\n",
            md_cell(reason(d, "posts_reason", "the account could not be read."))
        );
    } else {
        for (v, label) in [
            (&profile["name"], "Name"),
            (&profile["followers"], "Followers"),
            (&profile["following"], "Following"),
            (&profile["posts"], "Posts in total"),
            (&summary["analyzed"], "Posts analyzed"),
            (&summary["engagement"]["median"], "Median engagement"),
            (&summary["engagement"]["mean"], "Mean engagement"),
            (&summary["confidence"], "Confidence"),
        ] {
            if !v.is_null() && v.as_str() != Some("") {
                let _ = writeln!(o, "- {label}: {}", md_cell(&text(v)));
            }
        }
        if !summary.is_null() {
            let _ = writeln!(
                o,
                "- Sample: {} to {}",
                day_of(&summary["from"]),
                day_of(&summary["to"])
            );
        }
        if let Some(note) = d["posts_note"].as_str() {
            let _ = writeln!(o, "- Note: {}", md_cell(note));
        }
        o.push('\n');
    }

    o.push_str("## Followers\n\n");
    let days = list(&d["audience"]["days"]);
    if days.is_empty() {
        let _ = writeln!(
            o,
            "No data: {}\n",
            md_cell(reason(
                d,
                "audience_reason",
                "no audience snapshot is stored yet; sync the audience twice to see changes."
            ))
        );
    } else {
        o.push_str("| Day | Followers | Following | Gained | Lost |\n|---|---:|---:|---:|---:|\n");
        for x in days {
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} | {} |",
                md_cell(&text(&x["day"])),
                text(&x["followers"]),
                text(&x["following"]),
                text(&x["gained"]),
                text(&x["lost"])
            );
        }
        o.push_str(
            "\nHistory starts at the first sync; a change is dated by the sync that found it.\n\n",
        );
    }

    o.push_str("## Post performance\n\n");
    if summary["analyzed"].as_u64().unwrap_or(0) == 0 {
        let _ = writeln!(
            o,
            "No data: {}\n",
            md_cell(reason(d, "posts_reason", "no post could be read."))
        );
    } else {
        o.push_str("| Metric | Posts | Total | Mean | Median |\n|---|---:|---:|---:|---:|\n");
        for (key, label) in METRICS {
            let m = &summary["metrics"][key];
            if m["available"] == true {
                let _ = writeln!(
                    o,
                    "| {label} | {} | {} | {} | {} |",
                    text(&m["posts"]),
                    text(&m["total"]),
                    text(&m["mean"]),
                    text(&m["median"])
                );
            } else {
                let _ = writeln!(o, "| {label} | not visible | | | |");
            }
        }
        o.push('\n');
    }

    o.push_str("## Best times\n\n");
    let mut cells: Vec<&Value> = list(&d["times"]["cells"])
        .iter()
        .filter(|c| c["thin"] == false)
        .collect();
    if cells.is_empty() {
        let _ = writeln!(
            o,
            "No data: no weekday and hour has enough posts in the sample.\n"
        );
    } else {
        cells.sort_by(|a, b| {
            b["mean"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["mean"].as_f64().unwrap_or(0.0))
        });
        let _ = writeln!(
            o,
            "Time zone {}. Hours with fewer than {} posts are left out.\n",
            md_cell(&text(&d["times"]["timezone"])),
            text(&d["times"]["thin_below"])
        );
        o.push_str("| Day | Hour | Posts | Mean engagement |\n|---|---:|---:|---:|\n");
        for c in cells.iter().take(10) {
            let day = c["weekday"]
                .as_u64()
                .filter(|x| *x < 7)
                .map_or("–", |x| DAYS[x as usize]);
            let _ = writeln!(
                o,
                "| {day} | {}:00 | {} | {} |",
                text(&c["hour"]),
                text(&c["posts"]),
                text(&c["mean"])
            );
        }
        o.push('\n');
    }

    o.push_str("## Top posts\n\n");
    let top = list(&d["top"]);
    if top.is_empty() {
        o.push_str("No data: no post in the sample.\n\n");
    } else {
        o.push_str("| Date | Post | Engagement | Likes | Replies | Reposts | Views | Link |\n|---|---|---:|---:|---:|---:|---:|---|\n");
        for p in top.iter().take(TOP_POSTS) {
            let link = safe_url(&p["url"])
                .map(|u| format!("<{}>", u.replace(['<', '>', '|'], "")))
                .unwrap_or_default();
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} | {} | {} | {} | {link} |",
                day_of(&p["created_at"]),
                md_cell(p["text"].as_str().unwrap_or_default()),
                text(&p["engagement"]),
                text(&p["likes"]),
                text(&p["replies"]),
                text(&p["reposts"]),
                text(&p["views"])
            );
        }
        o.push('\n');
    }

    o.push_str("## Hashtags\n\n");
    let tags = list(&d["hashtags"]);
    if tags.is_empty() {
        o.push_str("No data: no hashtag in the posts of the sample.\n\n");
    } else {
        o.push_str("| Hashtag | Uses | Mean engagement | Median |\n|---|---:|---:|---:|\n");
        for t in tags.iter().take(TOP_HASHTAGS) {
            let _ = writeln!(
                o,
                "| \\#{} | {} | {} | {} |",
                md_cell(&text(&t["hashtag"])),
                text(&t["uses"]),
                text(&t["engagement_mean"]),
                text(&t["engagement_median"])
            );
        }
        o.push('\n');
    }

    o.push_str("## Monitors\n\n");
    let monitors = list(&d["monitors"]);
    if monitors.is_empty() {
        let _ = writeln!(
            o,
            "No data: {}",
            md_cell(reason(
                d,
                "monitors_reason",
                "no monitor is saved for this network."
            ))
        );
    } else {
        o.push_str("| Monitor | Query | Unread | Stored |\n|---|---|---:|---:|\n");
        for m in monitors {
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} |",
                md_cell(&text(&m["name"])),
                md_cell(&text(&m["query"])),
                text(&m["unread"]),
                text(&m["total_stored"])
            );
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EVIL: &str = "<script>alert(1)</script>\"'&|";

    fn full() -> Value {
        json!({
            "platform": "Short Posts",
            "handle": "ana",
            "generated": "2026-11-11 10:15 -03:00",
            "profile": { "name": EVIL, "followers": 1200, "following": 300, "posts": 4521 },
            "summary": {
                "analyzed": 3, "from": 1_793_491_200, "to": 1_793_923_200, "confidence": "low",
                "engagement": { "available": true, "mean": 20.5, "median": 20.0 },
                "metrics": {
                    "likes": { "available": true, "posts": 3, "total": 60, "mean": 20.0, "median": 20.0 },
                    "replies": { "available": false }, "reposts": { "available": false },
                    "saves": { "available": false }, "views": { "available": true, "posts": 2, "total": 400, "mean": 200.0, "median": 200.0 }
                }
            },
            "audience": { "days": [
                { "day": "2026-11-01", "followers": 1180, "following": 300, "gained": 0, "lost": 0 },
                { "day": "2026-11-08", "followers": 1200, "following": 300, "gained": 25, "lost": 5 }
            ] },
            "times": { "timezone": "-03:00", "thin_below": 3, "best": { "weekday": 1, "hour": 9, "mean": 31.0, "posts": 3 },
                "cells": [ { "weekday": 1, "hour": 9, "posts": 3, "mean": 31.0, "thin": false }, { "weekday": 6, "hour": 23, "posts": 1, "mean": 4.0, "thin": true }, { "weekday": 9, "hour": 99, "posts": 1, "mean": 4.0, "thin": true } ] },
            "top": [
                { "created_at": 1_793_923_200, "text": EVIL, "url": "https://x.com/ana/status/1", "engagement": 30, "likes": 30, "replies": null, "reposts": null, "views": 300 },
                { "created_at": null, "text": "", "url": "javascript:alert(1)", "engagement": 10, "likes": 10 }
            ],
            "hashtags": [ { "hashtag": EVIL, "uses": 2, "engagement_mean": 25.0, "engagement_median": 25.0 } ],
            "monitors": [ { "name": EVIL, "query": "rust lang:pt", "unread": 4, "total_stored": 40, "last_checked": 1_793_923_200_000i64 } ]
        })
    }

    #[test]
    fn the_report_is_one_self_contained_file_with_every_section() {
        let page = html(&full());
        assert!(page.starts_with("<!doctype html>"));
        for section in [
            "Account",
            "Followers",
            "Post performance",
            "Best times",
            "Top posts",
            "Hashtags",
            "Monitors",
        ] {
            assert!(page.contains(&format!("<h2>{section}</h2>")), "{section}");
        }
        assert_eq!(page.matches("<svg").count(), 3);
        assert!(page.contains("<polyline"));
        assert!(page.contains("<b>25</b> gained, <b>5</b> lost"));
        assert!(page.contains("Best: <b>Tue 09:00</b>"));
        assert!(page.contains("stroke-dasharray"));
        assert!(page.contains(">1200<") && page.contains(">20.5<"));
        assert!(page.contains("not visible to a signed-in session"));
        assert!(page.contains("2026-11-06"));
        assert!(!page.contains("No data:"));
        let lower = page.to_lowercase();
        for banned in [
            "<script", "src=", "<link", "@import", "url(", "<img", "<iframe", "onload=", "onclick=",
        ] {
            assert!(!lower.contains(banned), "{banned}");
        }
        assert_eq!(lower.matches("http").count(), 1);
    }

    #[test]
    fn network_text_is_escaped_and_unsafe_links_are_not_linked() {
        let page = html(&full());
        assert!(!page.contains("<script>alert"));
        assert_eq!(
            page.matches("&lt;script&gt;alert(1)&lt;/script&gt;&quot;&#39;&amp;|")
                .count(),
            4
        );
        assert_eq!(page.matches("<a href=").count(), 1);
        assert!(
            page.contains("<a href=\"https://x.com/ana/status/1\" rel=\"noreferrer noopener\">")
        );
        assert!(!page.contains("javascript:"));
        assert!(page.contains("(no text)"));
        assert_eq!(esc("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
        let evil_handle =
            html(&json!({ "platform": "Short Posts", "handle": "\"><script>x</script>" }));
        assert!(!evil_handle.contains("<script>"));
        let md = markdown(&full());
        assert!(!md.contains("<script>"));
        assert!(md.contains("&lt;script&gt;alert(1)&lt;/script&gt;\"'&\\|"));
        assert!(!md.contains("javascript:"));
        assert!(md.contains("<https://x.com/ana/status/1>"));
        assert_eq!(
            md_cell("a|b\n*c* [x](y) `z`"),
            "a\\|b \\*c\\* \\[x\\](y) \\`z\\`"
        );
    }

    #[test]
    fn sections_without_data_say_so_in_both_files() {
        let d = json!({
            "platform": "Photo Feed", "handle": "ana", "generated": "now",
            "posts_reason": "NOT_LOGGED_IN: sign in to Photo Feed",
            "audience_reason": "No audience snapshot yet.",
            "monitors_reason": "No monitor saved.",
        });
        let page = html(&d);
        assert_eq!(page.matches("No data:").count(), 7);
        assert!(page.contains("No data: NOT_LOGGED_IN: sign in to Photo Feed"));
        assert!(page.contains("No data: No audience snapshot yet."));
        assert!(page.contains("No data: No monitor saved."));
        assert!(!page.contains("<svg") && !page.contains("<table"));
        let md = markdown(&d);
        assert_eq!(md.matches("No data:").count(), 7);
        assert!(md.starts_with("# Photo Feed report for @ana"));
        let one_day = json!({ "platform": "Short Posts", "handle": "a", "audience": { "days": [{ "day": "2026-11-01", "followers": 10, "gained": 0, "lost": 0 }] } });
        let page = html(&one_day);
        assert!(page.contains("The trend line needs two days"));
        assert!(page.contains("No follower was gained or lost"));
    }

    #[test]
    fn the_markdown_twin_has_the_same_numbers() {
        let md = markdown(&full());
        for line in [
            "- Followers: 1200",
            "- Median engagement: 20",
            "| 2026-11-08 | 1200 | 300 | 25 | 5 |",
            "| Likes | 3 | 60 | 20 | 20 |",
            "| Replies | not visible | | | |",
            "| Tue | 9:00 | 3 | 31 |",
            "| rust lang:pt | 4 | 40 |",
        ] {
            assert!(md.contains(line), "{line}\n{md}");
        }
        assert_eq!(md.matches("\n## ").count(), 7);
    }
}
