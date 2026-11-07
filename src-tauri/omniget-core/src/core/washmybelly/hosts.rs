use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::elevate::ElevatedOp;
use super::plan::{self, Op, Outcome, PlanItem, Source};
use super::receipt::Undo;
use super::store::Ctx;
use super::{Os, Risk};

pub const BEGIN: &str = "# >>> OmniGet block list >>>";
pub const END: &str = "# <<< OmniGet block list <<<";

const FOREIGN_TAG: &str = "managed by privacy.sexy";

const MAX_BYTES: u64 = 16 * 1024 * 1024;

const ACTIONS: &[&str] = &["write"];

pub struct BlockList {
    pub id: &'static str,
    pub name: &'static str,
    pub license: &'static str,
    pub source_url: &'static str,
    pub default_on: &'static [Os],
    text: &'static str,
}

pub const LISTS: &[BlockList] = &[BlockList {
    id: "windowsspyblocker",
    name: "WindowsSpyBlocker",
    license: "MIT",
    source_url: "https://github.com/crazy-max/WindowsSpyBlocker",
    default_on: &[Os::Windows],
    text: include_str!("../../../data/washmybelly/hosts/windowsspyblocker/spy.txt"),
}];

fn list_by_id(id: &str) -> Option<&'static BlockList> {
    LISTS.iter().find(|l| l.id == id)
}

fn list_domains(id: &str) -> Option<&'static [String]> {
    static PARSED: OnceLock<Vec<Vec<String>>> = OnceLock::new();
    let all = PARSED.get_or_init(|| LISTS.iter().map(|l| parse_list(l.text)).collect());
    let i = LISTS.iter().position(|l| l.id == id)?;
    Some(&all[i])
}

fn default_lists() -> Vec<String> {
    let os = super::current_os();
    LISTS
        .iter()
        .filter(|l| l.default_on.is_empty() || l.default_on.contains(&os))
        .map(|l| l.id.to_string())
        .collect()
}

#[cfg(test)]
thread_local! {
    static TEST_HOSTS: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub fn hosts_path() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = TEST_HOSTS.with(|t| t.borrow().clone()) {
        return p;
    }
    system_hosts_path()
}

fn system_hosts_path() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        PathBuf::from(root)
            .join("System32")
            .join("drivers")
            .join("etc")
            .join("hosts")
    } else {
        PathBuf::from("/etc/hosts")
    }
}

pub fn is_writable(path: &Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

fn read_text(path: &Path) -> Result<String, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_BYTES => {
            return Err("the hosts file is too big to edit safely".into())
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(format!("cannot read the hosts file: {e}")),
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read the hosts file: {e}"))?;
    String::from_utf8(bytes)
        .map_err(|_| "the hosts file is not UTF-8 text; OmniGet leaves it alone".to_string())
}

fn is_reserved(domain: &str) -> bool {
    matches!(
        domain,
        "localhost"
            | "localhost.localdomain"
            | "local"
            | "broadcasthost"
            | "ip6-localhost"
            | "ip6-loopback"
            | "ip6-localnet"
            | "ip6-mcastprefix"
            | "ip6-allnodes"
            | "ip6-allrouters"
            | "ip6-allhosts"
            | "0.0.0.0"
    )
}

fn looks_like_ip(token: &str) -> bool {
    token.contains(':')
        || (token.split('.').count() == 4
            && token
                .split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
}

pub fn normalize_domain(raw: &str) -> Option<String> {
    let d = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.len() < 4 || d.len() > 253 || !d.contains('.') {
        return None;
    }
    if d.starts_with('.') || d.starts_with('-') || d.ends_with('-') {
        return None;
    }
    if !d
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return None;
    }
    if d.split('.')
        .any(|label| label.is_empty() || label.len() > 63)
    {
        return None;
    }
    if looks_like_ip(&d) || is_reserved(&d) {
        return None;
    }
    Some(d)
}

pub fn parse_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut tokens = line.split_whitespace();
        let Some(first) = tokens.next() else { continue };
        if looks_like_ip(first) {
            out.extend(tokens.filter_map(normalize_domain));
        } else if let Some(d) = normalize_domain(first) {
            out.push(d);
        }
    }
    out
}

