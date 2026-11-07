//! Document Properties ▸ Initial View and reading options: how the document opens (navigation
//! panel, page layout, magnification, page; window and interface options) and its language and
//! binding (ISO 32000-2 §7.7.2 Table 29, §12.2 viewer preferences).

use pdfcraft_cos::{Dict, Document, Object, PdfString};

use crate::OrganizeError;

/// The navigation panel shown on opening (`/PageMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Navigation {
    #[default]
    PageOnly,
    Bookmarks,
    Pages,
    Attachments,
    Layers,
}

/// `/PageLayout`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Default,
    SinglePage,
    SinglePageContinuous,
    TwoUp,
    TwoUpContinuous,
    TwoUpCoverPage,
    TwoUpContinuousCoverPage,
}

/// The magnification of the opening destination.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Magnification {
    #[default]
    Default,
    ActualSize,
    FitPage,
    FitWidth,
    FitHeight,
    FitVisible,
    /// Percent (100 = actual size).
    Percent(f64),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InitialView {
    pub navigation: Navigation,
    pub layout: Layout,
    pub magnification: Magnification,
    /// 0-based page opened to.
    pub page: usize,
    pub fit_window: bool,
    pub center_window: bool,
    pub full_screen: bool,
    /// Show the document title (not the file name) in the window title.
    pub display_title: bool,
    pub hide_menubar: bool,
    pub hide_toolbar: bool,
    pub hide_window_ui: bool,
    /// `/Lang` (e.g. "en-US").
    pub language: Option<String>,
    /// Right-edge binding (`/Direction /R2L`).
    pub right_to_left: bool,
    /// The navigation panel on leaving full screen (`/ViewerPreferences /NonFullScreenPageMode`,
    /// ISO 32000-2 Table 147), read only when `full_screen` is set. It is read, not written:
    /// [`set_initial_view`] leaves the entry as it is.
    pub non_full_screen: Navigation,
}

fn catalog(doc: &Document) -> Result<(pdfcraft_cos::ObjRef, Dict), OrganizeError> {
    let r = doc.root().ok_or(OrganizeError::NoPageTree)?;
    Ok((r, doc.get(r).as_dict().cloned().ok_or(OrganizeError::NoPageTree)?))
}

