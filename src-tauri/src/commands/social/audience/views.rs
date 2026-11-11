use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use serde::Serialize;
use serde_json::{json, Value};

use super::db::{EventRow, Member, Snap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Followers,
    Following,
    NotFollowingBack,
    Fans,
    Mutuals,
    NewFollowers,
    Unfollowers,
    Inactive,
    Kept,
    Hidden,
    Summary,
    Trend,
    Filters,
    Interests,
}

pub const VIEWS: [(&str, View); 14] = [
    ("followers", View::Followers),
    ("following", View::Following),
    ("not_following_back", View::NotFollowingBack),
    ("fans", View::Fans),
    ("mutuals", View::Mutuals),
    ("new_followers", View::NewFollowers),
    ("unfollowers", View::Unfollowers),
    ("inactive", View::Inactive),
    ("kept", View::Kept),
    ("hidden", View::Hidden),
    ("summary", View::Summary),
    ("trend", View::Trend),
    ("filters", View::Filters),
    ("interests", View::Interests),
];

impl View {
    pub fn parse(s: &str) -> Option<Self> {
        VIEWS.iter().find(|(id, _)| *id == s).map(|(_, v)| *v)
    }

    pub fn id(self) -> &'static str {
        VIEWS
            .iter()
            .find(|(_, v)| *v == self)
            .map(|(id, _)| *id)
            .unwrap_or_default()
    }

    pub fn is_list(self) -> bool {
        !matches!(
            self,
            View::Summary | View::Trend | View::Filters | View::Interests
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Newest,
    Oldest,
    Followers,
    FollowersAsc,
    Following,
    FollowingAsc,
    Posts,
    PostsAsc,
    Ratio,
    RatioAsc,
    RatioBalanced,
    Handle,
    Name,
    FirstSeen,
    FirstSeenAsc,
}

pub const SORTS: [(&str, Sort); 15] = [
    ("newest", Sort::Newest),
    ("oldest", Sort::Oldest),
    ("followers", Sort::Followers),
    ("followers_asc", Sort::FollowersAsc),
    ("following", Sort::Following),
    ("following_asc", Sort::FollowingAsc),
    ("posts", Sort::Posts),
    ("posts_asc", Sort::PostsAsc),
    ("ratio", Sort::Ratio),
    ("ratio_asc", Sort::RatioAsc),
    ("ratio_balanced", Sort::RatioBalanced),
    ("handle", Sort::Handle),
    ("name", Sort::Name),
    ("first_seen", Sort::FirstSeen),
    ("first_seen_asc", Sort::FirstSeenAsc),
];

type Range = (Option<f64>, Option<f64>);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub q: Option<String>,
    pub followers: Range,
    pub following: Range,
    pub posts: Range,
    pub ratio: Range,
    pub verified: Option<bool>,
    pub private: Option<bool>,
    pub has_avatar: Option<bool>,
    pub has_bio: Option<bool>,
    pub has_link: Option<bool>,
    pub kept: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub filter: Filter,
    pub sort: Sort,
    pub limit: usize,
    pub offset: usize,
    pub since: Option<i64>,
    pub include_hidden: bool,
    pub inactive_days: i64,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            filter: Filter::default(),
            sort: Sort::Newest,
            limit: 50,
            offset: 0,
            since: None,
            include_hidden: false,
            inactive_days: 90,
        }
    }
}

impl Query {
    pub fn parse(a: &Value) -> Result<Self, String> {
        let range = |name: &str| -> Range {
            (
                a[format!("min_{name}")].as_f64(),
                a[format!("max_{name}")].as_f64(),
            )
        };
        let sort = match a["sort"].as_str() {
            None => Sort::Newest,
            Some(s) => SORTS
                .iter()
                .find(|(id, _)| *id == s)
                .map(|(_, v)| *v)
                .ok_or_else(|| format!("INVALID_ARGUMENTS: unknown sort {s}"))?,
        };
        Ok(Self {
            filter: Filter {
                q: a["q"]
                    .as_str()
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty()),
                followers: range("followers"),
                following: range("following"),
                posts: range("posts"),
                ratio: range("ratio"),
                verified: a["verified"].as_bool(),
                private: a["private"].as_bool(),
                has_avatar: a["has_avatar"].as_bool(),
                has_bio: a["has_bio"].as_bool(),
                has_link: a["has_link"].as_bool(),
                kept: a["kept"].as_bool(),
            },
            sort,
            limit: a["limit"].as_u64().unwrap_or(50).clamp(1, 200) as usize,
            offset: a["offset"].as_u64().unwrap_or(0) as usize,
            since: a["since"].as_i64(),
            include_hidden: a["include_hidden"].as_bool().unwrap_or(false),
            inactive_days: a["inactive_days"].as_i64().unwrap_or(90).clamp(1, 3650),
        })
    }
}