pub fn whitelisted(domain: &str, whitelist: &[String]) -> bool {
    whitelist.iter().any(|w| {
        let w = w.trim().trim_end_matches('.').to_ascii_lowercase();
        !w.is_empty() && (domain == w || domain.ends_with(&format!(".{w}")))
    })
}

pub fn merge(lists: &[&[String]], whitelist: &[String]) -> (Vec<String>, usize) {
    let mut set = BTreeSet::new();
    let mut skipped = BTreeSet::new();
    for list in lists {
        for d in list.iter() {
            if whitelisted(d, whitelist) {
                skipped.insert(d.as_str());
            } else {
                set.insert(d.clone());
            }
        }
    }
    (set.into_iter().collect(), skipped.len())
}

fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn block_span(text: &str) -> Option<(usize, usize)> {
    let mut begin: Option<usize> = None;
    let mut idx = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if begin.is_none() {
            if trimmed == BEGIN {
                begin = Some(idx);
            }
        } else if trimmed == END {
            return Some((begin?, idx + line.len()));
        }
        idx += line.len();
    }
    begin.map(|b| (b, text.len()))
}

pub fn has_block(text: &str) -> bool {
    block_span(text).is_some()
}

pub fn block_body(text: &str) -> Option<&str> {
    let (start, end) = block_span(text)?;
    let inner = &text[start..end];
    let after_begin = inner.find('\n').map_or(inner.len(), |i| i + 1);
    let body = &inner[after_begin..];
    let before_end = match body.rfind(END) {
        Some(i) => body[..i].rfind('\n').map_or(0, |j| j + 1),
        None => body.len(),
    };
    Some(&body[..before_end])
}

pub fn blocked_domains(text: &str) -> Vec<String> {
    block_body(text).map(parse_list).unwrap_or_default()
}

pub fn count_blocked(text: &str) -> usize {
    block_body(text).map(|b| parse_list(b).len()).unwrap_or(0)
}

pub fn block_updated_at(text: &str) -> Option<String> {
    block_body(text)?.lines().find_map(|l| {
        let l = l.trim();
        l.strip_prefix("# updated ")
            .or_else(|| l.strip_prefix("# atualizado em "))
            .map(|s| s.trim().to_string())
    })
}

pub fn render_block(domains: &[String], lists: &[&str], updated_at: &str, nl: &str) -> String {
    let mut s = String::with_capacity(domains.len() * 32 + 256);
    for line in [
        BEGIN.to_string(),
        "# written by OmniGet (washmybelly); do not edit inside this block".to_string(),
        format!("# updated {updated_at}"),
        format!("# lists: {}", lists.join(", ")),
        format!("# {} domains", domains.len()),
    ] {
        s.push_str(&line);
        s.push_str(nl);
    }
    for d in domains {
        s.push_str("0.0.0.0 ");
        s.push_str(d);
        s.push_str(nl);
    }
    s.push_str(END);
    s.push_str(nl);
    s
}

pub fn apply_to_text(
    current: &str,
    domains: &[String],
    lists: &[&str],
    updated_at: &str,
) -> String {
    let nl = newline_of(current);
    let block = render_block(domains, lists, updated_at, nl);
    match block_span(current) {
        Some((start, end)) => format!("{}{}{}", &current[..start], block, &current[end..]),
        None => {
            let mut out = String::with_capacity(current.len() + block.len() + 2);
            out.push_str(current);
            if !current.is_empty() && !current.ends_with('\n') {
                out.push_str(nl);
            }
            out.push_str(&block);
            out
        }
    }
}

pub fn remove_from_text(current: &str) -> String {
    match block_span(current) {
        Some((start, end)) => format!("{}{}", &current[..start], &current[end..]),
        None => current.to_string(),
    }
}

pub fn check_replacement(current: &str, next: &str) -> Result<(), String> {
    if next.len() as u64 > MAX_BYTES {
        return Err("the new hosts file is too big".into());
    }
    let outside = |t: &str| {
        remove_from_text(t)
            .trim_end_matches(['\r', '\n'])
            .to_string()
    };
    if outside(current) != outside(next) {
        return Err("HOSTS_CHANGED: the hosts file changed outside the OmniGet block since the preview; preview again".into());
    }
    if next.matches(BEGIN).count() > 1 || next.matches(END).count() > 1 {
        return Err("the new hosts file has more than one OmniGet block".into());
    }
    if let Some(body) = block_body(next) {
        for line in body.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let mut tok = l.split_whitespace();
            let ok = matches!(tok.next(), Some("0.0.0.0") | Some("::"))
                && tok.next().and_then(normalize_domain).is_some()
                && tok.next().is_none();
            if !ok {
                return Err(format!("unexpected line inside the OmniGet block: {l}"));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "default_lists")]
    pub lists: Vec<String>,
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default)]
    pub extra: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            lists: default_lists(),
            whitelist: Vec::new(),
            extra: Vec::new(),
        }
    }
}

