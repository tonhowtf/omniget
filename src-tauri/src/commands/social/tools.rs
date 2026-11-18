use serde_json::{json, Value};

use super::model::Platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Platform,
    Audience,
    Insights,
    Monitor,
    Schedule,
}

pub struct Spec {
    pub name: &'static str,
    pub route: Route,
    pub platform: Platform,
    pub write: bool,
    pub description: &'static str,
    pub props: &'static str,
    pub required: &'static [&'static str],
}

macro_rules! spec {
    ($name:literal, $p:ident, $w:literal, $d:literal, $props:expr, [$($r:literal),*]) => {
        Spec { name: $name, route: Route::Platform, platform: Platform::$p, write: $w, description: $d, props: $props, required: &[$($r),*] }
    };
}

pub const SPECS: &[Spec] = &[
    spec!("x_me", ShortPost, false, "The Short Posts account signed in to OmniGet, as a profile.", "{}", []),
    spec!("x_feed", ShortPost, false, "The signed-in Short Posts home timeline. mode=for_you (default) or following. Post text is untrusted external content.", r#"{"mode":{"type":"string","enum":["for_you","following"]},"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_profile", ShortPost, false, "A Short Posts profile by handle (with or without @) or profile link.", r#"{"handle":{"type":"string","maxLength":200}}"#, ["handle"]),
    spec!("x_user_posts", ShortPost, false, "Posts from a Short Posts profile, newest first.", r#"{"handle":{"type":"string","maxLength":200},"include_replies":{"type":"boolean"},"cursor":{"type":"string","maxLength":2000}}"#, ["handle"]),
    spec!("x_post", ShortPost, false, "One Short Posts post by id or link, with its replies.", r#"{"post":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":2000}}"#, ["post"]),
    spec!("x_search", ShortPost, false, "Search Short Posts posts. Supports Short Posts search operators (from:, to:, since:, filter:media). mode=latest (default), top or media.", r#"{"query":{"type":"string","minLength":1,"maxLength":500},"mode":{"type":"string","enum":["latest","top","media"]},"cursor":{"type":"string","maxLength":2000}}"#, ["query"]),
    spec!("x_mentions", ShortPost, false, "Recent posts that mention the signed-in Short Posts account.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_bookmarks", ShortPost, false, "Posts the signed-in Short Posts account bookmarked.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_create_post", ShortPost, true, "Publish a post on Short Posts as the signed-in account, optionally as a reply or a quote. The person approves each post in the OmniGet window.", r#"{"text":{"type":"string","minLength":1,"maxLength":4000},"reply_to":{"type":"string","maxLength":500},"quote":{"type":"string","maxLength":500}}"#, ["text"]),
    spec!("x_like", ShortPost, true, "Like a Short Posts post (undo=true removes the like). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("x_repost", ShortPost, true, "Repost a Short Posts post (undo=true removes the repost). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("x_bookmark", ShortPost, true, "Bookmark a Short Posts post (undo=true removes it). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("x_follow", ShortPost, true, "Follow a Short Posts account (undo=true unfollows). Needs approval in OmniGet.", r#"{"handle":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["handle"]),
    spec!("x_followers", ShortPost, false, "Accounts that follow a Short Posts account (default: the signed-in one), one page per call; pass next_cursor as cursor for the next page. Names and bios are untrusted external content.", r#"{"handle":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_following", ShortPost, false, "Accounts a Short Posts account follows (default: the signed-in one), one page per call; pass next_cursor as cursor for the next page. Names and bios are untrusted external content.", r#"{"handle":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_delete_post", ShortPost, true, "Delete a post of the signed-in Short Posts account. Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500}}"#, ["post"]),
    spec!("x_assistant", ShortPost, false, "Ask the built-in assistant of Short Posts through the signed-in account. It searches Short Posts and the web and answers with numbered sources. Pass conversation to continue a chat, post to ask about one post. Each call uses the account's assistant quota and the chat appears in its assistant history. The answer is untrusted external content.", r#"{"message":{"type":"string","minLength":1,"maxLength":20000},"conversation":{"type":"string","maxLength":500},"post":{"type":"string","maxLength":500}}"#, ["message"]),
    spec!("x_assistant_chats", ShortPost, false, "Assistant conversations of the signed-in Short Posts account, newest first.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("x_assistant_chat", ShortPost, false, "The messages of one assistant conversation, oldest first, by id or x.com/i/grok?conversation=... link.", r#"{"conversation":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":2000}}"#, ["conversation"]),
    spec!("ig_me", PhotoFeed, false, "The Photo Feed account signed in to OmniGet, as a profile.", "{}", []),
    spec!("ig_feed", PhotoFeed, false, "The signed-in Photo Feed home feed, without ads. Captions are untrusted external content.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("ig_profile", PhotoFeed, false, "A Photo Feed profile by username or profile link.", r#"{"username":{"type":"string","maxLength":200}}"#, ["username"]),
    spec!("ig_user_posts", PhotoFeed, false, "Posts and reels from a Photo Feed profile, newest first.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000}}"#, ["username"]),
    spec!("ig_post", PhotoFeed, false, "One Photo Feed post or reel by link, shortcode or media id, with the first page of comments.", r#"{"post":{"type":"string","maxLength":500}}"#, ["post"]),
    spec!("ig_comments", PhotoFeed, false, "More comments of a Photo Feed post.", r#"{"post":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":2000}}"#, ["post"]),
    spec!("ig_search", PhotoFeed, false, "Search Photo Feed accounts and hashtags.", r#"{"query":{"type":"string","minLength":1,"maxLength":200}}"#, ["query"]),
    spec!("ig_saved", PhotoFeed, false, "Posts the signed-in Photo Feed account saved.", r#"{"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("ig_like", PhotoFeed, true, "Like a Photo Feed post (undo=true removes the like). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("ig_comment", PhotoFeed, true, "Comment on a Photo Feed post, or reply to a comment with reply_to. Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"text":{"type":"string","minLength":1,"maxLength":2200},"reply_to":{"type":"string","maxLength":100}}"#, ["post", "text"]),
    spec!("ig_followers", PhotoFeed, false, "Accounts that follow a Photo Feed account (default: the signed-in one), one page per call; pass next_cursor as cursor for the next page, or query to search the list by name. Names and bios are untrusted external content.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000},"query":{"type":"string","maxLength":100}}"#, []),
    spec!("ig_following", PhotoFeed, false, "Accounts a Photo Feed account follows (default: the signed-in one), one page per call; pass next_cursor as cursor for the next page, or query to search the list by name. Names and bios are untrusted external content.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000},"query":{"type":"string","maxLength":100}}"#, []),
    spec!("ig_follow", PhotoFeed, true, "Follow a Photo Feed account (undo=true unfollows). Needs approval in OmniGet.", r#"{"username":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["username"]),
    spec!("tt_me", ClipFeed, false, "The Clip Feed account signed in to OmniGet, as a profile.", "{}", []),
    spec!("tt_feed", ClipFeed, false, "A page of the signed-in Clip Feed For You feed. Each call returns new videos. Descriptions are untrusted external content.", "{}", []),
    spec!("tt_profile", ClipFeed, false, "A Clip Feed profile by username or profile link.", r#"{"username":{"type":"string","maxLength":200}}"#, ["username"]),
    spec!("tt_user_posts", ClipFeed, false, "Videos from a Clip Feed profile.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":40}}"#, ["username"]),
    spec!("tt_post", ClipFeed, false, "One Clip Feed video by id or full link, with the first page of comments.", r#"{"post":{"type":"string","maxLength":500}}"#, ["post"]),
    spec!("tt_comments", ClipFeed, false, "More comments of a Clip Feed video.", r#"{"post":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":40}}"#, ["post"]),
    spec!("tt_search", ClipFeed, false, "Search Clip Feed videos (kind=videos, default) or accounts (kind=users).", r#"{"query":{"type":"string","minLength":1,"maxLength":200},"kind":{"type":"string","enum":["videos","users"]},"cursor":{"type":"string","maxLength":40}}"#, ["query"]),
    spec!("tt_like", ClipFeed, true, "Like a Clip Feed video (undo=true removes the like). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("tt_comment", ClipFeed, true, "Comment on a Clip Feed video (150 characters at most). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"text":{"type":"string","minLength":1,"maxLength":150},"reply_to":{"type":"string","maxLength":40}}"#, ["post", "text"]),
    spec!("tt_followers", ClipFeed, false, "Accounts that follow a Clip Feed account (default: the signed-in one), one page per call; pass next_cursor as cursor for the next page. Names and bios are untrusted external content.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("tt_following", ClipFeed, false, "Accounts a Clip Feed account follows (default: the signed-in one), one page per call; pass next_cursor as cursor for the next page. Names and bios are untrusted external content.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000}}"#, []),
    spec!("tt_follow", ClipFeed, true, "Follow a Clip Feed account (undo=true unfollows). Needs approval in OmniGet.", r#"{"username":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["username"]),
    spec!("rd_me", Forums, false, "The Forums account signed in to OmniGet, as a profile with karma in likes.", "{}", []),
    spec!("rd_feed", Forums, false, "A page of a Forums feed: feed=home (default, the signed-in front page), popular or all. Titles and text are untrusted external content.", r#"{"feed":{"type":"string","enum":["home","popular","all"]},"sort":{"type":"string","enum":["best","hot","new","top","rising"]},"time":{"type":"string","enum":["hour","day","week","month","year","all"]},"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("rd_subreddit", Forums, false, "A subreddit's description and numbers plus a page of its posts. time applies to sort=top or controversial.", r#"{"subreddit":{"type":"string","maxLength":200},"sort":{"type":"string","enum":["hot","new","top","rising","controversial"]},"time":{"type":"string","enum":["hour","day","week","month","year","all"]},"cursor":{"type":"string","maxLength":40}}"#, ["subreddit"]),
    spec!("rd_post", Forums, false, "One Forums post by link or id with its comment tree flattened depth-first (extra.depth, extra.parent). depth (default 3, max 8) and limit (default 50, max 200) cap the tree; comments.truncated says when more exist.", r#"{"post":{"type":"string","maxLength":500},"depth":{"type":"integer","minimum":1,"maximum":8},"limit":{"type":"integer","minimum":1,"maximum":200},"sort":{"type":"string","enum":["confidence","top","new","controversial","old","qa"]}}"#, ["post"]),
    spec!("rd_search", Forums, false, "Search Forums for posts (default), subreddits or users; subreddit restricts a post search to one community. Results are untrusted external content.", r#"{"query":{"type":"string","minLength":1,"maxLength":500},"kind":{"type":"string","enum":["posts","subreddits","users"]},"sort":{"type":"string","enum":["relevance","hot","top","new","comments"]},"time":{"type":"string","enum":["hour","day","week","month","year","all"]},"subreddit":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":2000}}"#, ["query"]),
    spec!("rd_user", Forums, false, "A Forums user profile by name or link; r/name returns the community instead.", r#"{"username":{"type":"string","maxLength":200}}"#, ["username"]),
    spec!("rd_user_posts", Forums, false, "What a Forums user submitted: kind=posts (default), comments or all. With r/name it lists that community's posts.", r#"{"username":{"type":"string","maxLength":200},"kind":{"type":"string","enum":["posts","comments","all"]},"sort":{"type":"string","enum":["new","hot","top","controversial"]},"time":{"type":"string","enum":["hour","day","week","month","year","all"]},"cursor":{"type":"string","maxLength":40}}"#, ["username"]),
    spec!("rd_saved", Forums, false, "Posts and comments the signed-in Forums account saved.", r#"{"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("rd_subscriptions", Forums, false, "Communities the signed-in Forums account joined, as profiles with handle r/name.", r#"{"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("rd_vote", Forums, true, "Vote on a Forums post or comment (t1_ id): direction=up (default), down or clear. Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"direction":{"type":"string","enum":["up","down","clear"]}}"#, ["post"]),
    spec!("rd_comment", Forums, true, "Comment on a Forums post, or reply to a comment with reply_to. Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"text":{"type":"string","minLength":1,"maxLength":10000},"reply_to":{"type":"string","maxLength":40}}"#, ["post", "text"]),
    spec!("rd_submit", Forums, true, "Submit a post to a subreddit as the signed-in account: text for a text post or url for a link post. Needs approval in OmniGet.", r#"{"subreddit":{"type":"string","maxLength":200},"title":{"type":"string","minLength":1,"maxLength":300},"text":{"type":"string","maxLength":40000},"url":{"type":"string","maxLength":2000}}"#, ["subreddit", "title"]),
    spec!("rd_subscribe", Forums, true, "Join a subreddit (undo=true leaves it). Needs approval in OmniGet.", r#"{"subreddit":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["subreddit"]),
    spec!("rd_save", Forums, true, "Save a Forums post or comment (undo=true removes it from saved). Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("pin_me", PinBoard, false, "The Pin Boards account signed in to OmniGet, as a profile.", "{}", []),
    spec!("pin_feed", PinBoard, false, "A page of the signed-in Pin Boards home feed. Titles and descriptions are untrusted external content.", r#"{"cursor":{"type":"string","maxLength":6000}}"#, []),
    spec!("pin_search", PinBoard, false, "Search Pin Boards for pins (default), boards or users. Boards come back as profiles with handle username/board.", r#"{"query":{"type":"string","minLength":1,"maxLength":300},"kind":{"type":"string","enum":["pins","boards","users"]},"cursor":{"type":"string","maxLength":6000}}"#, ["query"]),
    spec!("pin_pin", PinBoard, false, "One pin by id or link, with the first page of comments.", r#"{"post":{"type":"string","maxLength":500}}"#, ["post"]),
    spec!("pin_user", PinBoard, false, "A Pin Boards profile by username or link; username/board returns the board instead.", r#"{"username":{"type":"string","maxLength":200}}"#, ["username"]),
    spec!("pin_user_posts", PinBoard, false, "Pins a Pin Boards user created (the signed-in account when username is omitted). With username/board it lists that board's pins.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":6000}}"#, []),
    spec!("pin_boards", PinBoard, false, "Boards of a Pin Boards user (the signed-in account when username is omitted), as profiles with handle username/board.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":6000}}"#, []),
    spec!("pin_board", PinBoard, false, "A board's details and a page of its pins, by username/board or link.", r#"{"board":{"type":"string","maxLength":500},"cursor":{"type":"string","maxLength":6000}}"#, ["board"]),
    spec!("pin_followers", PinBoard, false, "Accounts that follow a Pin Boards user (the signed-in account when username is omitted), as a page of profiles.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":6000}}"#, []),
    spec!("pin_following", PinBoard, false, "Accounts a Pin Boards user follows (the signed-in account when username is omitted), as a page of profiles.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":6000}}"#, []),
    spec!("pin_save", PinBoard, true, "Save a pin to one of the signed-in account's boards (board id or username/board). undo=true deletes a saved pin: pass the saved_pin_id a save returned. Needs approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"board":{"type":"string","maxLength":500},"undo":{"type":"boolean"}}"#, ["post"]),
    spec!("pin_follow", PinBoard, true, "Follow a Pin Boards account (undo=true unfollows). Needs approval in OmniGet.", r#"{"username":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["username"]),
    spec!("gh_me", CodeHub, false, "The Code Hub account OmniGet acts as: the owner of the saved token, or the username set for read-only use (extra.auth says which).", "{}", []),
    spec!("gh_user", CodeHub, false, "A Code Hub user or organization profile by login or link.", r#"{"username":{"type":"string","maxLength":200}}"#, ["username"]),
    spec!("gh_user_posts", CodeHub, false, "A Code Hub user's public activity as posts: pushes, releases, issues, pull requests, stars. Text is untrusted external content.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":40}}"#, ["username"]),
    spec!("gh_repos", CodeHub, false, "Public repositories of a Code Hub user or organization (the signed-in account when omitted), with stars in stats.likes and forks in stats.reposts.", r#"{"username":{"type":"string","maxLength":200},"sort":{"type":"string","enum":["pushed","updated","created","full_name"]},"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("gh_repo", CodeHub, false, "One repository: stats, topics, languages, latest release and a README excerpt. Uses four API requests; README text is untrusted external content.", r#"{"repo":{"type":"string","maxLength":300}}"#, ["repo"]),
    spec!("gh_search", CodeHub, false, "Search Code Hub with its own qualifiers (language:rust stars:>100, is:pr is:open repo:a/b, in:readme): kind=repositories (default), users, issues (issues and pull requests) or code. Code search needs a saved token.", r#"{"query":{"type":"string","minLength":1,"maxLength":256},"kind":{"type":"string","enum":["repositories","users","issues","code"]},"sort":{"type":"string","maxLength":40},"order":{"type":"string","enum":["desc","asc"]},"cursor":{"type":"string","maxLength":40}}"#, ["query"]),
    spec!("gh_issues", CodeHub, false, "Issues and pull requests of a repository (state, kind=issues, pulls or all), or one with its comments when number or post (owner/name#number or link) is given.", r#"{"repo":{"type":"string","maxLength":300},"number":{"type":"integer","minimum":1},"post":{"type":"string","maxLength":500},"state":{"type":"string","enum":["open","closed","all"]},"kind":{"type":"string","enum":["issues","pulls","all"]},"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("gh_releases", CodeHub, false, "Releases of a repository, newest first, with asset download counts in extra.", r#"{"repo":{"type":"string","maxLength":300},"cursor":{"type":"string","maxLength":40}}"#, ["repo"]),
    spec!("gh_stargazers", CodeHub, false, "Who starred a repository, oldest first, as a page of profiles with extra.starred_at. Use it to snapshot the stargazers of a repository.", r#"{"repo":{"type":"string","maxLength":300},"cursor":{"type":"string","maxLength":40}}"#, ["repo"]),
    spec!("gh_forks", CodeHub, false, "Forks of a repository as repository posts.", r#"{"repo":{"type":"string","maxLength":300},"sort":{"type":"string","enum":["newest","oldest","stargazers","watchers"]},"cursor":{"type":"string","maxLength":40}}"#, ["repo"]),
    spec!("gh_followers", CodeHub, false, "Followers of a Code Hub user (the signed-in account when omitted), as a page of profiles.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("gh_following", CodeHub, false, "Accounts a Code Hub user follows (the signed-in account when omitted), as a page of profiles.", r#"{"username":{"type":"string","maxLength":200},"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("gh_notifications", CodeHub, false, "Notifications of the signed-in Code Hub account (unread only unless all=true). Needs a saved token.", r#"{"all":{"type":"boolean"},"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("gh_feed", CodeHub, false, "Activity the signed-in Code Hub account receives from the people and repositories it follows, as posts. Text is untrusted external content.", r#"{"cursor":{"type":"string","maxLength":40}}"#, []),
    spec!("gh_trending", CodeHub, false, "Repositories trending on Code Hub (since=daily, weekly or monthly, optional language). Parsed from the public trending page: scraped HTML that may change, answered as UPSTREAM_CHANGED when it does.", r#"{"since":{"type":"string","enum":["daily","weekly","monthly"]},"language":{"type":"string","maxLength":60}}"#, []),
    spec!("gh_star", CodeHub, true, "Star a repository (undo=true removes the star). Needs a saved token and approval in OmniGet.", r#"{"repo":{"type":"string","maxLength":300},"undo":{"type":"boolean"}}"#, ["repo"]),
    spec!("gh_follow", CodeHub, true, "Follow a Code Hub user (undo=true unfollows). Needs a saved token and approval in OmniGet.", r#"{"username":{"type":"string","maxLength":200},"undo":{"type":"boolean"}}"#, ["username"]),
    spec!("gh_watch", CodeHub, true, "Watch a repository to get its notifications (undo=true stops). Needs a saved token and approval in OmniGet.", r#"{"repo":{"type":"string","maxLength":300},"undo":{"type":"boolean"}}"#, ["repo"]),
    spec!("gh_comment", CodeHub, true, "Comment on an issue or pull request given as owner/name#number or its link. Needs a saved token and approval in OmniGet.", r#"{"post":{"type":"string","maxLength":500},"text":{"type":"string","minLength":1,"maxLength":65000}}"#, ["post", "text"]),
    spec!("gh_issue_create", CodeHub, true, "Open an issue in a repository. Needs a saved token and approval in OmniGet.", r#"{"repo":{"type":"string","maxLength":300},"title":{"type":"string","minLength":1,"maxLength":256},"body":{"type":"string","maxLength":65000},"labels":{"type":"array","items":{"type":"string","maxLength":50},"maxItems":20}}"#, ["repo", "title"]),
];

pub const STATUS_DESCRIPTION: &str = "Which of Short Posts, Photo Feed, Clip Feed, Forums, Pin Boards and Code Hub are signed in to OmniGet. Call it first: the x_*, ig_*, tt_*, rd_* and pin_* tools act through the OmniGet browser with the person's own session, and gh_* uses the public Code Hub API with the token or username the person saved in the Code Hub screen (auth says which). A platform that is not signed in returns NOT_LOGGED_IN (ask the person to open that screen in OmniGet and sign in). Every write (post, like, repost, comment, follow, vote, save, star) waits for the person to approve it in the OmniGet window.";

pub fn all() -> impl Iterator<Item = &'static Spec> {
    SPECS
        .iter()
        .chain(super::audience::SPECS)
        .chain(super::insights::SPECS)
        .chain(super::monitor::SPECS)
        .chain(super::schedule::SPECS)
}

pub fn find(name: &str) -> Option<&'static Spec> {
    all().find(|s| s.name == name)
}

pub fn owns(name: &str) -> bool {
    name == "social_status" || find(name).is_some()
}

pub fn input_schema(spec: &Spec) -> Value {
    let props: Value = serde_json::from_str(spec.props).expect("static tool schema");
    json!({ "type": "object", "properties": props, "required": spec.required, "additionalProperties": false })
}

fn verb(tool: &str, undo: bool) -> &'static str {
    match (tool.split_once('_').map(|x| x.1).unwrap_or(tool), undo) {
        ("create_post", _) => "Publish a post",
        ("like", false) => "Like",
        ("like", true) => "Remove the like from",
        ("repost", false) => "Repost",
        ("repost", true) => "Undo the repost of",
        ("bookmark", false) => "Bookmark",
        ("bookmark", true) => "Remove the bookmark from",
        ("follow", false) => "Follow",
        ("follow", true) => "Unfollow",
        ("delete_post", _) => "Delete",
        ("comment", _) => "Comment on",
        ("schedule", _) => "Change the posting schedule",
        ("feeds", _) => "Change the auto-post feeds",
        ("vote", _) => "Vote on",
        ("submit", _) => "Submit a post to",
        ("subscribe", false) => "Join",
        ("subscribe", true) => "Leave",
        ("save", false) => "Save",
        ("save", true) => "Remove the saved",
        ("star", false) => "Star",
        ("star", true) => "Remove the star from",
        ("watch", false) => "Watch",
        ("watch", true) => "Stop watching",
        ("issue_create", _) => "Open an issue in",
        _ => "Act on",
    }
}

pub fn describe(spec: &Spec, a: &Value) -> (String, Value) {
    let undo = a["undo"].as_bool().unwrap_or(false);
    let target = [
        "post",
        "handle",
        "username",
        "repo",
        "subreddit",
        "reply_to",
    ]
    .iter()
    .find_map(|k| a[*k].as_str().filter(|s| !s.trim().is_empty()))
    .unwrap_or_default();
    let summary = match spec.name {
        "x_create_post" if a["reply_to"].is_string() => {
            format!("Reply on {} to {target}", spec.platform.label())
        }
        "x_create_post" => format!("Publish a post on {}", spec.platform.label()),
        _ => format!(
            "{} {target} on {}",
            verb(spec.name, undo),
            spec.platform.label()
        )
        .replace("  ", " "),
    };
    let mut details = serde_json::Map::new();
    if let Some(o) = a.as_object() {
        for (k, v) in o {
            if !v.is_null() {
                details.insert(k.clone(), v.clone());
            }
        }
    }
    (summary, Value::Object(details))
}

pub fn output_schema(name: &str) -> Option<Value> {
    let t = |k: &str| json!({ "type": k });
    let paged = json!({ "type": "object", "properties": { "count": t("integer"), "items": t("array"), "next_cursor": { "type": ["string", "null"] } } });
    if name == "social_status" {
        return Some(json!({ "type": "object", "properties": { "platforms": t("array") } }));
    }
    let spec = find(name)?;
    if spec.route != Route::Platform {
        return Some(t("object"));
    }
    Some(
        match spec.name.split_once('_').map(|x| x.1).unwrap_or_default() {
            "me" | "profile" | "user" => {
                json!({ "type": "object", "properties": { "id": t("string"), "handle": t("string"), "name": t("string"), "url": t("string") } })
            }
            "assistant" => {
                json!({ "type": "object", "properties": { "conversation_id": t("string"), "url": t("string"), "text": t("string"), "sources": t("array"), "follow_ups": t("array") } })
            }
            "repo" | "subreddit" | "board" | "issues" => t("object"),
            "post" | "pin" => {
                json!({ "type": "object", "properties": { "post": { "type": ["object", "null"] }, "replies": t("object"), "comments": t("object") } })
            }
            "search" if spec.platform == Platform::PhotoFeed => {
                json!({ "type": "object", "properties": { "users": t("array"), "hashtags": t("array") } })
            }
            _ if spec.write => json!({ "type": "object", "properties": { "ok": t("boolean") } }),
            _ => paged,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_has_a_valid_schema_and_a_unique_name() {
        let mut seen = std::collections::HashSet::new();
        for s in SPECS {
            assert!(seen.insert(s.name), "duplicate {}", s.name);
            assert!(s.name.starts_with(s.platform.prefix()), "{}", s.name);
            let schema = input_schema(s);
            for r in s.required {
                assert!(
                    schema["properties"][*r].is_object(),
                    "{} requires missing {r}",
                    s.name
                );
            }
            assert!(output_schema(s.name).is_some());
        }
    }

    #[test]
    fn describes_writes_for_the_approval_window() {
        let (sum, details) = describe(
            find("x_like").unwrap(),
            &json!({ "post": "https://x.com/a/status/1", "undo": true }),
        );
        assert_eq!(
            sum,
            "Remove the like from https://x.com/a/status/1 on Short Posts"
        );
        assert_eq!(details["undo"], true);
        let (sum, _) = describe(
            find("x_create_post").unwrap(),
            &json!({ "text": "oi", "reply_to": "1" }),
        );
        assert_eq!(sum, "Reply on Short Posts to 1");
    }

    #[test]
    fn the_new_networks_have_their_core_tools_and_write_flags() {
        for (prefix, platform) in [
            ("rd_", Platform::Forums),
            ("pin_", Platform::PinBoard),
            ("gh_", Platform::CodeHub),
        ] {
            for name in ["me", "feed", "search", "user", "user_posts"] {
                let spec = find(&format!("{prefix}{name}")).expect(name);
                assert_eq!(spec.platform, platform);
                assert!(!spec.write, "{prefix}{name}");
                assert_eq!(spec.route, Route::Platform);
            }
        }
        for name in [
            "rd_vote",
            "rd_comment",
            "rd_submit",
            "rd_subscribe",
            "rd_save",
            "pin_save",
            "pin_follow",
            "gh_star",
            "gh_follow",
            "gh_watch",
            "gh_comment",
            "gh_issue_create",
        ] {
            assert!(find(name).expect(name).write, "{name}");
        }
        for name in [
            "pin_followers",
            "pin_following",
            "gh_followers",
            "gh_following",
            "gh_stargazers",
        ] {
            assert!(!find(name).expect(name).write, "{name}");
        }
        assert!(SPECS.iter().all(|s| !s.props.contains("token")));
    }

    #[test]
    fn describes_writes_on_the_new_networks() {
        let (sum, _) = describe(
            find("gh_star").unwrap(),
            &json!({ "repo": "a/b", "undo": true }),
        );
        assert_eq!(sum, "Remove the star from a/b on Code Hub");
        let (sum, _) = describe(
            find("rd_subscribe").unwrap(),
            &json!({ "subreddit": "r/rust" }),
        );
        assert_eq!(sum, "Join r/rust on Forums");
        let (sum, _) = describe(
            find("pin_save").unwrap(),
            &json!({ "post": "123", "board": "ana/home" }),
        );
        assert_eq!(sum, "Save 123 on Pin Boards");
    }
}
