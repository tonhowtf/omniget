//! Self-signed HTTPS in portals (Maestri accepts it; M-L1).
//!
//! WebKit rejects an untrusted server certificate unless the navigation
//! delegate answers `webView:didReceiveAuthenticationChallenge:completionHandler:`.
//! wry's delegate does not, so the method is added to its class once at bind
//! time, before any portal exists (WebKit caches which delegate methods exist
//! when the delegate is set, so it must be there first).
//!
//! The decision is per portal: loopback hosts (`localhost`, `*.localhost`,
//! `127.0.0.1`, `::1`) are dev servers and pass; any other host with a bad
//! certificate is refused once and reported as `certError` on the portal's
//! state, so the node shows the warning and "Proceed" (`trust`) adds the host
//! to that portal only (persisted in the node's `data.trustedHosts`). Webviews
//! that are not portals (the app itself) get WebKit's default handling.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// label → hosts the user accepted for that portal.
fn trusted() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    static T: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// label → (host, url) of the last refused certificate.
fn errors() -> &'static Mutex<HashMap<String, (String, String)>> {
    static E: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    E.get_or_init(Default::default)
}

pub fn is_loopback(host: &str) -> bool {
    let h = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h == "::1"
        || h.parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Hosts accepted for `label` (seeded from the node's `trustedHosts`).
pub fn set_trusted(label: &str, hosts: impl IntoIterator<Item = String>) {
    if let Ok(mut t) = trusted().lock() {
        let e = t.entry(label.to_string()).or_default();
        for h in hosts {
            e.insert(h.to_ascii_lowercase());
        }
    }
}

pub fn trust(label: &str, host: &str) {
    set_trusted(label, [host.to_string()]);
    if let Ok(mut e) = errors().lock() {
        e.remove(label);
    }
}

pub fn is_trusted(label: &str, host: &str) -> bool {
    is_loopback(host)
        || trusted()
            .lock()
            .map(|t| {
                t.get(label)
                    .is_some_and(|s| s.contains(&host.to_ascii_lowercase()))
            })
            .unwrap_or(false)
}

/// `(host, url)` of the certificate this portal refused last, if any.
pub fn cert_error(label: &str) -> Option<(String, String)> {
    errors().lock().ok().and_then(|e| e.get(label).cloned())
}

pub fn clear_error(label: &str) {
    if let Ok(mut e) = errors().lock() {
        e.remove(label);
    }
}

pub fn forget(label: &str) {
    clear_error(label);
    if let Ok(mut t) = trusted().lock() {
        t.remove(label);
    }
}

fn record_error(label: &str, host: &str) {
    let url = super::host()
        .state_of(label)
        .map(|s| s.url)
        .unwrap_or_default();
    if let Ok(mut e) = errors().lock() {
        e.insert(label.to_string(), (host.to_string(), url));
    }
    // The page will never finish: wake whoever waits for the load and let the
    // node show the warning.
    super::host().set(label, |s| s.loading = false);
    super::host().fire_loads(label);
    if let Some(app) = super::app() {
        super::host().emit(&app, label);
    }
}

/// Adds the challenge handler to the navigation delegate class of `ww`
/// (the main window: same class as every portal's). Idempotent.
#[cfg(target_os = "macos")]
pub fn install(ww: &tauri::WebviewWindow) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = ww.with_webview(|pv| unsafe { mac::install_on(pv.inner() as *mut _) });
}

#[cfg(not(target_os = "macos"))]
pub fn install(_: &tauri::WebviewWindow) {}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void, CStr};

    use objc2::runtime::{AnyClass, AnyObject, Sel};
    use objc2::sel;

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecTrustEvaluateWithError(trust: *mut c_void, error: *mut *mut c_void) -> bool;
    }

    const USE_CREDENTIAL: isize = 0;
    const DEFAULT_HANDLING: isize = 1;
    const CANCEL: isize = 2;

    type Handler = block2::Block<dyn Fn(isize, *mut AnyObject)>;

    unsafe fn send0(obj: *mut c_void, sel: Sel) -> *mut c_void {
        if obj.is_null() {
            return std::ptr::null_mut();
        }
        let f: unsafe extern "C" fn(*mut c_void, Sel) -> *mut c_void =
            std::mem::transmute(objc2::ffi::objc_msgSend as *const c_void);
        f(obj, sel)
    }

    unsafe fn send1(obj: *mut c_void, sel: Sel, a: *mut c_void) -> *mut c_void {
        if obj.is_null() {
            return std::ptr::null_mut();
        }
        let f: unsafe extern "C" fn(*mut c_void, Sel, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc2::ffi::objc_msgSend as *const c_void);
        f(obj, sel, a)
    }

    unsafe fn string(ns: *mut c_void) -> String {
        let p = send0(ns, sel!(UTF8String)) as *const c_char;
        if p.is_null() {
            return String::new();
        }
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }

    /// Portal label of a WKWebView, from its window title
    /// (`OmniGet Portal portal-<id>`, see `super::super::window_title`).
    unsafe fn portal_label(webview: *mut c_void) -> Option<String> {
        let win = send0(webview, sel!(window));
        let title = string(send0(win, sel!(title)));
        title
            .strip_prefix("OmniGet Portal ")
            .filter(|l| l.starts_with(super::super::LABEL_PREFIX))
            .map(String::from)
    }

    unsafe extern "C-unwind" fn challenge(
        _this: *mut AnyObject,
        _sel: Sel,
        webview: *mut AnyObject,
        challenge: *mut AnyObject,
        handler: *mut Handler,
    ) {
        let answer = |d: isize, cred: *mut AnyObject| {
            if let Some(h) = handler.as_ref() {
                h.call((d, cred));
            }
        };
        let space = send0(challenge as *mut c_void, sel!(protectionSpace));
        let method = string(send0(space, sel!(authenticationMethod)));
        if method != "NSURLAuthenticationMethodServerTrust" {
            return answer(DEFAULT_HANDLING, std::ptr::null_mut());
        }
        let trust = send0(space, sel!(serverTrust));
        if trust.is_null() || SecTrustEvaluateWithError(trust, std::ptr::null_mut()) {
            return answer(DEFAULT_HANDLING, std::ptr::null_mut());
        }
        let Some(label) = portal_label(webview as *mut c_void) else {
            return answer(DEFAULT_HANDLING, std::ptr::null_mut());
        };
        let host = string(send0(space, sel!(host)));
        if super::is_trusted(&label, &host) {
            let Some(cls) = AnyClass::get(c"NSURLCredential") else {
                return answer(DEFAULT_HANDLING, std::ptr::null_mut());
            };
            let cred = send1(
                cls as *const AnyClass as *mut c_void,
                sel!(credentialForTrust:),
                trust,
            );
            return answer(USE_CREDENTIAL, cred as *mut AnyObject);
        }
        super::record_error(&label, &host);
        answer(CANCEL, std::ptr::null_mut())
    }

    pub unsafe fn install_on(webview: *mut c_void) {
        let delegate = send0(webview, sel!(navigationDelegate));
        if delegate.is_null() {
            return;
        }
        let cls = send0(delegate, sel!(class)) as *mut AnyClass;
        if cls.is_null() {
            return;
        }
        let imp: objc2::runtime::Imp = std::mem::transmute(
            challenge
                as unsafe extern "C-unwind" fn(
                    *mut AnyObject,
                    Sel,
                    *mut AnyObject,
                    *mut AnyObject,
                    *mut Handler,
                ),
        );
        let _ = objc2::ffi::class_addMethod(
            cls,
            sel!(webView:didReceiveAuthenticationChallenge:completionHandler:),
            imp,
            c"v@:@@@?".as_ptr(),
        );
    }
}