const PREFS_KEY: &str = "hosts_prefs";

fn prefs_from(a: &Value, saved: Prefs) -> Prefs {
    let pick = |k: &str, cur: Vec<String>| match a.get(k).and_then(Value::as_array) {
        Some(v) => v
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => cur,
    };
    Prefs {
        lists: pick("lists", saved.lists)
            .into_iter()
            .filter(|l| list_by_id(l).is_some())
            .collect(),
        whitelist: pick("whitelist", saved.whitelist),
        extra: pick("extra", saved.extra),
    }
}

pub struct Built {
    pub current: String,
    pub next: String,
    pub domains: Vec<String>,
    pub skipped_whitelist: usize,
    pub before: Vec<String>,
}

pub fn build_apply(current: &str, prefs: &Prefs, updated_at: &str) -> Built {
    let extra: Vec<String> = prefs
        .extra
        .iter()
        .filter_map(|d| normalize_domain(d))
        .collect();
    let mut lists: Vec<&[String]> = prefs
        .lists
        .iter()
        .filter_map(|id| list_domains(id))
        .collect();
    lists.push(&extra);
    let (domains, skipped) = merge(&lists, &prefs.whitelist);
    let before = blocked_domains(current);
    let mut labels: Vec<&str> = prefs
        .lists
        .iter()
        .filter_map(|id| list_by_id(id))
        .map(|l| l.name)
        .collect();
    if !extra.is_empty() {
        labels.push("your own");
    }
    let next = if has_block(current) && before == domains {
        current.to_string()
    } else if domains.is_empty() {
        remove_from_text(current)
    } else {
        apply_to_text(current, &domains, &labels, updated_at)
    };
    Built {
        current: current.to_string(),
        next,
        domains,
        skipped_whitelist: skipped,
        before,
    }
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn staging_dir(ctx: &Ctx) -> PathBuf {
    ctx.dir.join("hosts")
}

fn hash_of(text: &str) -> String {
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(text.as_bytes()))
}

fn staged_name_ok(name: &str) -> Option<&str> {
    let h = name.strip_prefix("staged-")?.strip_suffix(".hosts")?;
    (h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
}

pub fn stage(ctx: &Ctx, text: &str) -> Result<(PathBuf, String), String> {
    let dir = staging_dir(ctx);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let sha = hash_of(text);
    let path = dir.join(format!("staged-{sha}.hosts"));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
    }
    Ok((path, sha))
}

fn load_staged(ctx: &Ctx, args: &Value) -> Result<(PathBuf, String), String> {
    let staged = PathBuf::from(
        args.get("staged")
            .and_then(Value::as_str)
            .ok_or("staged is missing")?,
    );
    let sha = args.get("sha").and_then(Value::as_str).unwrap_or_default();
    let name = staged
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if staged_name_ok(name) != Some(sha) {
        return Err("the staged hosts file does not match the plan".into());
    }
    let parent = staged
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or("the staged hosts file is gone; preview again")?;
    let mut roots = vec![staging_dir(ctx)];
    if let Ok(d) = super::data_dir() {
        roots.push(d.join("hosts"));
    }
    if !roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .any(|r| r == parent)
    {
        return Err("the staged hosts file is outside the OmniGet staging folder".into());
    }
    let meta = std::fs::symlink_metadata(&staged)
        .map_err(|_| "the staged hosts file is gone; preview again")?;
    if !meta.file_type().is_file() {
        return Err("the staged hosts file is not a regular file".into());
    }
    let text = read_text(&staged)?;
    if hash_of(&text) != sha {
        return Err("the staged hosts file changed after the preview; preview again".into());
    }
    Ok((staged, text))
}

fn dns_flush_argv() -> Vec<Vec<&'static str>> {
    match super::current_os() {
        Os::Macos => vec![
            vec!["/usr/bin/dscacheutil", "-flushcache"],
            vec!["/usr/bin/killall", "-HUP", "mDNSResponder"],
        ],
        Os::Windows => vec![vec!["ipconfig", "/flushdns"]],
        Os::Linux => vec![vec!["resolvectl", "flush-caches"]],
    }
}