#[derive(Default)]
pub struct Data {
    pub platform: String,
    pub followers: Option<Vec<Member>>,
    pub following: Option<Vec<Member>>,
    pub keep: HashMap<String, String>,
    pub hidden: HashMap<String, String>,
    pub events: Vec<EventRow>,
    pub activity: HashMap<String, Option<i64>>,
    pub now: i64,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Row {
    #[serde(flatten)]
    pub member: Member,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follows_me: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followed_by_me: Option<bool>,
    pub kept: bool,
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between: Option<[i64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_post: Option<i64>,
    #[serde(skip)]
    pub event_id: Option<i64>,
}

pub fn profile_url(platform: &str, handle: &str) -> String {
    match platform {
        "x" => format!("https://x.com/{handle}"),
        "instagram" => format!("https://www.instagram.com/{handle}/"),
        "github" => format!("https://github.com/{handle}"),
        "pinterest" => format!("https://www.pinterest.com/{handle}/"),
        _ => format!("https://www.tiktok.com/@{handle}"),
    }
}

pub fn ratio(m: &Member) -> Option<f64> {
    match (m.followers, m.following) {
        (Some(a), Some(b)) if b > 0 => Some((a as f64 / b as f64 * 100.0).round() / 100.0),
        _ => None,
    }
}

pub fn has_avatar(m: &Member) -> bool {
    m.avatar
        .as_deref()
        .is_some_and(|a| !a.contains("default_profile") && !a.contains("44884218_345707102882519"))
}

pub fn has_link(bio: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?i)(https?://|www\.|\b[a-z0-9-]{2,}\.(com|net|org|io|co|me|app|dev|ly|gg|tv|link|bio|ai|xyz|br|to)\b)")
            .expect("static regex")
    })
    .is_match(bio)
}

fn in_range(v: Option<f64>, (min, max): Range) -> bool {
    if min.is_none() && max.is_none() {
        return true;
    }
    v.is_some_and(|v| min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m))
}

impl Filter {
    pub fn matches(&self, r: &Row) -> bool {
        let m = &r.member;
        let flag = |want: Option<bool>, got: bool| want.is_none_or(|w| w == got);
        let text = self.q.as_deref().is_none_or(|q| {
            m.handle.to_lowercase().contains(q)
                || m.name.to_lowercase().contains(q)
                || m.bio.to_lowercase().contains(q)
        });
        text && in_range(m.followers.map(|n| n as f64), self.followers)
            && in_range(m.following.map(|n| n as f64), self.following)
            && in_range(m.posts.map(|n| n as f64), self.posts)
            && in_range(r.ratio, self.ratio)
            && flag(self.verified, m.verified)
            && flag(self.private, m.private)
            && flag(self.has_avatar, has_avatar(m))
            && flag(self.has_bio, !m.bio.trim().is_empty())
            && flag(self.has_link, has_link(&m.bio))
            && flag(self.kept, r.kept)
    }
}

