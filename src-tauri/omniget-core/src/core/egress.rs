//! Per-execution egress broker. This is a network primitive, not a scheduler.
//! The caller must confine the *whole* engine process (including descendants),
//! and close inherited network descriptors before launching it. CONNECT does
//! not inspect TLS/HTTP status; retry policy must live in the engine/domain.
use base64::Engine;
use std::{
    collections::BTreeSet,
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct Policy {
    /// Exact socket exceptions granted by the trusted domain, never URL callers.
    pub local_allowances: BTreeSet<SocketAddr>,
    pub public_ports: BTreeSet<u16>,
    /// Host interface addresses are added automatically at startup.
    pub blocked_addresses: BTreeSet<IpAddr>,
    pub max_connections: u64,
    pub max_concurrent: usize,
    /// Aggregate upstream plus downstream bytes (including HTTP headers).
    pub max_bytes: u64,
    pub lifetime: Duration,
    pub connect_timeout: Duration,
    pub header_timeout: Duration,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            local_allowances: BTreeSet::new(),
            public_ports: [80, 443].into(),
            blocked_addresses: BTreeSet::new(),
            // HLS without keep-alive opens one CONNECT per fragment (a 3 h
            // VOD is >1000). Bytes and lifetime are the real budgets; this is
            // only a runaway guard (each connection is also byte-charged).
            max_connections: 8192,
            max_concurrent: 8,
            max_bytes: 512 * 1024 * 1024,
            lifetime: Duration::from_secs(900),
            connect_timeout: Duration::from_secs(15),
            header_timeout: Duration::from_secs(5),
        }
    }
}
impl Policy {
    fn permits(&self, address: SocketAddr) -> bool {
        self.local_allowances.contains(&address)
            || (self.public_ports.contains(&address.port())
                && is_public(address.ip())
                && !self.blocked_addresses.contains(&address.ip()))
    }
    /// Reject the entire answer if it contains a forbidden address. The returned
    /// addresses must be connected directly: never resolve the hostname again.
    pub fn validate_addresses(&self, addresses: Vec<SocketAddr>) -> io::Result<Vec<SocketAddr>> {
        if addresses.is_empty() || addresses.iter().any(|a| !self.permits(*a)) {
            return Err(denied());
        }
        Ok(addresses)
    }
}
fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "EGRESS_DENIED")
}
fn is_policy_denial(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied && e.to_string() == "EGRESS_DENIED"
}
/// Proxy answer for a connection that was not established.
fn refusal_response(e: &io::Error) -> &'static [u8] {
    if is_policy_denial(e) {
        b"HTTP/1.1 403 Blocked by OmniGet egress policy\r\nX-OmniGet-Egress: EGRESS_BLOCKED\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    } else {
        b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    }
}
/// Why the broker cut the execution short. The worker reports these as their
/// own codes: a size/time budget is OmniGet's policy, not a platform failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitReason {
    Bytes = 1,
    Lifetime = 2,
    Connections = 3,
}
impl LimitReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Bytes => "LIMIT_BYTES",
            Self::Lifetime => "LIMIT_TIME",
            Self::Connections => "LIMIT_CONNECTIONS",
        }
    }
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Bytes),
            2 => Some(Self::Lifetime),
            3 => Some(Self::Connections),
            _ => None,
        }
    }
}
/// First limit wins: later cuts are consequences of the first one.
fn record_limit(slot: &AtomicU8, reason: LimitReason) {
    let _ = slot.compare_exchange(0, reason as u8, Ordering::SeqCst, Ordering::SeqCst);
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "EGRESS_INVALID_REQUEST")
}

/// Conservative Internet-unicast policy. Transition mechanisms and special
/// purpose ranges are intentionally unavailable, even if sometimes routable.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let [x, y, z, _] = a.octets();
            !(x == 0
                || x == 10
                || x == 127
                || x >= 224
                || (x == 100 && (64..=127).contains(&y))
                || (x == 169 && y == 254)
                || (x == 172 && (16..=31).contains(&y))
                || (x == 192 && (y == 0 || y == 168 || (y == 88 && z == 99)))
                || (x == 198 && (y == 18 || y == 19 || (y == 51 && z == 100)))
                || (x == 203 && y == 0 && z == 113))
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            // Includes no mapped IPv4, NAT64, ULA, link-local, multicast, or 6to4.
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] <= 0x0fff)
        }
    }
}

#[cfg(unix)]
fn host_addresses() -> io::Result<BTreeSet<IpAddr>> {
    let mut head = std::ptr::null_mut();
    // getifaddrs owns a linked list until freeifaddrs; each sockaddr is valid
    // for its family and copied before that allocation is released.
    unsafe {
        if libc::getifaddrs(&mut head) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut result = BTreeSet::new();
        let mut node = head;
        while !node.is_null() {
            let address = (*node).ifa_addr;
            if !address.is_null() {
                match (*address).sa_family as i32 {
                    libc::AF_INET => {
                        let a = &*(address as *const libc::sockaddr_in);
                        result.insert(IpAddr::V4(Ipv4Addr::from(a.sin_addr.s_addr.to_ne_bytes())));
                    }
                    libc::AF_INET6 => {
                        let a = &*(address as *const libc::sockaddr_in6);
                        result.insert(IpAddr::V6(std::net::Ipv6Addr::from(a.sin6_addr.s6_addr)));
                    }
                    _ => {}
                }
            }
            node = (*node).ifa_next;
        }
        libc::freeifaddrs(head);
        Ok(result)
    }
}
#[cfg(not(unix))]
fn host_addresses() -> io::Result<BTreeSet<IpAddr>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "EGRESS_HOST_INTERFACES_UNAVAILABLE",
    ))
}