pub fn dns_flush_commands() -> Vec<String> {
    let sudo = !cfg!(windows);
    dns_flush_argv()
        .into_iter()
        .map(|argv| {
            let prog = argv[0].rsplit('/').next().unwrap_or(argv[0]);
            let line = std::iter::once(prog)
                .chain(argv[1..].iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            if sudo {
                format!("sudo {line}")
            } else {
                line
            }
        })
        .collect()
}

fn flush_dns() -> &'static str {
    if cfg!(test) {
        return "DNS cache flush skipped in tests";
    }
    let mut ok = true;
    for argv in dns_flush_argv() {
        let run = crate::core::process::std_command(argv[0])
            .args(&argv[1..])
            .output();
        ok &= matches!(run, Ok(o) if o.status.success());
    }
    if ok {
        "DNS cache flushed"
    } else {
        "the DNS cache clears by itself in a few minutes"
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    text: String,
}

fn check_action(action: &str) -> Result<(), String> {
    if ACTIONS.contains(&action) {
        Ok(())
    } else {
        Err(format!("hosts: unknown action {action}"))
    }
}

fn snapshot(ctx: &Ctx, receipt_id: &str, current: &str) -> Result<Undo, String> {
    let id = ctx.snapshot_put(
        receipt_id,
        "hosts",
        "hosts",
        &Snapshot {
            text: current.to_string(),
        },
    )?;
    Ok(Undo::Module {
        module: "hosts".into(),
        action: "restore_snapshot".into(),
        args: json!({ "snapshot": id }),
    })
}

pub fn plan_execute(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<Outcome, String> {
    check_action(action)?;
    let (_, next) = load_staged(ctx, args)?;
    let target = hosts_path();
    let current = read_text(&target)?;
    check_replacement(&current, &next)?;
    if target.exists() && !is_writable(&target) {
        return Err("NEEDS_ADMIN: the hosts file needs an administrator; preview again".into());
    }
    let undo = snapshot(ctx, receipt_id, &current)?;
    std::fs::write(&target, &next).map_err(|e| format!("cannot write the hosts file: {e}"))?;
    let n = count_blocked(&next);
    Ok(Outcome {
        detail: format!("{n} domains blocked · {}", flush_dns()),
        bytes: 0,
        undo: Some(undo),
    })
}

pub fn plan_elevated(
    ctx: &Ctx,
    receipt_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Vec<ElevatedOp>, Option<Undo>), String> {
    check_action(action)?;
    let (staged, next) = load_staged(ctx, args)?;
    let target = hosts_path();
    let current = read_text(&target)?;
    check_replacement(&current, &next)?;
    let undo = snapshot(ctx, receipt_id, &current)?;
    Ok((vec![ElevatedOp::WriteHosts { staged }], Some(undo)))
}

pub fn plan_undo(ctx: &Ctx, action: &str, args: &Value) -> Result<String, String> {
    if action != "restore_snapshot" {
        return Err(format!("hosts: cannot undo {action}"));
    }
    let id = args
        .get("snapshot")
        .and_then(Value::as_str)
        .ok_or("snapshot is missing")?;
    let snap: Snapshot = ctx.snapshot_get(id)?;
    let target = hosts_path();
    let current = read_text(&target)?;
    check_replacement(&current, &snap.text)?;
    if target.exists() && !is_writable(&target) {
        let (staged, _) = stage(ctx, &snap.text)?;
        let mut cmds = super::elevate::copy_command(&[ElevatedOp::WriteHosts { staged }]);
        cmds.extend(dns_flush_commands());
        return Err(format!(
            "NEEDS_ADMIN: putting the hosts file back needs an administrator. Use Privacy > Trackers > Remove, or run: {}",
            cmds.join(" && ")
        ));
    }
    std::fs::write(&target, &snap.text).map_err(|e| format!("cannot write the hosts file: {e}"))?;
    Ok(format!("hosts file restored · {}", flush_dns()))
}

pub fn elevated_write(staged: &Path) -> Result<String, String> {
    let target = system_hosts_path();
    let detail = elevated_write_to(staged, &target)?;
    Ok(format!("{detail} · {}", flush_dns()))
}

fn elevated_write_to(staged: &Path, target: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(staged).map_err(|e| format!("staged file: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("REFUSED: the staged hosts file is not a regular file".into());
    }
    let staged = staged.canonicalize().map_err(|e| e.to_string())?;
    let name = staged
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let sha = staged_name_ok(name)
        .ok_or("REFUSED: unexpected staged file name")?
        .to_string();
    let hosts_dir = staged.parent().ok_or("REFUSED: no parent")?;
    let wmb = hosts_dir.parent().ok_or("REFUSED: no data folder")?;
    let named = |p: &Path, n: &str| p.file_name().and_then(|x| x.to_str()) == Some(n);
    if !named(hosts_dir, "hosts") || !named(wmb, "washmybelly") {
        return Err("REFUSED: the staged file is not in <data>/washmybelly/hosts".into());
    }
    let key_ok =
        std::fs::symlink_metadata(wmb.join("elevate.key")).is_ok_and(|m| m.file_type().is_file());
    if !key_ok {
        return Err("REFUSED: the staged file is not in the OmniGet data folder".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&staged as &Path, hosts_dir] {
            let mode = std::fs::metadata(p)
                .map_err(|e| e.to_string())?
                .permissions()
                .mode();
            if mode & 0o002 != 0 {
                return Err(
                    "REFUSED: the staged file or its folder is writable by everyone".into(),
                );
            }
        }
    }
    let next = read_text(&staged)?;
    if hash_of(&next) != sha {
        return Err("REFUSED: the staged file changed after the plan".into());
    }
    let current = read_text(target)?;
    check_replacement(&current, &next)?;
    std::fs::write(target, &next).map_err(|e| format!("cannot write the hosts file: {e}"))?;
    Ok(format!(
        "hosts file written ({} domains blocked)",
        count_blocked(&next)
    ))
}

fn saved_prefs(ctx: &Ctx) -> Prefs {
    ctx.kv_get(PREFS_KEY).unwrap_or_default()
}

fn lists_json() -> Vec<Value> {
    LISTS
        .iter()
        .map(|l| {
            json!({
                "id": l.id, "name": l.name, "license": l.license, "source_url": l.source_url,
                "domains": list_domains(l.id).map_or(0, |d| d.len()),
            })
        })
        .collect()
}

fn status(ctx: &Ctx) -> Result<Value, String> {
    let path = hosts_path();
    let exists = path.exists();
    let text = read_text(&path)?;
    let outside = remove_from_text(&text);
    let user: Vec<&str> = outside.lines().filter(|l| !l.trim().is_empty()).collect();
    Ok(json!({
        "path": path,
        "exists": exists,
        "writable": exists && is_writable(&path),
        "bytes": text.len(),
        "block_present": has_block(&text),
        "blocked": count_blocked(&text),
        "updated_at": block_updated_at(&text),
        "user_lines": user.len(),
        "user_preview": user.iter().take(40).collect::<Vec<_>>(),
        "foreign_lines": text.lines().filter(|l| l.contains(FOREIGN_TAG)).count(),
        "lists": lists_json(),
        "prefs": saved_prefs(ctx),
        "flush_commands": dns_flush_commands(),
    }))
}

fn preview_lines(next: &str, max_block: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut shown = 0usize;
    let mut hidden = 0usize;
    for line in next.lines() {
        let t = line.trim();
        if t == BEGIN {
            in_block = true;
        } else if t == END {
            in_block = false;
        } else if in_block && !t.starts_with('#') {
            if shown >= max_block {
                hidden += 1;
                continue;
            }
            shown += 1;
        }
        out.push(line.to_string());
    }
    (out, hidden)
}

fn describe(b: &Built) -> Value {
    let before: BTreeSet<&String> = b.before.iter().collect();
    let after: BTreeSet<&String> = b.domains.iter().collect();
    let (lines, hidden) = preview_lines(&b.next, 60);
    json!({
        "unchanged": b.current == b.next,
        "domains": count_blocked(&b.next),
        "blocked_before": b.before.len(),
        "added": after.difference(&before).count(),
        "removed": before.difference(&after).count(),
        "skipped_whitelist": b.skipped_whitelist,
        "bytes": b.next.len(),
        "preview": lines,
        "preview_hidden": hidden,
    })
}

fn build_for(ctx: &Ctx, a: &Value, mode: &str) -> Result<(Built, Prefs), String> {
    let current = read_text(&hosts_path())?;
    let prefs = prefs_from(a, saved_prefs(ctx));
    let built = match mode {
        "apply" => build_apply(&current, &prefs, &now_stamp()),
        "remove" => Built {
            next: remove_from_text(&current),
            before: blocked_domains(&current),
            current,
            domains: Vec::new(),
            skipped_whitelist: 0,
        },
        _ => return Err(format!("hosts: unknown mode {mode}")),
    };
    Ok((built, prefs))
}

fn plan_for(ctx: &Ctx, a: &Value) -> Result<Value, String> {
    let mode = a
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("apply")
        .to_string();
    let (built, prefs) = build_for(ctx, a, &mode)?;
    if mode == "apply" {
        ctx.kv_set(PREFS_KEY, &prefs)?;
    }
    let mut out = describe(&built);
    if built.current == built.next {
        out["plan_id"] = Value::Null;
        out["item_ids"] = json!([]);
        return Ok(out);
    }
    check_replacement(&built.current, &built.next)?;
    let (staged, sha) = stage(ctx, &built.next)?;
    let target = hosts_path();
    let admin = target.exists() && !is_writable(&target);
    let n = count_blocked(&built.next);
    let (label, group) = if mode == "remove" || n == 0 {
        (
            "Remove the OmniGet tracker block from the hosts file".to_string(),
            "remove",
        )
    } else {
        (
            format!("Block {n} tracker domains in the hosts file"),
            "apply",
        )
    };
    let item = PlanItem {
        id: format!("hosts:write:{sha}"),
        label,
        detail: Some(target.to_string_lossy().to_string()),
        group: group.into(),
        bytes: 0,
        risk: Risk::Reversible,
        admin,
        op: Op::Module {
            module: "hosts".into(),
            action: "write".into(),
            args: json!({ "staged": staged, "sha": sha, "mode": mode }),
        },
    };
    let ids = vec![item.id.clone()];
    let p = plan::offer(ctx, "hosts", vec![item], Source::Ui)?;
    let mut commands = super::elevate::copy_command(&[ElevatedOp::WriteHosts {
        staged: staged.clone(),
    }]);
    commands.extend(dns_flush_commands());
    out["plan_id"] = json!(p.id);
    out["item_ids"] = json!(ids);
    out["admin"] = json!(admin);
    out["staged"] = json!(staged);
    out["commands"] = json!(commands);
    Ok(out)
}

pub async fn ui_action(action: &str, a: Value) -> Result<Value, String> {
    let action = action.to_string();
    super::tools::blocking(move || {
        let ctx = Ctx::app()?;
        match action.as_str() {
            "status" => status(&ctx),
            "preview" => {
                let mode = a
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("apply")
                    .to_string();
                let (built, _) = build_for(&ctx, &a, &mode)?;
                Ok(describe(&built))
            }
            "plan" => plan_for(&ctx, &a),
            _ => Err(format!("hosts: unknown screen action {action}")),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = include_str!("../../../tests/washmybelly_fixtures/hosts/user_hosts.txt");

    fn doms(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn with_hosts<R>(path: &Path, f: impl FnOnce() -> R) -> R {
        TEST_HOSTS.with(|t| *t.borrow_mut() = Some(path.to_path_buf()));
        let r = f();
        TEST_HOSTS.with(|t| *t.borrow_mut() = None);
        r
    }

    #[test]
    fn block_is_appended_and_user_text_opens_the_file() {
        let out = apply_to_text(
            USER,
            &doms(&["ads.example.com"]),
            &["AdAway"],
            "2026-01-01 10:00",
        );
        assert!(out.starts_with(USER));
        assert_eq!(count_blocked(&out), 1);
        assert_eq!(block_updated_at(&out).as_deref(), Some("2026-01-01 10:00"));
        check_replacement(USER, &out).unwrap();
    }

    #[test]
    fn reapplying_never_touches_the_user_text_nor_duplicates() {
        let first = apply_to_text(USER, &doms(&["a.example.com"]), &["x"], "t1");
        let edited = format!("# new line\n{first}10.0.0.9 other.lan\n");
        let second = apply_to_text(
            &edited,
            &doms(&["b.example.com", "c.example.com"]),
            &["x"],
            "t2",
        );
        assert_eq!(remove_from_text(&edited), remove_from_text(&second));
        assert_eq!(second.matches(BEGIN).count(), 1);
        assert_eq!(count_blocked(&second), 2);
        assert!(!second.contains("a.example.com"));
    }

    #[test]
    fn apply_is_idempotent_and_remove_gives_the_file_back() {
        let prefs = Prefs {
            lists: vec![],
            whitelist: vec![],
            extra: doms(&["ads.example.com", "trk.example.net"]),
        };
        let one = build_apply(USER, &prefs, "t1");
        let two = build_apply(&one.next, &prefs, "t2");
        assert_eq!(
            one.next, two.next,
            "same choices, same file (the date is kept)"
        );
        assert_eq!(remove_from_text(&one.next), USER);
        assert_eq!(remove_from_text(USER), USER);
    }

    #[test]
    fn a_file_without_final_newline_only_gains_that_newline() {
        let raw = "127.0.0.1 localhost";
        let out = apply_to_text(raw, &doms(&["ads.example.com"]), &["x"], "t");
        assert_eq!(remove_from_text(&out), "127.0.0.1 localhost\n");
        check_replacement(raw, &out).unwrap();
        check_replacement(&out, raw).unwrap();
    }

    #[test]
    fn an_unterminated_block_is_repaired() {
        let broken = format!("{USER}{BEGIN}\n0.0.0.0 leftover.example.com\n");
        let out = apply_to_text(&broken, &doms(&["new.example.com"]), &["x"], "t");
        assert_eq!(out.matches(BEGIN).count(), 1);
        assert!(!out.contains("leftover"));
        assert_eq!(remove_from_text(&out), USER);
    }

    #[test]
    fn crlf_stays_crlf() {
        let win = "127.0.0.1\tlocalhost\r\n# comment\r\n";
        let out = apply_to_text(win, &doms(&["ads.example.com"]), &["x"], "t");
        assert!(out.contains("0.0.0.0 ads.example.com\r\n"));
        assert_eq!(remove_from_text(&out), win);
    }

    #[test]
    fn whitelist_drops_domains_and_subdomains() {
        let a = doms(&["b.example.com", "a.example.com", "ads.google.com"]);
        let b = doms(&["a.example.com", "cdn.ads.google.com"]);
        let (out, skipped) = merge(&[&a, &b], &doms(&["ads.google.com"]));
        assert_eq!(out, doms(&["a.example.com", "b.example.com"]));
        assert_eq!(skipped, 2);
        assert!(!whitelisted(
            "notads.google.com",
            &doms(&["ads.google.com"])
        ));
        let prefs = Prefs {
            lists: vec!["windowsspyblocker".into()],
            whitelist: doms(&["microsoft.com"]),
            extra: vec![],
        };
        let built = build_apply(USER, &prefs, "t");
        assert!(built.skipped_whitelist > 0);
        assert!(!built
            .next
            .lines()
            .any(|l| l.ends_with(".microsoft.com") || l.ends_with(" microsoft.com")));
    }

    #[test]
    fn embedded_lists_parse_and_skip_localhost() {
        let spy = list_domains("windowsspyblocker").unwrap();
        assert!(spy.len() > 300, "{}", spy.len());
        assert!(!spy.iter().any(|d| d == "localhost"));
        assert!(parse_list("nope\n192.168.0.1\n::1 ip6-localhost\n").is_empty());
        assert_eq!(
            parse_list("0.0.0.0 a.example.com b.example.com # x").len(),
            2
        );
    }

    #[test]
    fn check_replacement_refuses_changes_outside_and_odd_lines() {
        let good = apply_to_text(USER, &doms(&["ads.example.com"]), &["x"], "t");
        let outside = good.replace("10.0.0.7 server.lan", "6.6.6.6 bank.example.com");
        assert!(check_replacement(USER, &outside)
            .unwrap_err()
            .starts_with("HOSTS_CHANGED"));
        let hijack = good.replace("0.0.0.0 ads.example.com", "6.6.6.6 bank.example.com");
        assert!(check_replacement(USER, &hijack).is_err());
        let two = format!("{good}{BEGIN}\n{END}\n");
        assert!(check_replacement(USER, &two).is_err());
    }

    fn fake_data(root: &Path) -> PathBuf {
        let wmb = root.join("washmybelly");
        std::fs::create_dir_all(wmb.join("hosts")).unwrap();
        std::fs::write(wmb.join("elevate.key"), [7u8; 32]).unwrap();
        wmb
    }

    #[test]
    fn elevated_write_validates_location_hash_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let wmb = fake_data(dir.path());
        let target = dir.path().join("hosts");
        std::fs::write(&target, USER).unwrap();
        let ctx = Ctx::at(&wmb);
        let next = apply_to_text(USER, &doms(&["ads.example.com"]), &["x"], "t");

        let (staged, _) = stage(&ctx, &next).unwrap();
        elevated_write_to(&staged, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), next);

        std::fs::write(&target, USER).unwrap();
        std::fs::write(&staged, next.replace("ads.example.com", "adz.example.com")).unwrap();
        assert!(elevated_write_to(&staged, &target)
            .unwrap_err()
            .contains("changed"));

        let stray = dir.path().join(staged.file_name().unwrap());
        std::fs::write(&stray, &next).unwrap();
        assert!(elevated_write_to(&stray, &target)
            .unwrap_err()
            .starts_with("REFUSED"));

        let evil = format!("6.6.6.6 bank.example.com\n{next}");
        let (evil_path, _) = stage(&ctx, &evil).unwrap();
        assert!(elevated_write_to(&evil_path, &target)
            .unwrap_err()
            .starts_with("HOSTS_CHANGED"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), USER);

        std::fs::remove_file(wmb.join("elevate.key")).unwrap();
        let (again, _) = stage(&ctx, &next).unwrap();
        assert!(elevated_write_to(&again, &target)
            .unwrap_err()
            .starts_with("REFUSED"));

        #[cfg(unix)]
        {
            std::fs::write(wmb.join("elevate.key"), [7u8; 32]).unwrap();
            let link = wmb
                .join("hosts")
                .join(format!("staged-{}.hosts", hash_of(&next)));
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink(&stray, &link).unwrap();
            assert!(elevated_write_to(&link, &target)
                .unwrap_err()
                .starts_with("REFUSED"));
        }
    }

    #[test]
    fn plan_execute_writes_snapshots_and_undo_restores() {
        let dir = tempfile::tempdir().unwrap();
        let wmb = fake_data(dir.path());
        let ctx = Ctx::at(&wmb);
        let target = dir.path().join("hosts");
        std::fs::write(&target, USER).unwrap();
        with_hosts(&target, || {
            let next = apply_to_text(USER, &doms(&["ads.example.com"]), &["x"], "t");
            let (staged, sha) = stage(&ctx, &next).unwrap();
            let args = json!({ "staged": staged, "sha": sha });
            let out = plan_execute(&ctx, "rcpt_test", "write", &args).unwrap();
            assert_eq!(std::fs::read_to_string(&target).unwrap(), next);
            assert!(
                plan_execute(&ctx, "r", "rm", &args).is_err(),
                "closed action list"
            );
            let Some(Undo::Module { action, args, .. }) = out.undo else {
                panic!("no undo")
            };
            plan_undo(&ctx, &action, &args).unwrap();
            assert_eq!(std::fs::read_to_string(&target).unwrap(), USER);

            let (ops, undo) = plan_elevated(
                &ctx,
                "rcpt_x",
                "write",
                &json!({ "staged": staged, "sha": sha }),
            )
            .unwrap();
            assert_eq!(
                ops,
                vec![ElevatedOp::WriteHosts {
                    staged: staged.clone()
                }]
            );
            assert!(undo.is_some());
            assert!(plan_elevated(
                &ctx,
                "r",
                "write",
                &json!({ "staged": staged, "sha": "0000000000000000" })
            )
            .is_err());
        });
    }

    #[test]
    fn flush_commands_are_fixed() {
        let cmds = dns_flush_commands();
        assert!(!cmds.is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(
                cmds,
                vec![
                    "sudo dscacheutil -flushcache",
                    "sudo killall -HUP mDNSResponder"
                ]
            );
        }
    }

    #[test]
    fn preview_hides_the_long_tail() {
        let many: Vec<String> = (0..100).map(|i| format!("t{i}.example.com")).collect();
        let next = apply_to_text(USER, &many, &["x"], "t");
        let (lines, hidden) = preview_lines(&next, 10);
        assert_eq!(hidden, 90);
        assert!(lines.iter().any(|l| l == END));
    }
}