/// The document's initial view as stored.
pub fn initial_view(doc: &Document) -> InitialView {
    let Ok((_, c)) = catalog(doc) else { return InitialView::default() };
    let name = |d: &Dict, k: &[u8]| d.get(k).map(|o| doc.resolve(o)).and_then(|o| o.as_name().map(<[u8]>::to_vec));
    let mut v = InitialView {
        navigation: match name(&c, b"PageMode").as_deref() {
            Some(b"UseOutlines") => Navigation::Bookmarks,
            Some(b"UseThumbs") => Navigation::Pages,
            Some(b"UseAttachments") => Navigation::Attachments,
            Some(b"UseOC") => Navigation::Layers,
            _ => Navigation::PageOnly,
        },
        full_screen: name(&c, b"PageMode").as_deref() == Some(b"FullScreen"),
        layout: match name(&c, b"PageLayout").as_deref() {
            Some(b"SinglePage") => Layout::SinglePage,
            Some(b"OneColumn") => Layout::SinglePageContinuous,
            Some(b"TwoPageLeft") => Layout::TwoUp,
            Some(b"TwoColumnLeft") => Layout::TwoUpContinuous,
            Some(b"TwoPageRight") => Layout::TwoUpCoverPage,
            Some(b"TwoColumnRight") => Layout::TwoUpContinuousCoverPage,
            _ => Layout::Default,
        },
        language: c.get(b"Lang").map(|o| doc.resolve(o)).and_then(|o| o.as_string().map(|s| s.to_text())).filter(|s| !s.is_empty()),
        ..InitialView::default()
    };
    if let Some(vp) = c.get(b"ViewerPreferences").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()) {
        let flag = |k: &[u8]| matches!(vp.get(k).map(|o| doc.resolve(o)).as_deref(), Some(Object::Bool(true)));
        v.fit_window = flag(b"FitWindow");
        v.center_window = flag(b"CenterWindow");
        v.display_title = flag(b"DisplayDocTitle");
        v.hide_menubar = flag(b"HideMenubar");
        v.hide_toolbar = flag(b"HideToolbar");
        v.hide_window_ui = flag(b"HideWindowUI");
        v.right_to_left = name(&vp, b"Direction").as_deref() == Some(b"R2L");
        // Table 147: meaningful only with `/PageMode /FullScreen`, and only these four values.
        if v.full_screen {
            v.non_full_screen = match name(&vp, b"NonFullScreenPageMode").as_deref() {
                Some(b"UseOutlines") => Navigation::Bookmarks,
                Some(b"UseThumbs") => Navigation::Pages,
                Some(b"UseOC") => Navigation::Layers,
                _ => Navigation::PageOnly,
            };
        }
    }
    // The opening destination: an explicit array, a GoTo action's `/D`, or a named destination (a
    // name or string, §12.3.2.4) as the explicit one it names, so that the page and magnification
    // read here are where the document opens. A name that names nothing reads as page 1.
    let dest = c.get(b"OpenAction").map(|o| doc.resolve(o)).and_then(|o| match &*o {
        Object::Array(_) | Object::Name(_) | Object::String(_) => crate::outline::resolve_dest(doc, &o),
        Object::Dict(d) if d.name(b"S") == Some(b"GoTo") => d.get(b"D").and_then(|x| crate::outline::resolve_dest(doc, x)),
        _ => None,
    });
    if let Some(d) = dest {
        let pages = crate::walk(doc).unwrap_or_default();
        v.page = d
            .first()
            .and_then(|p| match p {
                Object::Ref(r) => pages.iter().position(|(pr, _)| pr == r),
                Object::Int(i) => usize::try_from(*i).ok(),
                _ => None,
            })
            .unwrap_or(0);
        let kind = d.get(1).and_then(|k| k.as_name().map(<[u8]>::to_vec));
        v.magnification = match kind.as_deref() {
            Some(b"Fit") => Magnification::FitPage,
            Some(b"FitH") => Magnification::FitWidth,
            Some(b"FitV") => Magnification::FitHeight,
            Some(b"FitB" | b"FitBH" | b"FitBV") => Magnification::FitVisible,
            Some(b"XYZ") => match d.get(4).and_then(|z| z.as_f64()) {
                Some(z) if z > 0.0 && (z - 1.0).abs() < 1e-6 => Magnification::ActualSize,
                Some(z) if z > 0.0 => Magnification::Percent(z * 100.0),
                _ => Magnification::Default,
            },
            _ => Magnification::Default,
        };
    }
    v
}

/// Whether viewers should show the document title rather than the file name
/// (`/ViewerPreferences /DisplayDocTitle`), without reading the rest of the initial view.
pub fn displays_doc_title(doc: &Document) -> bool {
    let Ok((_, c)) = catalog(doc) else { return false };
    c.get(b"ViewerPreferences")
        .map(|o| doc.resolve(o))
        .and_then(|o| o.as_dict().map(|vp| matches!(vp.get(b"DisplayDocTitle").map(|x| doc.resolve(x)).as_deref(), Some(Object::Bool(true)))))
        .unwrap_or(false)
}