pub struct Broker {
    pub address: SocketAddr,
    nonce: String,
    authorization: String,
    cancellation: CancellationToken,
    task: Option<JoinHandle<()>>,
    connections: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    denials: Arc<AtomicU64>,
    limit: Arc<AtomicU8>,
}
impl Broker {
    pub async fn start(mut policy: Policy, parent: CancellationToken) -> io::Result<Self> {
        if policy.max_concurrent == 0
            || policy.max_concurrent > 256
            || policy.max_connections == 0
            || policy.max_bytes == 0
            || policy.lifetime.is_zero()
        {
            return Err(invalid());
        }
        policy.blocked_addresses.extend(host_addresses()?);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let nonce = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let authorization = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("omniget:{nonce}"))
        );
        let expected_authorization = Arc::new(authorization.clone());
        let cancellation = parent.child_token();
        let stop = cancellation.clone();
        let connections = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let denials = Arc::new(AtomicU64::new(0));
        let count = connections.clone();
        let used = bytes.clone();
        let denied_count = denials.clone();
        let limit = Arc::new(AtomicU8::new(0));
        let limit_slot = limit.clone();
        let task = tokio::spawn(async move {
            let permits = Arc::new(Semaphore::new(policy.max_concurrent));
            let policy = Arc::new(policy);
            let deadline = tokio::time::sleep(policy.lifetime);
            tokio::pin!(deadline);
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=stop.cancelled()=>break,
                    _=&mut deadline=>{record_limit(&limit_slot,LimitReason::Lifetime);stop.cancel();break},
                    Some(_)=children.join_next(), if !children.is_empty()=>{},
                    result=listener.accept()=> {
                        let Ok((stream,_))=result else {break};
                        if count.fetch_add(1,Ordering::SeqCst)>=policy.max_connections { record_limit(&limit_slot,LimitReason::Connections);stop.cancel();break; }
                        let Ok(permit)=permits.clone().try_acquire_owned() else {drop(stream);continue};
                        let p=policy.clone();let s=stop.clone();let b=used.clone();let auth=expected_authorization.clone();let d=denied_count.clone();let l=limit_slot.clone();
                        children.spawn(async move {
                            let _permit=permit;
                            tokio::select! { _=s.cancelled()=>{}, _=serve(stream,p,b,s.clone(),auth,d,l)=>{} }
                        });
                    }
                }
            }
            stop.cancel();
            children.abort_all();
            while children.join_next().await.is_some() {}
        });
        Ok(Self {
            address,
            nonce,
            authorization,
            cancellation,
            task: Some(task),
            connections,
            bytes,
            denials,
            limit,
        })
    }
    pub fn proxy_url(&self) -> String {
        format!("http://omniget:{}@{}", self.nonce, self.address)
    }
    /// Secret for fixed internal clients/tests only. Never log or persist.
    pub fn proxy_authorization(&self) -> &str {
        &self.authorization
    }
    pub fn revoke(&self) {
        self.cancellation.cancel();
    }
    pub fn is_revoked(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    /// Connections refused by this broker's own policy (not upstream failures).
    /// A worker failure after a denial is OmniGet's block, not the platform's.
    pub fn policy_denials(&self) -> u64 {
        self.denials.load(Ordering::SeqCst)
    }
    /// The budget that revoked this broker, if a budget did (not a cancel).
    pub fn limit_reason(&self) -> Option<LimitReason> {
        LimitReason::from_u8(self.limit.load(Ordering::SeqCst))
    }
    pub fn counters(&self) -> (u64, u64) {
        (
            self.connections.load(Ordering::SeqCst),
            self.bytes.load(Ordering::SeqCst),
        )
    }
    pub async fn shutdown(mut self) {
        self.revoke();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
    /// Network-only macOS profile. Filesystem/IPC restrictions are separately
    /// required before this may confine arbitrary remote code or shell tools.
    pub fn network_profile(&self) -> String {
        format!("(version 1)(allow default)(deny network*)(allow network-outbound (remote tcp4 \"localhost:{}\"))",self.address.port())
    }
    /// Launch fixed trusted engine helper, never a client-selected executable.
    /// Caller supplies only schema-validated engine args and must retain Broker.
    #[cfg(target_os = "macos")]
    pub fn confined_command(&self, helper: &Path) -> io::Result<tokio::process::Command> {
        self.command_with_profile(helper, self.network_profile())
    }
    /// File confinement for fixed engines. Personal paths outside the explicit
    /// roots are denied; no home-directory wildcard or shared temporary root.
    #[cfg(target_os = "macos")]
    pub fn confined_command_with_files(
        &self,
        helper: &Path,
        read_roots: &[std::path::PathBuf],
        write_roots: &[std::path::PathBuf],
    ) -> io::Result<tokio::process::Command> {
        let profile = self.files_profile(helper, read_roots, write_roots)?;
        self.command_with_profile(&helper.canonicalize()?, profile)
    }
    #[cfg(target_os = "macos")]
    fn files_profile(
        &self,
        helper: &Path,
        read_roots: &[std::path::PathBuf],
        write_roots: &[std::path::PathBuf],
    ) -> io::Result<String> {
        let mut profile = self.network_profile();
        profile.push_str("(deny file-read*)(deny file-write*)(deny mach-lookup)(deny mach-register)(deny appleevent-send)(allow file-read* (literal \"/\"))(allow file-read* (literal \"/private/var/select/sh\"))");
        for root in [
            "/System",
            "/usr",
            "/bin",
            "/sbin",
            "/Library",
            "/private/var/db/dyld",
        ] {
            if Path::new(root).exists() {
                append_file_rule(&mut profile, "file-read*", Path::new(root))?;
            }
        }
        for device in ["/dev/null", "/dev/random", "/dev/urandom"] {
            append_file_rule(&mut profile, "file-read*", Path::new(device))?;
        }
        append_file_rule(&mut profile, "file-write*", Path::new("/dev/null"))?;
        append_file_rule(&mut profile, "file-read*", helper)?;
        for root in read_roots {
            append_file_rule(&mut profile, "file-read*", root)?;
        }
        for root in write_roots {
            append_file_rule(&mut profile, "file-read*", root)?;
            append_file_rule(&mut profile, "file-write*", root)?;
        }
        Ok(profile)
    }
    #[cfg(target_os = "macos")]
    fn command_with_profile(
        &self,
        helper: &Path,
        profile: String,
    ) -> io::Result<tokio::process::Command> {
        if !helper.is_absolute() || self.is_revoked() {
            return Err(invalid());
        }
        let mut c = tokio::process::Command::new("/usr/bin/sandbox-exec");
        c.arg("-p").arg(profile).arg(helper);
        c.env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HTTP_PROXY", self.proxy_url())
            .env("HTTPS_PROXY", self.proxy_url())
            .env("http_proxy", self.proxy_url())
            .env("https_proxy", self.proxy_url())
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .kill_on_drop(true);
        c.process_group(0);
        c.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Do not inherit already-connected sockets: Seatbelt checks acquisition.
        // CLOEXEC preserves Rust spawn's error pipe until successful exec while
        // closing every other non-standard descriptor at that same boundary.
        let descriptor_limit = unsafe { libc::getdtablesize() };
        if descriptor_limit < 3 {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            c.pre_exec(move || {
                for fd in 3..descriptor_limit {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags >= 0 && libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        Ok(c)
    }
    #[cfg(not(target_os = "macos"))]
    pub fn confined_command_with_files(
        &self,
        _helper: &Path,
        _read_roots: &[std::path::PathBuf],
        _write_roots: &[std::path::PathBuf],
    ) -> io::Result<tokio::process::Command> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "EGRESS_SANDBOX_UNAVAILABLE",
        ))
    }
    #[cfg(not(target_os = "macos"))]
    pub fn confined_command(&self, _helper: &Path) -> io::Result<tokio::process::Command> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "EGRESS_SANDBOX_UNAVAILABLE",
        ))
    }
}
impl Drop for Broker {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[cfg(target_os = "macos")]
fn append_file_rule(profile: &mut String, operation: &str, path: &Path) -> io::Result<()> {
    let resolved = path.canonicalize()?;
    if resolved.parent().is_none() {
        return Err(invalid());
    }
    let text = resolved.to_str().ok_or_else(invalid)?;
    if text.chars().any(char::is_control) {
        return Err(invalid());
    }
    let quoted = serde_json::to_string(text).map_err(|_| invalid())?;
    if operation == "file-read*" && resolved.is_file() {
        if !path.is_absolute() {
            return Err(invalid());
        }
        // Exact loader aliases need metadata access; file contents remain pinned
        // to the canonical grant. Never grant an ancestor as a readable subtree.
        // Canonical ancestors too: PyInstaller onefile engines (official
        // yt-dlp_macos) realpath() their own executable, so /tmp -> /private/tmp
        // needs lstat of /private and /private/tmp (exit 255 otherwise).
        // Literal metadata only: no readdir, no content, no sibling access.
        for ancestor in path.ancestors().chain(resolved.ancestors()) {
            let raw = ancestor.to_str().ok_or_else(invalid)?;
            if raw.chars().any(char::is_control) {
                return Err(invalid());
            }
            let quoted = serde_json::to_string(raw).map_err(|_| invalid())?;
            profile.push_str(&format!("(allow file-read-metadata (literal {quoted}))"));
        }
    }
    let filter = if resolved.is_dir() {
        "subpath"
    } else {
        "literal"
    };
    profile.push_str(&format!("(allow {operation} ({filter} {quoted}))"));
    Ok(())
}

struct Request {
    authorization: Option<String>,
    host: String,
    port: u16,
    connect: bool,
    forward: Vec<u8>,
}
fn parse(bytes: &[u8]) -> io::Result<Request> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    if text
        .bytes()
        .any(|b| b == 0 || (b < 32 && b != b'\r' && b != b'\n' && b != b'\t'))
    {
        return Err(invalid());
    }
    // Reject bare CR/LF before reconstructing upstream headers.
    let without_crlf = text.replace("\r\n", "");
    if without_crlf.contains(['\r', '\n']) {
        return Err(invalid());
    }
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or_else(invalid)?;
    let parts: Vec<_> = first.split(' ').collect();
    if parts.len() != 3 || !matches!(parts[2], "HTTP/1.1" | "HTTP/1.0") {
        return Err(invalid());
    }
    let connect = parts[0] == "CONNECT";
    if !connect && !matches!(parts[0], "GET" | "HEAD") {
        return Err(invalid());
    }
    let url = url::Url::parse(&if connect {
        format!("http://{}/", parts[1])
    } else {
        parts[1].to_owned()
    })
    .map_err(|_| invalid())?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let host = url
        .host_str()
        .ok_or_else(invalid)?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = url.port_or_known_default().ok_or_else(invalid)?;
    if connect
        && (url.path() != "/" || url.query().is_some() || !parts[1].ends_with(&format!(":{port}")))
    {
        return Err(invalid());
    }
    let mut headers = Vec::new();
    let mut host_seen = false;
    let mut authorization = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(invalid());
        }
        let (name, value) = line.split_once(':').ok_or_else(invalid)?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(invalid());
        }
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "host" => {
                if host_seen {
                    return Err(invalid());
                }
                host_seen = true;
            }
            "content-length" | "transfer-encoding" | "upgrade" | "expect" | "trailer" => {
                return Err(invalid())
            }
            "proxy-authorization" => {
                if authorization.replace(value.trim().to_owned()).is_some() {
                    return Err(invalid());
                }
            }
            "connection" | "proxy-connection" | "proxy-authenticate" => {}
            _ => headers.push(format!("{name}:{value}\r\n")),
        }
    }
    let mut forward = Vec::new();
    if !connect {
        let target = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_owned(),
        };
        let authority = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        forward = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n{}\r\n",
            parts[0],
            target,
            authority,
            headers.concat()
        )
        .into_bytes();
    }
    Ok(Request {
        authorization,
        host,
        port,
        connect,
        forward,
    })
}
async fn serve(
    mut client: TcpStream,
    policy: Arc<Policy>,
    bytes: Arc<AtomicU64>,
    stop: CancellationToken,
    expected_authorization: Arc<String>,
    denials: Arc<AtomicU64>,
    limit: Arc<AtomicU8>,
) -> io::Result<()> {
    let request = tokio::time::timeout(policy.header_timeout, async {
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            if header.len() >= 16 * 1024 {
                return Err(invalid());
            }
            if client.read(&mut byte).await? == 0 {
                return Err(invalid());
            }
            header.push(byte[0]);
        }
        parse(&header)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "EGRESS_HEADER_TIMEOUT"))??;
    if !request
        .authorization
        .as_deref()
        .is_some_and(|a| constant_time_equal(a.as_bytes(), expected_authorization.as_bytes()))
    {
        let _=client.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"OmniGet worker\"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;
        return Err(denied());
    }
    let (host, port) = (request.host.clone(), request.port);
    let connection = tokio::time::timeout(
        policy.connect_timeout,
        connect_upstream(
            &policy,
            move || {
                let host = host.clone();
                async move {
                    Ok(if let Ok(ip) = host.parse::<IpAddr>() {
                        vec![SocketAddr::new(ip, port)]
                    } else {
                        tokio::net::lookup_host((host.as_str(), port))
                            .await?
                            .collect()
                    })
                }
            },
            |address| TcpStream::connect(address),
        ),
    )
    .await
    .map_err(|_| connect_timeout())?;
    let mut upstream = match connection {
        Ok(s) => s,
        Err(e) => {
            // A policy refusal is OmniGet's decision and says so; an upstream
            // that could not be reached is a gateway failure, not a 403.
            let _ = client.write_all(refusal_response(&e)).await;
            if is_policy_denial(&e) {
                denials.fetch_add(1, Ordering::SeqCst);
            }
            return Err(e);
        }
    };
    if request.connect {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        let (cr, cw) = client.split();
        let (ur, uw) = upstream.split();
        tokio::try_join!(
            copy_limited(cr, uw, &bytes, policy.max_bytes, &stop, &limit),
            copy_limited(ur, cw, &bytes, policy.max_bytes, &stop, &limit)
        )?;
    } else {
        charge(
            &bytes,
            request.forward.len() as u64,
            policy.max_bytes,
            &stop,
            &limit,
        )?;
        upstream.write_all(&request.forward).await?;
        // Do not forward any more client bytes: prevents pipelining/smuggling
        // turning a single checked HTTP request into an uncontrolled stream.
        copy_limited(
            &mut upstream,
            &mut client,
            &bytes,
            policy.max_bytes,
            &stop,
            &limit,
        )
        .await?;
    }
    Ok(())
}
fn connect_timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "EGRESS_CONNECT_TIMEOUT")
}
/// Resolution rounds inside one connect budget. CDNs often answer one address
/// per query; an edge that drops SYNs must not eat the whole budget when the
/// next query would name a live one.
const CONNECT_ROUNDS: u32 = 3;
/// Every answer is validated (fail closed) before any address in it is used,
/// and each address gets a bounded slice of the budget.
async fn connect_upstream<R, RF, C, CF>(
    policy: &Policy,
    mut resolve: R,
    mut connect: C,
) -> io::Result<TcpStream>
where
    R: FnMut() -> RF,
    RF: Future<Output = io::Result<Vec<SocketAddr>>>,
    C: FnMut(SocketAddr) -> CF,
    CF: Future<Output = io::Result<TcpStream>>,
{
    let started = Instant::now();
    let attempt = (policy.connect_timeout / CONNECT_ROUNDS).max(Duration::from_millis(1));
    let mut last = None;
    for _ in 0..CONNECT_ROUNDS {
        let remaining = policy.connect_timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let addresses = match tokio::time::timeout(remaining, resolve()).await {
            Ok(result) => result?,
            Err(_) => break,
        };
        let addresses = policy.validate_addresses(addresses)?;
        let current_host = host_addresses()?;
        if addresses
            .iter()
            .any(|a| current_host.contains(&a.ip()) && !policy.local_allowances.contains(a))
        {
            return Err(denied());
        }
        for address in addresses {
            let remaining = policy.connect_timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(attempt.min(remaining), connect(address)).await {
                Ok(Ok(s)) => return Ok(s),
                Ok(Err(e)) => last = Some(e),
                Err(_) => last = Some(connect_timeout()),
            }
        }
    }
    Err(last.unwrap_or_else(connect_timeout))
}
fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0u8, |different, (a, b)| different | (a ^ b))
        == 0
}