fn by_number(rows: &mut [Row], key: impl Fn(&Row) -> Option<f64>, desc: bool) {
    rows.sort_by(|a, b| match (key(a), key(b)) {
        (Some(x), Some(y)) => {
            let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
            if desc {
                o.reverse()
            } else {
                o
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
}

pub fn sort(rows: &mut [Row], by: Sort) {
    let n = |v: Option<i64>| v.map(|x| x as f64);
    match by {
        Sort::Newest => {}
        Sort::Oldest => rows.reverse(),
        Sort::Followers => by_number(rows, |r| n(r.member.followers), true),
        Sort::FollowersAsc => by_number(rows, |r| n(r.member.followers), false),
        Sort::Following => by_number(rows, |r| n(r.member.following), true),
        Sort::FollowingAsc => by_number(rows, |r| n(r.member.following), false),
        Sort::Posts => by_number(rows, |r| n(r.member.posts), true),
        Sort::PostsAsc => by_number(rows, |r| n(r.member.posts), false),
        Sort::Ratio => by_number(rows, |r| r.ratio, true),
        Sort::RatioAsc => by_number(rows, |r| r.ratio, false),
        Sort::RatioBalanced => by_number(
            rows,
            |r| r.ratio.filter(|x| *x > 0.0).map(|x| x.ln().abs()),
            false,
        ),
        Sort::Handle => rows.sort_by_key(|r| r.member.handle.to_lowercase()),
        Sort::Name => rows.sort_by_key(|r| r.member.name.to_lowercase()),
        Sort::FirstSeen => by_number(rows, |r| n(r.member.first_seen), true),
        Sort::FirstSeenAsc => by_number(rows, |r| n(r.member.first_seen), false),
    }
}

#[derive(Debug, PartialEq)]
pub enum Activity {
    Inactive(Option<i64>),
    Active,
    Unknown,
}

pub fn activity_of(m: &Member, d: &Data, inactive_days: i64) -> Activity {
    if m.posts == Some(0) {
        return Activity::Inactive(None);
    }
    match d.activity.get(&m.id) {
        None => Activity::Unknown,
        Some(None) => Activity::Inactive(None),
        Some(Some(t)) if *t < d.now - inactive_days * 86400 => Activity::Inactive(Some(*t)),
        Some(Some(_)) => Activity::Active,
    }
}

pub fn inactive_unknown<'a>(d: &'a Data, q: &Query) -> Vec<&'a Member> {
    d.following
        .iter()
        .flatten()
        .filter(|m| q.include_hidden || !d.hidden.contains_key(&m.id))
        .filter(|m| activity_of(m, d, q.inactive_days) == Activity::Unknown)
        .collect()
}

pub fn rows(d: &Data, view: View, q: &Query) -> Vec<Row> {
    fn ids(l: &Option<Vec<Member>>) -> Option<HashSet<&str>> {
        l.as_ref()
            .map(|l| l.iter().map(|m| m.id.as_str()).collect())
    }
    fn within(set: &Option<HashSet<&str>>, id: &str) -> bool {
        set.as_ref().is_some_and(|s| s.contains(id))
    }
    let fo_ids = ids(&d.followers);
    let fg_ids = ids(&d.following);
    let decorate = |m: &Member| Row {
        url: profile_url(&d.platform, &m.handle),
        ratio: ratio(m),
        follows_me: fo_ids.as_ref().map(|s| s.contains(m.id.as_str())),
        followed_by_me: fg_ids.as_ref().map(|s| s.contains(m.id.as_str())),
        kept: d.keep.contains_key(&m.id),
        hidden: d.hidden.contains_key(&m.id),
        member: m.clone(),
        ..Default::default()
    };
    let fo = d.followers.as_deref().unwrap_or_default();
    let fg = d.following.as_deref().unwrap_or_default();
    let both = fo_ids.is_some() && fg_ids.is_some();
    let mut out: Vec<Row> = match view {
        View::Followers => fo.iter().map(decorate).collect(),
        View::Following => fg.iter().map(decorate).collect(),
        View::NotFollowingBack if both => fg
            .iter()
            .filter(|m| !within(&fo_ids, &m.id))
            .map(decorate)
            .collect(),
        View::Fans if both => fo
            .iter()
            .filter(|m| !within(&fg_ids, &m.id))
            .map(decorate)
            .collect(),
        View::Mutuals if both => fo
            .iter()
            .filter(|m| within(&fg_ids, &m.id))
            .map(decorate)
            .collect(),
        View::NewFollowers | View::Unfollowers => {
            let lost = view == View::Unfollowers;
            let want = if lost { "lost" } else { "gained" };
            d.events
                .iter()
                .filter(|e| e.event == want && q.since.is_none_or(|s| e.newer >= s))
                .map(|e| {
                    let mut r = decorate(&e.member);
                    r.event = Some(e.event.clone());
                    r.between = Some([e.older, e.newer]);
                    r.current = Some(within(&fo_ids, &e.member.id));
                    r.event_id = Some(e.id);
                    if lost {
                        r.status = Some(if e.gone == Some(true) {
                            "gone"
                        } else {
                            "unfollowed"
                        });
                        r.checked = Some(e.gone.is_some());
                    }
                    r
                })
                .collect()
        }
        View::Inactive => fg
            .iter()
            .filter_map(|m| match activity_of(m, d, q.inactive_days) {
                Activity::Inactive(last) => {
                    let mut r = decorate(m);
                    r.activity = Some(if last.is_some() {
                        "inactive"
                    } else {
                        "no_posts"
                    });
                    r.last_post = last;
                    Some(r)
                }
                _ => None,
            })
            .collect(),
        View::Kept | View::Hidden => {
            let marks = if view == View::Kept {
                &d.keep
            } else {
                &d.hidden
            };
            let mut seen = HashSet::new();
            let mut list: Vec<Row> = fg
                .iter()
                .chain(fo)
                .filter(|m| marks.contains_key(&m.id) && seen.insert(m.id.as_str()))
                .map(decorate)
                .collect();
            let mut rest: Vec<(&String, &String)> = marks
                .iter()
                .filter(|(id, _)| !seen.contains(id.as_str()))
                .collect();
            rest.sort();
            list.extend(rest.into_iter().map(|(id, handle)| {
                decorate(&Member {
                    id: id.clone(),
                    handle: handle.clone(),
                    ..Default::default()
                })
            }));
            list
        }
        _ => Vec::new(),
    };
    if !q.include_hidden && view != View::Hidden {
        out.retain(|r| !r.hidden);
    }
    out.retain(|r| q.filter.matches(r));
    sort(&mut out, q.sort);
    out
}

pub fn unavailable(rows: &[Row]) -> Vec<&'static str> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if rows.iter().all(|r| r.member.followers.is_none()) {
        out.push("followers");
    }
    if rows.iter().all(|r| r.member.following.is_none()) {
        out.push("following");
        out.push("ratio");
    }
    if rows.iter().all(|r| r.member.posts.is_none()) {
        out.push("posts");
    }
    if rows.iter().all(|r| r.member.bio.is_empty()) {
        out.push("bio");
    }
    out
}