/// Write the initial view. An opening action that isn't a plain destination (a script, say) is
/// kept unless the page or magnification asks for a destination, and an opening action whose page
/// and magnification `v` leaves as [`initial_view`] reads them is kept as it is.
pub fn set_initial_view(doc: &mut Document, v: &InitialView) -> Result<(), OrganizeError> {
    let same_destination = {
        let now = initial_view(doc);
        now.page == v.page && now.magnification == v.magnification
    };
    let (root, c) = catalog(doc)?;
    let pages = crate::walk(doc)?;
    if v.page >= pages.len() {
        return Err(OrganizeError::NoSuchPage(v.page));
    }
    let page_ref = pages[v.page].0;
    let mut vp = c.get(b"ViewerPreferences").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()).unwrap_or_default();
    for (k, on) in [
        (&b"FitWindow"[..], v.fit_window),
        (b"CenterWindow", v.center_window),
        (b"DisplayDocTitle", v.display_title),
        (b"HideMenubar", v.hide_menubar),
        (b"HideToolbar", v.hide_toolbar),
        (b"HideWindowUI", v.hide_window_ui),
    ] {
        if on {
            vp.set(k.to_vec(), Object::Bool(true));
        } else {
            vp.remove(k);
        }
    }
    if v.right_to_left {
        vp.set(b"Direction".to_vec(), Object::name("R2L"));
    } else {
        vp.remove(b"Direction");
    }
    // A destination, explicit or named (§12.3.2.4), or a GoTo action to one.
    let open_action_is_dest = match c.get(b"OpenAction").map(|o| doc.resolve(o)) {
        None => true,
        Some(o) => {
            matches!(&*o, Object::Array(_) | Object::Name(_) | Object::String(_)) || o.as_dict().is_some_and(|d| d.name(b"S") == Some(b"GoTo"))
        }
    };
    let dest = match v.magnification {
        Magnification::Default if v.page == 0 => None,
        Magnification::Default => Some(vec![Object::Ref(page_ref), Object::name("XYZ"), Object::Null, Object::Null, Object::Null]),
        Magnification::ActualSize => Some(vec![Object::Ref(page_ref), Object::name("XYZ"), Object::Null, Object::Null, Object::Real(1.0)]),
        Magnification::Percent(p) => {
            Some(vec![Object::Ref(page_ref), Object::name("XYZ"), Object::Null, Object::Null, Object::Real((p / 100.0).clamp(0.01, 64.0))])
        }
        Magnification::FitPage => Some(vec![Object::Ref(page_ref), Object::name("Fit")]),
        Magnification::FitWidth => Some(vec![Object::Ref(page_ref), Object::name("FitH"), Object::Null]),
        Magnification::FitHeight => Some(vec![Object::Ref(page_ref), Object::name("FitV"), Object::Null]),
        Magnification::FitVisible => Some(vec![Object::Ref(page_ref), Object::name("FitB")]),
    };
    doc.update_dict(root, |c| {
        let mode = if v.full_screen {
            Some("FullScreen")
        } else {
            match v.navigation {
                Navigation::PageOnly => None,
                Navigation::Bookmarks => Some("UseOutlines"),
                Navigation::Pages => Some("UseThumbs"),
                Navigation::Attachments => Some("UseAttachments"),
                Navigation::Layers => Some("UseOC"),
            }
        };
        match mode {
            Some(m) => c.set(b"PageMode".to_vec(), Object::name(m)),
            None => {
                c.remove(b"PageMode");
            }
        }
        let layout = match v.layout {
            Layout::Default => None,
            Layout::SinglePage => Some("SinglePage"),
            Layout::SinglePageContinuous => Some("OneColumn"),
            Layout::TwoUp => Some("TwoPageLeft"),
            Layout::TwoUpContinuous => Some("TwoColumnLeft"),
            Layout::TwoUpCoverPage => Some("TwoPageRight"),
            Layout::TwoUpContinuousCoverPage => Some("TwoColumnRight"),
        };
        match layout {
            Some(l) => c.set(b"PageLayout".to_vec(), Object::name(l)),
            None => {
                c.remove(b"PageLayout");
            }
        }
        if vp.is_empty() {
            c.remove(b"ViewerPreferences");
        } else {
            c.set(b"ViewerPreferences".to_vec(), Object::Dict(vp.clone()));
        }
        match &v.language {
            Some(l) if !l.trim().is_empty() => c.set(b"Lang".to_vec(), PdfString::text(l.trim())),
            _ => {
                c.remove(b"Lang");
            }
        }
        match (&dest, open_action_is_dest) {
            // The opening page and magnification are as read: the action stays as written, with
            // its position or named destination (§12.3.2.4) that the view model doesn't hold.
            _ if same_destination => {}
            (Some(d), _) => c.set(b"OpenAction".to_vec(), Object::Array(d.clone())),
            (None, true) => {
                c.remove(b"OpenAction");
            }
            // A script or other action stays.
            (None, false) => {}
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_cos::{SaveOptions, write_full};
    use std::sync::Arc;

    fn flags(view: InitialView) -> [bool; 6] {
        [view.fit_window, view.center_window, view.display_title, view.hide_menubar, view.hide_toolbar, view.hide_window_ui]
    }

    #[test]
    fn initial_view_resolves_indirect_boolean_preferences() {
        let keys: [&[u8]; 6] = [b"FitWindow", b"CenterWindow", b"DisplayDocTitle", b"HideMenubar", b"HideToolbar", b"HideWindowUI"];
        for enabled in [true, false] {
            for indirect_dictionary in [false, true] {
                let mut doc = Document::new_empty();
                let boolean = doc.add(Object::Bool(enabled));
                let mut preferences = Dict::new();
                for key in keys {
                    preferences.set(key.to_vec(), Object::Ref(boolean));
                }
                let preferences = if indirect_dictionary { Object::Ref(doc.add(preferences)) } else { Object::Dict(preferences) };
                let root = doc.root().unwrap();
                doc.update_dict(root, |c| c.set(b"ViewerPreferences".to_vec(), preferences)).unwrap();
                assert_eq!(flags(initial_view(&doc)), [enabled; 6]);
                let saved = write_full(&doc, &SaveOptions::default()).unwrap();
                let reopened = Document::open(Arc::new(saved)).unwrap();
                assert_eq!(flags(initial_view(&reopened)), [enabled; 6]);
                assert_eq!(displays_doc_title(&reopened), enabled);
            }
        }
    }

    #[test]
    fn initial_view_malformed_boolean_preferences_keep_false_defaults() {
        for value in [Object::Null, Object::Int(1), Object::name("true"), Object::Ref(pdfcraft_cos::ObjRef::new(999_999, 0))] {
            let mut doc = Document::new_empty();
            let value = doc.add(value);
            let mut preferences = Dict::new();
            preferences.set(b"FitWindow".to_vec(), Object::Ref(value));
            let root = doc.root().unwrap();
            doc.update_dict(root, |c| c.set(b"ViewerPreferences".to_vec(), Object::Dict(preferences))).unwrap();
            assert_eq!(flags(initial_view(&doc)), [false; 6]);
        }
    }

    /// A catalog with `/PageMode page_mode` and `/ViewerPreferences << /NonFullScreenPageMode mode >>`.
    fn with_modes(page_mode: &str, mode: Object) -> Document {
        let mut doc = Document::new_empty();
        let mode = doc.add(mode);
        let mut preferences = Dict::new();
        preferences.set(b"NonFullScreenPageMode".to_vec(), Object::Ref(mode));
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| {
            c.set(b"PageMode".to_vec(), Object::name(page_mode));
            c.set(b"ViewerPreferences".to_vec(), Object::Dict(preferences));
        })
        .unwrap();
        doc
    }

    #[test]
    fn non_full_screen_page_mode_is_read_with_full_screen_only() {
        for (mode, want) in [
            ("UseOutlines", Navigation::Bookmarks),
            ("UseThumbs", Navigation::Pages),
            ("UseOC", Navigation::Layers),
            ("UseNone", Navigation::PageOnly),
            // Not a value Table 147 lists.
            ("UseAttachments", Navigation::PageOnly),
            ("FullScreen", Navigation::PageOnly),
        ] {
            let v = initial_view(&with_modes("FullScreen", Object::name(mode)));
            assert!(v.full_screen);
            assert_eq!(v.non_full_screen, want, "{mode}");
            // Ignored unless the document opens in full screen.
            let v = initial_view(&with_modes("UseThumbs", Object::name(mode)));
            assert_eq!((v.navigation, v.non_full_screen), (Navigation::Pages, Navigation::PageOnly), "{mode}");
        }
        assert_eq!(initial_view(&with_modes("FullScreen", Object::Int(1))).non_full_screen, Navigation::PageOnly);
    }
}