fn charge(
    counter: &AtomicU64,
    n: u64,
    max: u64,
    stop: &CancellationToken,
    limit: &AtomicU8,
) -> io::Result<()> {
    if counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |old| {
            old.checked_add(n).filter(|v| *v <= max)
        })
        .is_err()
    {
        record_limit(limit, LimitReason::Bytes);
        stop.cancel();
        return Err(io::Error::new(io::ErrorKind::Other, "EGRESS_BYTE_BUDGET"));
    }
    Ok(())
}
async fn copy_limited<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut r: R,
    mut w: W,
    counter: &AtomicU64,
    max: u64,
    stop: &CancellationToken,
    limit: &AtomicU8,
) -> io::Result<()> {
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let n = r.read(&mut buffer).await?;
        if n == 0 {
            w.shutdown().await?;
            return Ok(());
        }
        charge(counter, n as u64, max, stop, limit)?;
        w.write_all(&buffer[..n]).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn special_ranges_denied() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2001:db8::1",
            "2002:808:808::1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}")
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}")
        }
    }
    #[test]
    fn mixed_dns_fails_closed() {
        let p = Policy::default();
        assert!(p
            .validate_addresses(vec![
                "8.8.8.8:443".parse().unwrap(),
                "127.0.0.1:443".parse().unwrap()
            ])
            .is_err());
    }
    #[test]
    fn parser_rejects_smuggling() {
        for r in [
            "GET http://a/ HTTP/1.1\r\nContent-Length: 0\r\n\r\n",
            "CONNECT user@a:443 HTTP/1.1\r\n\r\n",
            "CONNECT a:443/x HTTP/1.1\r\n\r\n",
            "GET http://a/ HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n",
            "POST http://a/ HTTP/1.1\r\n\r\n",
        ] {
            assert!(parse(r.as_bytes()).is_err(), "{r}")
        }
        let r =
            parse(b"GET http://example.com/a?b=c HTTP/1.1\r\nProxy-Authorization: SECRET\r\n\r\n")
                .unwrap();
        assert!(!String::from_utf8(r.forward).unwrap().contains("SECRET"));
    }
    #[tokio::test]
    async fn d15_unreachable_granted_upstream_is_a_gateway_failure_not_a_denial() {
        // A granted socket with nothing listening: the connection fails
        // upstream, which is not a policy decision.
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = closed.local_addr().unwrap();
        drop(closed);
        let mut p = Policy::default();
        p.local_allowances.insert(address);
        let b = Broker::start(p, CancellationToken::new()).await.unwrap();
        let mut c = TcpStream::connect(b.address).await.unwrap();
        c.write_all(
            format!(
                "CONNECT {address} HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
                b.proxy_authorization()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).await.unwrap();
        assert!(
            out.starts_with(b"HTTP/1.1 502"),
            "{}",
            String::from_utf8_lossy(&out)
        );
        assert_eq!(b.policy_denials(), 0);
        b.shutdown().await;
    }
    #[tokio::test]
    async fn exact_grant_and_revoke() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = target.local_addr().unwrap();
        let mut p = Policy::default();
        p.local_allowances.insert(address);
        let b = Broker::start(p, CancellationToken::new()).await.unwrap();
        let mut denied = TcpStream::connect(b.address).await.unwrap();
        denied
            .write_all(
                format!(
                    "CONNECT 127.0.0.1:1 HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
                    b.proxy_authorization()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut out = Vec::new();
        denied.read_to_end(&mut out).await.unwrap();
        assert!(out.starts_with(b"HTTP/1.1 403"));
        // D-15: the refusal names OmniGet's policy and is counted, so the
        // worker failure is not reported as the platform's 403.
        assert!(String::from_utf8_lossy(&out).contains("X-OmniGet-Egress: EGRESS_BLOCKED"));
        assert_eq!(b.policy_denials(), 1);
        let mut c = TcpStream::connect(b.address).await.unwrap();
        c.write_all(
            format!(
                "CONNECT {address} HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
                b.proxy_authorization()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let (_accepted, _) = target.accept().await.unwrap();
        let mut response = [0; 39];
        c.read_exact(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200"));
        b.shutdown().await;
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), c.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
    /// Tunnel one CONNECT through `b` to `target`, then close it.
    async fn tunnel_once(b: &Broker, target: SocketAddr) -> Vec<u8> {
        let mut c = TcpStream::connect(b.address).await.unwrap();
        c.write_all(
            format!(
                "CONNECT {target} HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
                b.proxy_authorization()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut out = vec![0; 39];
        let n = tokio::time::timeout(Duration::from_secs(2), c.read(&mut out))
            .await
            .map(|r| r.unwrap_or(0))
            .unwrap_or(0);
        out.truncate(n);
        out
    }
    /// HLS through the broker opens one CONNECT per fragment when the engine
    /// does not keep connections alive: a 3 h Twitch VOD is >1000 fragments.
    /// The total number of connections must not cut it; bytes/time do.
    #[tokio::test]
    async fn hls_many_sequential_connections_do_not_revoke() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = target.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((s, _)) = target.accept().await else {
                    break;
                };
                drop(s);
            }
        });
        let mut p = Policy::default();
        p.local_allowances.insert(address);
        let b = Broker::start(p, CancellationToken::new()).await.unwrap();
        for i in 0..300 {
            let out = tunnel_once(&b, address).await;
            assert!(
                out.starts_with(b"HTTP/1.1 200"),
                "fragment {i}: {}",
                String::from_utf8_lossy(&out)
            );
        }
        assert!(!b.is_revoked());
        server.abort();
        b.shutdown().await;
    }
    #[tokio::test]
    async fn limit_reason_names_the_budget_that_cut() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = target.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = target.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let _ = s.write_all(&[7u8; 4096]).await;
                    let mut sink = [0; 1024];
                    while matches!(s.read(&mut sink).await, Ok(n) if n > 0) {}
                });
            }
        });
        // Connections.
        let mut p = Policy::default();
        p.local_allowances.insert(address);
        p.max_connections = 2;
        let b = Broker::start(p.clone(), CancellationToken::new())
            .await
            .unwrap();
        for _ in 0..3 {
            let _ = tunnel_once(&b, address).await;
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while !b.is_revoked() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(b.limit_reason(), Some(LimitReason::Connections));
        b.shutdown().await;
        // Bytes: the upstream greeting alone exceeds the budget.
        let mut bytes = p.clone();
        bytes.max_connections = 256;
        bytes.max_bytes = 1024;
        let b = Broker::start(bytes, CancellationToken::new())
            .await
            .unwrap();
        let _ = tunnel_once(&b, address).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !b.is_revoked() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(b.limit_reason(), Some(LimitReason::Bytes));
        b.shutdown().await;
        // Lifetime.
        let mut time = p.clone();
        time.max_connections = 256;
        time.lifetime = Duration::from_millis(50);
        let b = Broker::start(time, CancellationToken::new()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(b.is_revoked());
        assert_eq!(b.limit_reason(), Some(LimitReason::Lifetime));
        b.shutdown().await;
        // A plain cancel is not a limit.
        let parent = CancellationToken::new();
        let b = Broker::start(p, parent.clone()).await.unwrap();
        parent.cancel();
        assert!(b.is_revoked());
        assert_eq!(b.limit_reason(), None);
        b.shutdown().await;
        server.abort();
        assert_eq!(LimitReason::Bytes.code(), "LIMIT_BYTES");
        assert_eq!(LimitReason::Lifetime.code(), "LIMIT_TIME");
        assert_eq!(LimitReason::Connections.code(), "LIMIT_CONNECTIONS");
    }
    /// bsky case: the CDN edge in the first DNS answer drops SYNs; the next
    /// answer names a live edge. One budget must cover a re-resolution.
    #[tokio::test]
    async fn connect_re_resolves_after_unresponsive_address() {
        let live = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good = live.local_addr().unwrap();
        let dead: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let mut p = Policy::default();
        p.local_allowances.insert(good);
        p.local_allowances.insert(dead);
        p.connect_timeout = Duration::from_millis(900);
        let answers = std::sync::Mutex::new(vec![vec![good], vec![dead]]);
        let resolves = AtomicU64::new(0);
        let started = Instant::now();
        let stream = tokio::time::timeout(
            p.connect_timeout,
            connect_upstream(
                &p,
                || {
                    resolves.fetch_add(1, Ordering::SeqCst);
                    let next = answers.lock().unwrap().pop().unwrap_or_default();
                    async move { Ok(next) }
                },
                |address| async move {
                    if address == dead {
                        std::future::pending::<()>().await;
                    }
                    TcpStream::connect(address).await
                },
            ),
        )
        .await
        .expect("one budget covers the retry")
        .unwrap();
        assert_eq!(stream.peer_addr().unwrap(), good);
        assert_eq!(resolves.load(Ordering::SeqCst), 2);
        assert!(started.elapsed() < p.connect_timeout);
        // A forbidden address in a later answer still fails closed.
        let answers = std::sync::Mutex::new(vec![vec!["127.0.0.1:1".parse().unwrap()], vec![dead]]);
        let err = connect_upstream(
            &p,
            || {
                let next = answers.lock().unwrap().pop().unwrap_or_default();
                async move { Ok(next) }
            },
            |address| async move {
                if address == dead {
                    std::future::pending::<()>().await;
                }
                TcpStream::connect(address).await
            },
        )
        .await
        .unwrap_err();
        assert!(is_policy_denial(&err));
    }
    #[tokio::test]
    async fn byte_budget_revokes() {
        let t = CancellationToken::new();
        let n = AtomicU64::new(0);
        let l = AtomicU8::new(0);
        charge(&n, 10, 10, &t, &l).unwrap();
        assert!(charge(&n, 1, 10, &t, &l).is_err());
        assert!(t.is_cancelled());
        assert_eq!(
            LimitReason::from_u8(l.load(Ordering::SeqCst)),
            Some(LimitReason::Bytes)
        );
        assert_eq!(n.load(Ordering::SeqCst), 10);
    }
    #[tokio::test]
    async fn another_broker_credential_is_rejected_before_connect() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = target.local_addr().unwrap();
        let mut policy = Policy::default();
        policy.local_allowances.insert(addr);
        let old = Broker::start(policy.clone(), CancellationToken::new())
            .await
            .unwrap();
        let stale = old.proxy_authorization().to_owned();
        old.shutdown().await;
        let current = Broker::start(policy, CancellationToken::new())
            .await
            .unwrap();
        for verb in ["GET", "CONNECT"] {
            let mut client = TcpStream::connect(current.address).await.unwrap();
            let destination = if verb == "GET" {
                format!("http://{addr}/")
            } else {
                addr.to_string()
            };
            client
                .write_all(
                    format!(
                        "{verb} {destination} HTTP/1.1\r\nProxy-Authorization: {stale}\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            assert!(response.starts_with(b"HTTP/1.1 407"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), target.accept())
                .await
                .is_err()
        );
        current.shutdown().await;
    }
    #[tokio::test]
    async fn http_does_not_forward_pipelined_request() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = target.local_addr().unwrap();
        let mut p = Policy::default();
        p.local_allowances.insert(addr);
        let b = Broker::start(p, CancellationToken::new()).await.unwrap();
        let server = tokio::spawn(async move {
            let (mut c, _) = target.accept().await.unwrap();
            let mut received = Vec::new();
            let mut byte = [0];
            while !received.ends_with(b"\r\n\r\n") {
                c.read_exact(&mut byte).await.unwrap();
                received.push(byte[0]);
            }
            c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            let received = String::from_utf8(received).unwrap();
            assert!(!received.contains("forbidden"));
            assert!(!received
                .to_ascii_lowercase()
                .contains("proxy-authorization"));
        });
        let mut c = TcpStream::connect(b.address).await.unwrap();
        c.write_all(format!("GET http://{addr}/safe HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\nGET http://127.0.0.1:1/forbidden HTTP/1.1\r\n\r\n",b.proxy_authorization()).as_bytes()).await.unwrap();
        let mut out = Vec::new();
        // Closing with intentionally unread pipelined bytes may send TCP RST.
        if let Err(e) = c.read_to_end(&mut out).await {
            assert_eq!(e.kind(), io::ErrorKind::ConnectionReset);
        }
        assert!(out.ends_with(b"ok"));
        server.await.unwrap();
        b.shutdown().await;
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn actual_sandbox_curl_must_use_broker() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = target.local_addr().unwrap();
        let mut p = Policy::default();
        p.local_allowances.insert(addr);
        let b = Broker::start(p, CancellationToken::new()).await.unwrap();
        // Same port on IPv6 must not become an alternative sandbox escape.
        let v6 = TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, b.address.port()))
            .await
            .unwrap();
        let v6_denied = b
            .confined_command(Path::new("/usr/bin/curl"))
            .unwrap()
            .args([
                "--noproxy",
                "*",
                "-sS",
                "--max-time",
                "2",
                &format!("http://[::1]:{}/", b.address.port()),
            ])
            .output()
            .await
            .unwrap();
        assert!(!v6_denied.status.success());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), v6.accept())
                .await
                .is_err()
        );
        let denied = b
            .confined_command(Path::new("/usr/bin/curl"))
            .unwrap()
            .args([
                "--noproxy",
                "*",
                "-sS",
                "--max-time",
                "2",
                &format!("http://{addr}/"),
            ])
            .output()
            .await
            .unwrap();
        assert!(!denied.status.success());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), target.accept())
                .await
                .is_err()
        );
        let server = tokio::spawn(async move {
            let (mut c, _) = target.accept().await.unwrap();
            let mut data = [0; 4096];
            let _ = c.read(&mut data).await.unwrap();
            c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let allowed = b
            .confined_command(Path::new("/usr/bin/curl"))
            .unwrap()
            .args(["-sS", "--max-time", "3", &format!("http://{addr}/")])
            .output()
            .await
            .unwrap();
        assert!(
            allowed.status.success(),
            "{}",
            String::from_utf8_lossy(&allowed.stderr)
        );
        assert_eq!(allowed.stdout, b"ok");
        server.await.unwrap();
        b.shutdown().await;
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn inherited_connected_descriptor_closed_at_exec() {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let connected = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        // Deliberately create an inheritable FD, unlike Rust's default sockets.
        let fd = unsafe { libc::fcntl(connected.as_raw_fd(), libc::F_DUPFD, 128) };
        assert!(fd >= 128);
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let broker = Broker::start(Policy::default(), CancellationToken::new())
            .await
            .unwrap();
        let code=format!("import os\ntry: os.write({fd}, b'LEAK'); print('leaked')\nexcept OSError: print('closed')");
        let result = broker
            .confined_command(Path::new("/usr/bin/python3"))
            .unwrap()
            .args(["-c", &code])
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"closed\n");
        drop(owned);
        drop(connected);
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut peer, &mut data).unwrap();
        assert!(data.is_empty());
        broker.shutdown().await;
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn actual_file_sandbox_denies_ungranted_and_symlink_targets() {
        let root = std::env::temp_dir().join(format!(
            "omniget-egress-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let grant = root.join("granted");
        std::fs::create_dir_all(&grant).unwrap();
        let grant = grant.canonicalize().unwrap();
        let outside = root.join("private.txt");
        std::fs::write(&outside, b"synthetic-secret").unwrap();
        std::os::unix::fs::symlink(&outside, grant.join("escape")).unwrap();
        let b = Broker::start(Policy::default(), CancellationToken::new())
            .await
            .unwrap();
        let script="printf ok > \"$1/allowed\" || exit 10; test \"$(cat \"$1/allowed\")\" = ok || exit 11; if cat \"$2\" >/dev/null 2>&1; then exit 12; fi; if cat \"$1/escape\" >/dev/null 2>&1; then exit 13; fi; if printf bad > \"$3\" 2>/dev/null; then exit 14; fi; if ln \"$2\" \"$1/hardlink\" 2>/dev/null; then exit 15; fi; if cat \"$4\" >/dev/null 2>&1; then exit 16; fi; printf confined";
        let result = b
            .confined_command_with_files(Path::new("/bin/sh"), &[], &[grant.clone()])
            .unwrap()
            .current_dir(&grant)
            .args([
                "-c",
                script,
                "worker",
                grant.to_str().unwrap(),
                outside.to_str().unwrap(),
                root.join("ungranted-write").to_str().unwrap(),
                &format!(
                    "/System/Volumes/Data{}",
                    outside.canonicalize().unwrap().display()
                ),
            ])
            .output()
            .await
            .unwrap();
        b.shutdown().await;
        assert!(
            result.status.success(),
            "exit={:?}, stderr={}",
            result.status.code(),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"confined");
        assert_eq!(std::fs::read(&outside).unwrap(), b"synthetic-secret");
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Real module + official PyInstaller standalone (opt-in: it is a 37 MB
    /// external binary). `OMNIGET_TEST_STANDALONE_YTDLP=/abs/yt-dlp`.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn actual_file_sandbox_runs_standalone_ytdlp_version() {
        let Some(binary) = std::env::var_os("OMNIGET_TEST_STANDALONE_YTDLP") else {
            eprintln!("skipped: OMNIGET_TEST_STANDALONE_YTDLP unset");
            return;
        };
        let binary = std::path::PathBuf::from(binary);
        let runtime = std::env::temp_dir().join(format!(
            "omniget-egress-ytdlp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&runtime).unwrap();
        let runtime = runtime.canonicalize().unwrap();
        let b = Broker::start(Policy::default(), CancellationToken::new())
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(120),
            b.confined_command_with_files(&binary, &[], &[runtime.clone()])
                .unwrap()
                .current_dir(&runtime)
                .env("TMPDIR", &runtime)
                .args(["--ignore-config", "--version"])
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        b.shutdown().await;
        eprintln!(
            "standalone yt-dlp in sandbox: exit={:?} elapsed={:?} stdout={} stderr={}",
            result.status.code(),
            started.elapsed(),
            String::from_utf8_lossy(&result.stdout).trim(),
            String::from_utf8_lossy(&result.stderr).trim()
        );
        let _ = std::fs::remove_dir_all(&runtime);
        assert!(result.status.success());
        assert!(!result.stdout.is_empty());
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn file_grant_ancestors_are_metadata_only() {
        // Non-canonical /tmp alias on purpose (/tmp -> /private/tmp).
        let root = Path::new("/tmp").join(format!(
            "omniget-egress-ancestors-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let granted = root.join("bin").join("engine");
        std::fs::write(&granted, b"granted").unwrap();
        let sibling = root.join("bin").join("sibling");
        std::fs::write(&sibling, b"synthetic-secret").unwrap();
        let b = Broker::start(Policy::default(), CancellationToken::new())
            .await
            .unwrap();
        let profile = b
            .files_profile(Path::new("/bin/sh"), &[granted.clone()], &[])
            .unwrap();
        let canonical = granted.canonicalize().unwrap();
        for ancestor in canonical.ancestors().skip(1) {
            let a = serde_json::to_string(ancestor.to_str().unwrap()).unwrap();
            assert!(profile.contains(&format!("(allow file-read-metadata (literal {a}))")));
            assert!(!profile.contains(&format!("(subpath {a})")), "{a}");
        }
        assert!(!profile.contains("(subpath \"/private\")"));
        assert!(!profile.contains("(subpath \"/private/tmp\")"));
        let script = "test \"$(cat \"$1\")\" = granted || exit 10; stat \"$4\" >/dev/null 2>&1 || exit 11; stat /private/tmp >/dev/null 2>&1 || exit 12; if cat \"$2\" >/dev/null 2>&1; then exit 13; fi; if ls \"$3\" >/dev/null 2>&1; then exit 14; fi; if ls /private/tmp >/dev/null 2>&1; then exit 15; fi; printf confined";
        let result = b
            .confined_command_with_files(Path::new("/bin/sh"), &[granted.clone()], &[])
            .unwrap()
            .args([
                "-c",
                script,
                "worker",
                canonical.to_str().unwrap(),
                sibling.canonicalize().unwrap().to_str().unwrap(),
                canonical.parent().unwrap().to_str().unwrap(),
                canonical
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_str()
                    .unwrap(),
            ])
            .output()
            .await
            .unwrap();
        b.shutdown().await;
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            result.status.success(),
            "exit={:?}, stderr={}",
            result.status.code(),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"confined");
    }
}
