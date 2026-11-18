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
    pub local_allowances: BTreeSet<SocketAddr>,
    pub public_ports: BTreeSet<u16>,
    pub blocked_addresses: BTreeSet<IpAddr>,
    pub max_connections: u64,
    pub max_concurrent: usize,
    pub max_bytes: u64,
    pub lifetime: Duration,
    pub connect_timeout: Duration,
    pub header_timeout: Duration,
    pub upstream: Option<Upstream>,
}

#[derive(Clone)]
pub struct Upstream {
    proxy: String,
    route: Option<crate::core::telegram::net::route::Route>,
}
impl std::fmt::Debug for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::core::proxy_pool::redact(&self.proxy))
    }
}
impl Upstream {
    pub fn new(proxy: &str) -> Self {
        use crate::core::telegram::net::route::{Route, RouteKind, RouteSource};
        let route = url::Url::parse(proxy).ok().and_then(|url| {
            let kind = match url.scheme() {
                "http" => RouteKind::Http,
                "socks5" | "socks5h" => RouteKind::Socks5,
                _ => return None,
            };
            let host = url.host_str()?.trim_matches(['[', ']']).to_owned();
            let mut route = Route::new(kind, host, url.port()?, RouteSource::Manual);
            let decoded = |text: &str| {
                urlencoding::decode(text).map_or_else(|_| text.to_owned(), |t| t.into_owned())
            };
            if !url.username().is_empty() {
                route.username = Some(decoded(url.username()));
                route.password = url.password().map(decoded);
            }
            Some(route)
        });
        Self {
            proxy: proxy.to_owned(),
            route,
        }
    }
    pub fn from_settings() -> Option<Self> {
        crate::core::http_client::proxy_url().map(|proxy| Self::new(&proxy))
    }
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        if self.route.is_some() {
            if let Ok(found) =
                crate::core::telegram::net::dns::resolve_doh_through(host, &self.proxy).await
            {
                return Ok(found
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, port))
                    .collect());
            }
        }
        Ok(tokio::net::lookup_host((host, port)).await?.collect())
    }
    async fn connect(&self, target: SocketAddr) -> io::Result<TcpStream> {
        use crate::core::telegram::net::{
            route::RouteKind,
            tunnel::{self, Target},
        };
        let Some(route) = &self.route else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "EGRESS_UPSTREAM_UNSUPPORTED",
            ));
        };
        let stream = match TcpStream::connect((route.host.as_str(), route.port)).await {
            Ok(stream) => stream,
            Err(e) => {
                crate::core::proxy_pool::report(&self.proxy, false);
                return Err(e);
            }
        };
        let target = Target::Ip(target);
        let tunnelled = match route.kind {
            RouteKind::Socks5 => tunnel::socks5_connect(stream, route, &target).await,
            _ => tunnel::http_connect(stream, route, &target).await,
        };
        tunnelled.map_err(|_| {
            io::Error::new(io::ErrorKind::ConnectionRefused, "EGRESS_UPSTREAM_REFUSED")
        })
    }
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            local_allowances: BTreeSet::new(),
            public_ports: [80, 443].into(),
            blocked_addresses: BTreeSet::new(),
            max_connections: 8192,
            max_concurrent: 8,
            max_bytes: 512 * 1024 * 1024,
            lifetime: Duration::from_secs(900),
            connect_timeout: Duration::from_secs(15),
            header_timeout: Duration::from_secs(5),
            upstream: None,
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
fn refusal_response(e: &io::Error) -> &'static [u8] {
    if is_policy_denial(e) {
        b"HTTP/1.1 403 Blocked by OmniGet egress policy\r\nX-OmniGet-Egress: EGRESS_BLOCKED\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    } else {
        b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    }
}
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
fn record_limit(slot: &AtomicU8, reason: LimitReason) {
    let _ = slot.compare_exchange(0, reason as u8, Ordering::SeqCst, Ordering::SeqCst);
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "EGRESS_INVALID_REQUEST")
}

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
        let expected_authorization = Arc::new(authorization);
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
    pub fn revoke(&self) {
        self.cancellation.cancel();
    }
    pub fn is_revoked(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub fn policy_denials(&self) -> u64 {
        self.denials.load(Ordering::SeqCst)
    }
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
    pub fn network_profile(&self) -> String {
        format!("(version 1)(allow default)(deny network*)(allow network-outbound (remote tcp4 \"localhost:{}\"))",self.address.port())
    }
    #[cfg(target_os = "macos")]
    pub fn confined_command(&self, helper: &Path) -> io::Result<tokio::process::Command> {
        self.command_with_profile(helper, self.network_profile())
    }
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
    let route = policy.clone();
    let names = policy.clone();
    let connection = tokio::time::timeout(
        policy.connect_timeout,
        connect_upstream(
            &policy,
            move || {
                let host = host.clone();
                let names = names.clone();
                async move {
                    Ok(if let Ok(ip) = host.parse::<IpAddr>() {
                        vec![SocketAddr::new(ip, port)]
                    } else if let Some(upstream) = &names.upstream {
                        upstream.resolve(&host, port).await?
                    } else {
                        tokio::net::lookup_host((host.as_str(), port))
                            .await?
                            .collect()
                    })
                }
            },
            move |address: SocketAddr| {
                let route = route.clone();
                async move {
                    match &route.upstream {
                        Some(upstream) if !route.local_allowances.contains(&address) => {
                            upstream.connect(address).await
                        }
                        _ => TcpStream::connect(address).await,
                    }
                }
            },
        ),
    )
    .await
    .map_err(|_| connect_timeout())?;
    let mut upstream = match connection {
        Ok(s) => s,
        Err(e) => {
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
const CONNECT_ROUNDS: u32 = 3;
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
        return Err(io::Error::other("EGRESS_BYTE_BUDGET"));
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
mod upstream_tests {
    use super::*;

    async fn fake_proxy() -> (SocketAddr, tokio::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (seen, heads) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && head.len() < 4096 {
                    if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                        break;
                    }
                    head.push(byte[0]);
                }
                let _ = seen.send(String::from_utf8_lossy(&head).into_owned()).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\nhello")
                    .await;
            }
        });
        (address, heads)
    }

    async fn through(broker: &Broker, target: &str) -> String {
        let url = url::Url::parse(&broker.proxy_url()).unwrap();
        let auth = base64::engine::general_purpose::STANDARD
            .encode(format!("omniget:{}", url.password().unwrap()));
        let mut client = TcpStream::connect(broker.address).await.unwrap();
        client
            .write_all(
                format!(
                    "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut answer = vec![0u8; 256];
        let mut filled = 0;
        while filled < answer.len() {
            match tokio::time::timeout(Duration::from_secs(5), client.read(&mut answer[filled..]))
                .await
            {
                Ok(Ok(n)) if n > 0 => filled += n,
                _ => break,
            }
            let got = &answer[..filled];
            let refused = got.ends_with(b"\r\n\r\n") && !got.starts_with(b"HTTP/1.1 200");
            if got.ends_with(b"hello") || refused {
                break;
            }
        }
        String::from_utf8_lossy(&answer[..filled]).into_owned()
    }

    #[cfg_attr(windows, ignore = "egress cannot list host interfaces on Windows yet")]
    #[tokio::test]
    async fn an_allowed_address_goes_out_through_the_upstream_proxy() {
        let (proxy, mut heads) = fake_proxy().await;
        let policy = Policy {
            upstream: Some(Upstream::new(&format!("http://bob:p%40ss@{proxy}"))),
            ..Policy::default()
        };
        let broker = Broker::start(policy, CancellationToken::new())
            .await
            .unwrap();
        let answer = through(&broker, "93.184.216.34:443").await;
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.ends_with("hello"), "{answer}");
        let head = heads.recv().await.unwrap();
        assert!(head.starts_with("CONNECT 93.184.216.34:443 "), "{head}");
        let expected = base64::engine::general_purpose::STANDARD.encode("bob:p@ss");
        assert!(
            head.contains(&format!("Proxy-Authorization: Basic {expected}")),
            "{head}"
        );
        broker.shutdown().await;
    }

    #[cfg_attr(windows, ignore = "egress cannot list host interfaces on Windows yet")]
    #[tokio::test]
    async fn the_policy_still_refuses_a_private_address_before_the_upstream_sees_it() {
        let (proxy, mut heads) = fake_proxy().await;
        let policy = Policy {
            upstream: Some(Upstream::new(&format!("http://{proxy}"))),
            ..Policy::default()
        };
        let broker = Broker::start(policy, CancellationToken::new())
            .await
            .unwrap();
        for target in [
            "127.0.0.1:443",
            "10.0.0.5:443",
            "169.254.169.254:443",
            "93.184.216.34:22",
        ] {
            let answer = through(&broker, target).await;
            assert!(answer.starts_with("HTTP/1.1 403"), "{target}: {answer}");
        }
        assert_eq!(broker.policy_denials(), 4);
        assert!(heads.try_recv().is_err());
        broker.shutdown().await;
    }

    #[cfg_attr(windows, ignore = "egress cannot list host interfaces on Windows yet")]
    #[tokio::test]
    async fn a_proxy_the_broker_cannot_speak_never_becomes_a_direct_connection() {
        let policy = Policy {
            upstream: Some(Upstream::new("https://bob:hunter2@proxy.example:443")),
            ..Policy::default()
        };
        assert!(!format!("{policy:?}").contains("hunter2"));
        let broker = Broker::start(policy, CancellationToken::new())
            .await
            .unwrap();
        let answer = through(&broker, "93.184.216.34:443").await;
        assert!(answer.starts_with("HTTP/1.1 502"), "{answer}");
        broker.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "network: run by hand with --ignored"]
    async fn a_name_is_resolved_through_the_proxy_and_still_checked_by_the_policy() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let proxy = listener.local_addr().unwrap();
        let (seen, mut heads) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let seen = seen.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") && head.len() < 4096 {
                        if client.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let target = head.split(' ').nth(1).unwrap_or_default().to_owned();
                    let _ = seen.send(target.clone()).await;
                    let Ok(mut remote) = TcpStream::connect(target.as_str()).await else {
                        return;
                    };
                    let _ = client
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await;
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
                });
            }
        });
        let upstream = Upstream::new(&format!("http://{proxy}"));
        let found = upstream.resolve("example.com", 443).await.unwrap();
        assert!(!found.is_empty());
        assert!(found.iter().all(|a| is_public(a.ip())), "{found:?}");
        let mut asked = Vec::new();
        while let Ok(target) = heads.try_recv() {
            asked.push(target);
        }
        assert!(
            asked.iter().any(|t| t.contains("cloudflare-dns.com")
                || t.contains("dns.google")
                || t.contains("dns.quad9.net")),
            "{asked:?}"
        );
        assert!(Policy::default().validate_addresses(found).is_ok());
    }
}
