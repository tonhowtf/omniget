use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

fn trusted() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    static T: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

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

pub fn cert_error(label: &str) -> Option<(String, String)> {
    errors().lock().ok().and_then(|e| e.get(label).cloned())
}

pub fn clear_error(label: &str) {
    if let Ok(mut e) = errors().lock() {
        e.remove(label);
    }
}

#[cfg(target_os = "macos")]
fn record_error(label: &str, host: &str) {
    let url = super::host()
        .state_of(label)
        .map(|s| s.url)
        .unwrap_or_default();
    if let Ok(mut e) = errors().lock() {
        e.insert(label.to_string(), (host.to_string(), url));
    }
    super::host().set(label, |s| s.loading = false);
    super::host().fire_loads(label);
    if let Some(app) = super::app() {
        super::host().emit(&app, label);
    }
}

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
