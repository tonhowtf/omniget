//! Page boxes (ISO 32000-2 §14.11.2): Acrobat's Set Page Boxes and Crop, execution plan M4.4.
//!
//! The crop, bleed, trim and art boxes default to their "parent" box (crop → media; the others
//! → crop) and are clipped to the media box. Margins are measured inward from the media box in
//! default user space (unrotated), as Acrobat's dialog does.

use pdfcraft_cos::{Document, Object};

use crate::{OrganizeError, check, walk};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageBox {
    Media,
    Crop,
    Bleed,
    Trim,
    Art,
}

impl PageBox {
    pub fn key(self) -> &'static [u8] {
        match self {
            PageBox::Media => b"MediaBox",
            PageBox::Crop => b"CropBox",
            PageBox::Bleed => b"BleedBox",
            PageBox::Trim => b"TrimBox",
            PageBox::Art => b"ArtBox",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().trim_end_matches("box") {
            "media" => Some(Self::Media),
            "crop" => Some(Self::Crop),
            "bleed" => Some(Self::Bleed),
            "trim" => Some(Self::Trim),
            "art" => Some(Self::Art),
            _ => None,
        }
    }
}

/// What to set a box to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxSpec {
    /// Back to its default (the parent box).
    Remove,
    /// An absolute rectangle in default user space.
    Rect([f64; 4]),
    /// Margins inward from the media box: left, bottom, right, top (points).
    Margins([f64; 4]),
}

fn rect_of(o: Option<&Object>, doc: &Document) -> Option<[f64; 4]> {
    let o = doc.resolve(o?);
    let v: Vec<f64> = o.as_array()?.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
    (v.len() == 4 && v.iter().all(|x| x.is_finite())).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// The effective boxes of every page: media, crop, bleed, trim, art.
pub fn page_boxes(doc: &Document) -> Result<Vec<[[f64; 4]; 5]>, OrganizeError> {
    Ok(walk(doc)?
        .iter()
        .map(|(page, inherited)| {
            let obj = doc.get(*page);
            let d = obj.as_dict().cloned().unwrap_or_default();
            let own = |k: &[u8]| rect_of(d.get(k).or_else(|| inherited.get(k)), doc);
            let media = own(b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
            let crop = own(b"CropBox").map(|c| intersect(c, media)).unwrap_or(media);
            let child = |k: &[u8]| rect_of(d.get(k), doc).map(|c| intersect(c, media)).unwrap_or(crop);
            [media, crop, child(b"BleedBox"), child(b"TrimBox"), child(b"ArtBox")]
        })
        .collect())
}

/// Set a box on `pages` (0-based). Margins apply to each page's own media box.
pub fn set_page_box(doc: &mut Document, pages: &[usize], which: PageBox, spec: BoxSpec) -> Result<(), OrganizeError> {
    let all = walk(doc)?;
    check(pages, all.len())?;
    let boxes = page_boxes(doc)?;
    let bad = |m: &str| OrganizeError::InvalidBox(m.to_string());
    if which == PageBox::Media && spec == BoxSpec::Remove {
        return Err(bad("the media box can't be removed"));
    }
    // Validate every page first so a failure changes nothing.
    let mut plan = Vec::new();
    for &i in pages {
        let media = boxes[i][0];
        let rect = match spec {
            BoxSpec::Remove => None,
            BoxSpec::Rect(r) => {
                if !r.iter().all(|x| x.is_finite()) {
                    return Err(bad("the rectangle is not a number"));
                }
                let r = [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])];
                Some(if which == PageBox::Media { r } else { intersect(r, media) })
            }
            BoxSpec::Margins(m) => {
                if !m.iter().all(|x| x.is_finite() && *x >= 0.0) {
                    return Err(bad("margins must be zero or more"));
                }
                Some([media[0] + m[0], media[1] + m[1], media[2] - m[2], media[3] - m[3]])
            }
        };
        if let Some(r) = rect
            && (r[2] - r[0] < 1.0 || r[3] - r[1] < 1.0)
        {
            return Err(bad(&format!("page {} would be smaller than a point", i + 1)));
        }
        plan.push((all[i].0, all[i].1.contains(which.key()), rect, media));
    }
    for (page, inherited, rect, media) in plan {
        doc.update_dict(page, |d| match rect {
            Some(r) => d.set(which.key().to_vec(), Object::Array(r.iter().map(|v| Object::Real(*v)).collect())),
            // An inherited crop box would come back: pin the default explicitly instead.
            None if inherited => d.set(which.key().to_vec(), Object::Array(media.iter().map(|v| Object::Real(*v)).collect())),
            None => {
                d.remove(which.key());
            }
        })?;
    }
    Ok(())
}