pub fn paged(view: View, rows: Vec<Row>, q: &Query) -> Value {
    let total = rows.len();
    let missing = unavailable(&rows);
    let items: Vec<Row> = rows.into_iter().skip(q.offset).take(q.limit).collect();
    json!({
        "view": view.id(),
        "total": total,
        "returned": items.len(),
        "offset": q.offset,
        "truncated": q.offset + items.len() < total,
        "unavailable_fields": missing,
        "items": items,
    })
}

pub fn counts(d: &Data, q: &Query) -> Value {
    let base = Query {
        since: q.since,
        include_hidden: q.include_hidden,
        inactive_days: q.inactive_days,
        ..Default::default()
    };
    let mut out = serde_json::Map::new();
    for (id, v) in VIEWS.iter().filter(|(_, v)| v.is_list()) {
        out.insert((*id).to_string(), json!(rows(d, *v, &base).len()));
    }
    out.insert(
        "inactive_unknown".into(),
        json!(inactive_unknown(d, &base).len()),
    );
    Value::Object(out)
}

fn day(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Day {
    pub day: String,
    pub followers: Option<i64>,
    pub following: Option<i64>,
    pub ratio: Option<f64>,
    pub gained: i64,
    pub lost: i64,
}

pub fn trend(snaps: &[Snap], events: &[EventRow], since: Option<i64>) -> Vec<Day> {
    let mut days: BTreeMap<String, Day> = BTreeMap::new();
    for s in snaps {
        let Some(at) = s.finished.filter(|f| since.is_none_or(|x| *f >= x)) else {
            continue;
        };
        let d = days.entry(day(at)).or_default();
        if s.kind == "followers" {
            d.followers = Some(s.count);
        } else {
            d.following = Some(s.count);
        }
    }
    for e in events.iter().filter(|e| since.is_none_or(|x| e.newer >= x)) {
        let d = days.entry(day(e.newer)).or_default();
        if e.event == "gained" {
            d.gained += 1;
        } else {
            d.lost += 1;
        }
    }
    days.into_iter()
        .map(|(k, mut d)| {
            d.day = k;
            d.ratio = match (d.followers, d.following) {
                (Some(a), Some(b)) if b > 0 => Some((a as f64 / b as f64 * 100.0).round() / 100.0),
                _ => None,
            };
            d
        })
        .collect()
}

const BUCKETS: [(&str, i64); 5] = [
    ("under_7d", 7),
    ("7_30d", 30),
    ("30_90d", 90),
    ("90_365d", 365),
    ("over_365d", i64::MAX),
];

pub fn retention(followers: &[Member], history_started: Option<i64>, now: i64) -> Value {
    let mut counts = [0usize; 5];
    let mut before = 0usize;
    for m in followers {
        match (m.first_seen, history_started) {
            (Some(seen), Some(start)) if seen > start => {
                let days = (now - seen).max(0) / 86400;
                let i = BUCKETS.iter().position(|(_, max)| days < *max).unwrap_or(4);
                counts[i] += 1;
            }
            _ => before += 1,
        }
    }
    json!({
        "buckets": BUCKETS.iter().zip(counts).map(|((id, _), n)| json!({ "bucket": id, "count": n })).collect::<Vec<_>>(),
        "since_before_history": before,
        "history_started": history_started,
        "history_days": history_started.map(|s| (now - s).max(0) / 86400),
    })
}

pub fn csv_cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

fn iso(ts: Option<i64>) -> String {
    ts.and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

pub fn csv(rows: &[Row]) -> String {
    let num = |v: Option<i64>| v.map(|n| n.to_string()).unwrap_or_default();
    let flag = |v: Option<bool>| v.map(|b| b.to_string()).unwrap_or_default();
    let mut out = String::from(
        "handle,name,id,url,verified,private,followers,following,posts,ratio,bio,follows_me,followed_by_me,kept,first_seen,event,between_from,between_to,status,last_post\r\n",
    );
    for r in rows {
        let m = &r.member;
        let cells = [
            csv_cell(&m.handle),
            csv_cell(&m.name),
            csv_cell(&m.id),
            csv_cell(&r.url),
            m.verified.to_string(),
            m.private.to_string(),
            num(m.followers),
            num(m.following),
            num(m.posts),
            r.ratio.map(|x| x.to_string()).unwrap_or_default(),
            csv_cell(&m.bio),
            flag(r.follows_me),
            flag(r.followed_by_me),
            r.kept.to_string(),
            iso(m.first_seen),
            r.event.clone().unwrap_or_default(),
            iso(r.between.map(|b| b[0])),
            iso(r.between.map(|b| b[1])),
            r.status.unwrap_or_default().to_string(),
            iso(r.last_post),
        ];
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, followers: Option<i64>, following: Option<i64>) -> Member {
        Member {
            id: id.into(),
            handle: format!("user{id}"),
            name: format!("Name {id}"),
            followers,
            following,
            ..Default::default()
        }
    }

    fn list(ids: &[&str]) -> Vec<Member> {
        ids.iter().map(|i| m(i, None, None)).collect()
    }

    fn data() -> Data {
        Data {
            platform: "x".into(),
            followers: Some(list(&["1", "2", "3"])),
            following: Some(list(&["2", "3", "4", "5"])),
            now: 1_000_000,
            ..Default::default()
        }
    }

    fn ids(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.member.id.as_str()).collect()
    }

    fn event(id: i64, kind: &str, user: &str, newer: i64, gone: Option<bool>) -> EventRow {
        EventRow {
            id,
            event: kind.into(),
            older: newer - 100,
            newer,
            gone,
            member: m(user, None, None),
        }
    }

    #[test]
    fn every_set_view_is_plain_set_arithmetic() {
        let d = data();
        let q = Query::default();
        assert_eq!(ids(&rows(&d, View::Followers, &q)), vec!["1", "2", "3"]);
        assert_eq!(
            ids(&rows(&d, View::Following, &q)),
            vec!["2", "3", "4", "5"]
        );
        assert_eq!(ids(&rows(&d, View::NotFollowingBack, &q)), vec!["4", "5"]);
        assert_eq!(ids(&rows(&d, View::Fans, &q)), vec!["1"]);
        assert_eq!(ids(&rows(&d, View::Mutuals, &q)), vec!["2", "3"]);
        let r = &rows(&d, View::Followers, &q)[0];
        assert_eq!((r.follows_me, r.followed_by_me), (Some(true), Some(false)));
        assert_eq!(r.url, "https://x.com/user1");
    }

    #[test]
    fn views_that_need_both_lists_are_empty_with_only_one() {
        let mut d = data();
        d.following = None;
        let q = Query::default();
        assert_eq!(rows(&d, View::Followers, &q).len(), 3);
        assert!(rows(&d, View::Fans, &q).is_empty());
        assert!(rows(&d, View::NotFollowingBack, &q).is_empty());
        assert!(rows(&d, View::Mutuals, &q).is_empty());
        assert_eq!(rows(&d, View::Followers, &q)[0].followed_by_me, None);
    }

    #[test]
    fn hidden_accounts_leave_every_view_and_kept_ones_are_flagged() {
        let mut d = data();
        d.hidden.insert("2".into(), "user2".into());
        d.keep.insert("4".into(), "user4".into());
        d.keep.insert("77".into(), "left".into());
        let q = Query::default();
        assert_eq!(ids(&rows(&d, View::Mutuals, &q)), vec!["3"]);
        assert_eq!(ids(&rows(&d, View::Followers, &q)), vec!["1", "3"]);
        let nfb = rows(&d, View::NotFollowingBack, &q);
        assert!(nfb[0].kept && !nfb[1].kept);
        assert_eq!(ids(&rows(&d, View::Hidden, &q)), vec!["2"]);
        assert_eq!(ids(&rows(&d, View::Kept, &q)), vec!["4", "77"]);
        let all = Query {
            include_hidden: true,
            ..Default::default()
        };
        assert_eq!(ids(&rows(&d, View::Mutuals, &all)), vec!["2", "3"]);
        let c = counts(&d, &q);
        assert_eq!(c["followers"], 2);
        assert_eq!(c["hidden"], 1);
        assert_eq!(c["kept"], 2);
        assert_eq!(c["inactive_unknown"], 3);
    }

    #[test]
    fn event_views_carry_the_window_and_the_status() {
        let mut d = data();
        d.events = vec![
            event(1, "lost", "9", 900, Some(true)),
            event(2, "lost", "1", 800, None),
            event(3, "gained", "3", 800, None),
            event(4, "lost", "8", 500, Some(false)),
        ];
        let q = Query::default();
        let lost = rows(&d, View::Unfollowers, &q);
        assert_eq!(ids(&lost), vec!["9", "1", "8"]);
        assert_eq!(lost[0].between, Some([800, 900]));
        assert_eq!(
            (lost[0].status, lost[0].checked),
            (Some("gone"), Some(true))
        );
        assert_eq!(
            (lost[1].status, lost[1].checked),
            (Some("unfollowed"), Some(false))
        );
        assert_eq!(lost[1].current, Some(true));
        assert_eq!(
            (lost[2].status, lost[2].checked),
            (Some("unfollowed"), Some(true))
        );
        assert_eq!(lost[2].event_id, Some(4));
        let gained = rows(&d, View::NewFollowers, &q);
        assert_eq!(ids(&gained), vec!["3"]);
        assert_eq!(gained[0].status, None);
        let recent = Query {
            since: Some(850),
            ..Default::default()
        };
        assert_eq!(ids(&rows(&d, View::Unfollowers, &recent)), vec!["9"]);
    }

    #[test]
    fn inactive_uses_cached_last_post_and_reports_unknown() {
        let mut d = data();
        d.now = 200 * 86400;
        d.following = Some(vec![
            m("2", None, None),
            m("3", None, None),
            m("4", None, None),
            Member {
                posts: Some(0),
                ..m("5", None, None)
            },
            m("6", None, None),
        ]);
        d.activity.insert("2".into(), Some(190 * 86400));
        d.activity.insert("3".into(), Some(50 * 86400));
        d.activity.insert("4".into(), None);
        let q = Query::default();
        let r = rows(&d, View::Inactive, &q);
        assert_eq!(ids(&r), vec!["3", "4", "5"]);
        assert_eq!(
            (r[0].activity, r[0].last_post),
            (Some("inactive"), Some(50 * 86400))
        );
        assert_eq!((r[1].activity, r[1].last_post), (Some("no_posts"), None));
        assert_eq!(
            inactive_unknown(&d, &q)
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["6"]
        );
        let strict = Query {
            inactive_days: 5,
            ..Default::default()
        };
        assert_eq!(
            ids(&rows(&d, View::Inactive, &strict)),
            vec!["2", "3", "4", "5"]
        );
    }

    #[test]
    fn filters_combine() {
        let mut d = data();
        d.followers = Some(vec![
            Member {
                bio: "Designer, see https://ana.dev".into(),
                verified: true,
                avatar: Some("https://pbs.twimg.com/a.jpg".into()),
                posts: Some(40),
                ..m("1", Some(1000), Some(100))
            },
            Member {
                private: true,
                avatar: Some("https://abs.twimg.com/sticky/default_profile_images/x.png".into()),
                ..m("2", Some(10), Some(500))
            },
            m("3", None, None),
        ]);
        let run = |a: Value| {
            ids(&rows(&d, View::Followers, &Query::parse(&a).unwrap()))
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(json!({ "q": "DESIGNER" })), vec!["1"]);
        assert_eq!(run(json!({ "q": "user2" })), vec!["2"]);
        assert_eq!(run(json!({ "q": "name 3" })), vec!["3"]);
        assert_eq!(run(json!({ "min_followers": 100 })), vec!["1"]);
        assert_eq!(run(json!({ "max_followers": 100 })), vec!["2"]);
        assert_eq!(run(json!({ "min_ratio": 1 })), vec!["1"]);
        assert_eq!(run(json!({ "max_ratio": 0.5 })), vec!["2"]);
        assert_eq!(run(json!({ "min_posts": 1 })), vec!["1"]);
        assert_eq!(run(json!({ "verified": true })), vec!["1"]);
        assert_eq!(run(json!({ "private": false })), vec!["1", "3"]);
        assert_eq!(run(json!({ "has_avatar": false })), vec!["2", "3"]);
        assert_eq!(run(json!({ "has_bio": true })), vec!["1"]);
        assert_eq!(run(json!({ "has_link": true })), vec!["1"]);
        assert_eq!(
            run(json!({ "verified": true, "max_followers": 5 })),
            Vec::<String>::new()
        );
        assert!(Query::parse(&json!({ "sort": "nope" })).is_err());
        assert!(has_link("portfolio: ana.dev"));
        assert!(!has_link("hello. world"));
    }

    #[test]
    fn sorts_put_missing_values_last() {
        let mut d = data();
        d.followers = Some(vec![
            Member {
                first_seen: Some(30),
                ..m("1", Some(50), Some(100))
            },
            m("2", None, None),
            Member {
                first_seen: Some(90),
                posts: Some(3),
                ..m("3", Some(900), Some(1000))
            },
            Member {
                first_seen: Some(60),
                ..m("4", Some(300), Some(10))
            },
        ]);
        let run = |s: &str| {
            ids(&rows(
                &d,
                View::Followers,
                &Query::parse(&json!({ "sort": s })).unwrap(),
            ))
            .join("")
        };
        assert_eq!(run("newest"), "1234");
        assert_eq!(run("oldest"), "4321");
        assert_eq!(run("followers"), "3412");
        assert_eq!(run("followers_asc"), "1432");
        assert_eq!(run("following"), "3142");
        assert_eq!(run("ratio"), "4312");
        assert_eq!(run("ratio_asc"), "1342");
        assert_eq!(run("ratio_balanced"), "3142");
        assert_eq!(run("posts"), "3124");
        assert_eq!(run("first_seen"), "3412");
        assert_eq!(run("first_seen_asc"), "1432");
        assert_eq!(run("handle"), "1234");
    }

    #[test]
    fn paging_reports_total_returned_and_truncated() {
        let d = data();
        let q = Query::parse(&json!({ "limit": 2, "offset": 1 })).unwrap();
        let v = paged(View::Following, rows(&d, View::Following, &q), &q);
        assert_eq!(
            (v["total"].as_u64(), v["returned"].as_u64()),
            (Some(4), Some(2))
        );
        assert_eq!(v["truncated"], true);
        assert_eq!(v["items"][0]["id"], "3");
        assert_eq!(v["items"][0]["kept"], false);
        assert_eq!(
            v["unavailable_fields"],
            json!(["followers", "following", "ratio", "posts", "bio"])
        );
        let q = Query::parse(&json!({ "offset": 2 })).unwrap();
        let v = paged(View::Following, rows(&d, View::Following, &q), &q);
        assert_eq!(v["truncated"], false);
    }

    #[test]
    fn trend_groups_snapshots_and_events_by_day() {
        let snap = |kind: &str, finished: i64, count: i64| Snap {
            id: 0,
            kind: kind.into(),
            started: finished - 5,
            finished: Some(finished),
            state: "complete".into(),
            count,
            expected: None,
            cursor: None,
        };
        let d1 = 1_762_819_200;
        let d2 = d1 + 86400;
        let snaps = [
            snap("followers", d1 + 10, 100),
            snap("following", d1 + 20, 50),
            snap("followers", d1 + 3600, 101),
            snap("followers", d2 + 10, 99),
        ];
        let events = [
            event(1, "gained", "1", d1 + 3600, None),
            event(2, "lost", "2", d2 + 10, None),
            event(3, "lost", "3", d2 + 10, None),
            event(4, "gained", "4", d2 + 10, None),
        ];
        let t = trend(&snaps, &events, None);
        assert_eq!(
            t,
            vec![
                Day {
                    day: "2025-11-11".into(),
                    followers: Some(101),
                    following: Some(50),
                    ratio: Some(2.02),
                    gained: 1,
                    lost: 0
                },
                Day {
                    day: "2025-11-12".into(),
                    followers: Some(99),
                    following: None,
                    ratio: None,
                    gained: 1,
                    lost: 2
                },
            ]
        );
        assert_eq!(trend(&snaps, &events, Some(d2)).len(), 1);
    }

    #[test]
    fn retention_buckets_by_first_seen_and_separates_the_baseline() {
        let day = 86400;
        let now = 1000 * day;
        let start = 100 * day;
        let at = |id: &str, seen: Option<i64>| Member {
            first_seen: seen,
            ..m(id, None, None)
        };
        let followers = vec![
            at("1", Some(start)),
            at("2", None),
            at("3", Some(now - 2 * day)),
            at("4", Some(now - 7 * day)),
            at("5", Some(now - 29 * day)),
            at("6", Some(now - 89 * day)),
            at("7", Some(now - 200 * day)),
            at("8", Some(now - 400 * day)),
        ];
        let r = retention(&followers, Some(start), now);
        let counts: Vec<u64> = r["buckets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["count"].as_u64().unwrap())
            .collect();
        assert_eq!(counts, vec![1, 2, 1, 1, 1]);
        assert_eq!(r["buckets"][0]["bucket"], "under_7d");
        assert_eq!(r["since_before_history"], 2);
        assert_eq!(r["history_days"], 900);
    }

    #[test]
    fn csv_escapes_quotes_commas_newlines_and_formulas() {
        assert_eq!(csv_cell("plain"), "plain");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("line\nbreak"), "\"line\nbreak\"");
        assert_eq!(csv_cell("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_cell("@ana, dev"), "\"'@ana, dev\"");
        let mut d = data();
        d.followers = Some(vec![Member {
            bio: "dev, \"maker\"".into(),
            first_seen: Some(0),
            ..m("1", Some(10), Some(5))
        }]);
        let out = csv(&rows(&d, View::Followers, &Query::default()));
        let lines: Vec<&str> = out.split("\r\n").collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("handle,name,id,url,"));
        assert_eq!(
            lines[1],
            "user1,Name 1,1,https://x.com/user1,false,false,10,5,,2,\"dev, \"\"maker\"\"\",true,false,false,1970-01-01T00:00:00Z,,,,,"
        );
    }

    #[test]
    fn view_names_round_trip() {
        for (id, v) in VIEWS {
            assert_eq!(View::parse(id), Some(v));
            assert_eq!(v.id(), id);
        }
        assert_eq!(View::parse("nope"), None);
    }
}
