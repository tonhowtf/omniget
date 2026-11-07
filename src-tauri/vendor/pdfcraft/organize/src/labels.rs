//! Page labels (ISO 32000-2 §12.4.2): "i, ii, iii, 1, 2, A-1…", execution plan M4.4.
//!
//! Labels are a number tree of ranges: from page `start` on, use a numbering `style`, a
//! `prefix` and a first number. [`number_pages`] is Acrobat's "Number pages": relabel pages
//! `from..=to`, and keep every later page showing the label it had before. The tree is written
//! back as one flat `/Nums` array, with ranges that merely continue the previous one merged.

use std::collections::HashSet;

use pdfcraft_cos::page_labels::{MAX_LABEL_BYTES, MAX_LABEL_TREE_DEPTH, MAX_LABEL_TREE_WORK, MAX_PREFIX_BYTES, MAX_PREFIX_TOTAL_BYTES, alpha, roman};
use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::{OrganizeError, page_count, pages_root};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelStyle {
    /// 1, 2, 3
    Decimal,
    /// I, II, III
    UpperRoman,
    /// i, ii, iii
    LowerRoman,
    /// A … Z, AA … ZZ
    UpperAlpha,
    /// a … z, aa … zz
    LowerAlpha,
    /// Prefix only (no number).
    None,
}

impl LabelStyle {
    fn from_name(n: &[u8]) -> Self {
        match n {
            b"D" => Self::Decimal,
            b"R" => Self::UpperRoman,
            b"r" => Self::LowerRoman,
            b"A" => Self::UpperAlpha,
            b"a" => Self::LowerAlpha,
            _ => Self::None,
        }
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Self::Decimal => Some("D"),
            Self::UpperRoman => Some("R"),
            Self::LowerRoman => Some("r"),
            Self::UpperAlpha => Some("A"),
            Self::LowerAlpha => Some("a"),
            Self::None => None,
        }
    }

    /// Format `n` (≥ 1) in this style for previews.
    /// A numeral exceeding 1024 bytes falls back to decimal; editing and document label
    /// queries use the checked formatter and return an error instead of storing a fallback.
    pub fn format(self, n: u32) -> String {
        self.format_checked(n, MAX_LABEL_BYTES).unwrap_or_else(|_| n.to_string())
    }

    fn format_checked(self, n: u32, max_bytes: usize) -> Result<String, OrganizeError> {
        let mut out = match self {
            Self::Decimal => Some(n.to_string()),
            Self::UpperRoman | Self::LowerRoman => roman(u64::from(n), max_bytes),
            Self::UpperAlpha | Self::LowerAlpha => alpha(u64::from(n), max_bytes),
            Self::None => Some(String::new()),
        }
        .ok_or_else(|| invalid_label("a label exceeds 1024 UTF-8 bytes"))?;
        if out.len() > max_bytes {
            return Err(invalid_label("a label exceeds 1024 UTF-8 bytes"));
        }
        if matches!(self, Self::UpperRoman | Self::UpperAlpha) {
            out.make_ascii_uppercase();
        }
        Ok(out)
    }
}

/// One labelling range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelRange {
    /// First page (0-based) of the range.
    pub start: usize,
    pub style: LabelStyle,
    pub prefix: String,
    /// The number of the range's first page (`/St`, at least 1).
    pub first: u32,
}

impl LabelRange {
    fn number(&self, page: usize) -> Result<u32, OrganizeError> {
        if self.style == LabelStyle::None {
            return Ok(self.first); // /St is ignored when the range has no numbering style.
        }
        let offset = page.checked_sub(self.start).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| invalid_label("a page number is too large"))?;
        self.first.checked_add(offset).ok_or_else(|| invalid_label("a page number overflows"))
    }

    fn label(&self, page: usize) -> Result<String, OrganizeError> {
        let remaining = MAX_LABEL_BYTES.checked_sub(self.prefix.len()).ok_or_else(|| invalid_label("a prefix exceeds 1024 UTF-8 bytes"))?;
        let number = self.style.format_checked(self.number(page)?, remaining)?;
        let mut label = self.prefix.clone();
        label.push_str(&number);
        Ok(label)
    }
}

fn invalid_label(reason: &str) -> OrganizeError {
    OrganizeError::Invalid(format!("Page labels could not be processed: {reason}"))
}

fn default_range() -> LabelRange {
    LabelRange { start: 0, style: LabelStyle::Decimal, prefix: String::new(), first: 1 }
}

/// The document's label ranges, sorted by start page (empty when it has no `/PageLabels`).
/// This compatibility query returns an empty list for a tree exceeding resource limits.
/// Label display and edits use the checked reader and return an error instead.
pub fn page_label_ranges(doc: &Document) -> Vec<LabelRange> {
    checked_ranges(doc).unwrap_or_default()
}

// Borrow direct objects instead of cloning complete untrusted dictionaries or arrays.
fn with_label_object<T>(doc: &Document, object: &Object, f: impl FnOnce(&Object) -> Result<T, OrganizeError>) -> Result<T, OrganizeError> {
    let Object::Ref(r) = object else { return f(object) };
    let mut seen = HashSet::from([*r]);
    let mut object = doc.get(*r);
    while let Object::Ref(r) = object.as_ref() {
        if seen.len() >= MAX_LABEL_TREE_DEPTH || !seen.insert(*r) {
            return Err(invalid_label("a number tree reference is cyclic or too deeply nested"));
        }
        object = doc.get(*r);
    }
    f(&object)
}

fn label_work(left: &mut usize, count: usize) -> Result<(), OrganizeError> {
    *left = left.checked_sub(count).ok_or_else(|| invalid_label("the number tree has too many entries"))?;
    Ok(())
}

fn read_label_node(
    doc: &Document,
    node: &Object,
    depth: usize,
    left: &mut usize,
    seen: &mut HashSet<ObjRef>,
    entries: &mut Vec<LabelRange>,
    prefix_left: &mut usize,
) -> Result<(), OrganizeError> {
    label_work(left, 1)?;
    if depth > MAX_LABEL_TREE_DEPTH {
        return Err(invalid_label("the number tree is too deep"));
    }
    // Only indirect nodes can be reached twice; key them by reference, not by the address of
    // a resolved object (a missing reference resolves to a fresh, short-lived `Null`).
    if let Object::Ref(r) = node
        && !seen.insert(*r)
    {
        return Err(invalid_label("the number tree repeats a node or contains a cycle"));
    }
    with_label_object(doc, node, |node| {
        let Some(d) = node.as_dict() else { return Ok(()) };
        if let Some(nums) = d.get(b"Nums") {
            with_label_object(doc, nums, |nums| {
                let Some(nums) = nums.as_array() else { return Ok(()) };
                label_work(left, nums.len().div_ceil(2))?;
                for pair in nums.as_chunks::<2>().0 {
                    let Some(start) = pair[0].as_int().and_then(|n| usize::try_from(n).ok()) else { continue };
                    with_label_object(doc, &pair[1], |spec| {
                        let Some(spec) = spec.as_dict() else { return Ok(()) };
                        let prefix = match spec.get(b"P") {
                            Some(prefix) => with_label_object(doc, prefix, |p| match p.as_string() {
                                Some(p) if p.bytes.len() > MAX_PREFIX_BYTES => Err(invalid_label("a raw prefix exceeds 2050 bytes")),
                                Some(p) => Ok(p.to_text()),
                                None => Ok(String::new()),
                            })?,
                            None => String::new(),
                        };
                        if prefix.len() > MAX_LABEL_BYTES {
                            return Err(invalid_label("a decoded prefix exceeds 1024 UTF-8 bytes"));
                        }
                        // A prefix object shared by many ranges is copied into each: count every copy.
                        *prefix_left =
                            prefix_left.checked_sub(prefix.len()).ok_or_else(|| invalid_label("decoded prefixes exceed 4 MiB in total"))?;
                        let style = spec.name(b"S").map_or(LabelStyle::None, LabelStyle::from_name);
                        let first = spec.int(b"St").unwrap_or(1).max(1);
                        let first = if style == LabelStyle::None {
                            // /St has no effect without /S; keep the existing compatibility clamp.
                            first.min(i64::from(u32::MAX)) as u32
                        } else {
                            u32::try_from(first).map_err(|_| invalid_label("a first number exceeds the supported integer range"))?
                        };
                        entries.push(LabelRange { start, style, prefix, first });
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
        }
        if let Some(kids) = d.get(b"Kids") {
            with_label_object(doc, kids, |kids| {
                let Some(kids) = kids.as_array() else { return Ok(()) };
                label_work(left, kids.len())?;
                if !kids.is_empty() && depth == MAX_LABEL_TREE_DEPTH {
                    return Err(invalid_label("the number tree is too deep"));
                }
                // Recursion is bounded before descending, and arrays are borrowed throughout.
                for kid in kids {
                    read_label_node(doc, kid, depth + 1, left, seen, entries, prefix_left)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    })
}

fn checked_ranges(doc: &Document) -> Result<Vec<LabelRange>, OrganizeError> {
    let Some(root) = doc.root() else { return Ok(Vec::new()) };
    let catalog = doc.get(root);
    let Some(tree) = catalog.as_dict().and_then(|c| c.get(b"PageLabels")) else { return Ok(Vec::new()) };
    let mut entries = Vec::new();
    let (mut left, mut prefix_left) = (MAX_LABEL_TREE_WORK, MAX_PREFIX_TOTAL_BYTES);
    read_label_node(doc, tree, 0, &mut left, &mut HashSet::new(), &mut entries, &mut prefix_left)?;
    // A start page given twice keeps its later range, as readers that overwrite do (the sort is
    // stable, so the later one is last among equals).
    entries.sort_by_key(|r| r.start);
    let mut unique: Vec<LabelRange> = Vec::with_capacity(entries.len());
    for r in entries {
        match unique.last_mut() {
            Some(last) if last.start == r.start => *last = r,
            _ => unique.push(r),
        }
    }
    Ok(unique)
}

/// A view of every page's label: the sorted ranges and the page count, with no string kept per
/// page. [`PageLabels::label`] builds one label; [`PageLabels::iter`] builds them one at a time, so
/// memory follows the ranges, not the pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageLabels {
    ranges: Vec<LabelRange>,
    pages: usize,
}

impl PageLabels {
    /// Reads the document's `/PageLabels` and page count. A tree that is cyclic, too deep or too
    /// large is an error.
    pub fn new(doc: &Document) -> Result<Self, OrganizeError> {
        Ok(Self { ranges: checked_ranges(doc)?, pages: page_count(doc)? })
    }

    /// The number of pages the labels cover.
    pub fn pages(&self) -> usize {
        self.pages
    }

    /// The label of page `page` (0-based), as a viewer shows it: a page with no range, or with an
    /// empty range label, shows its physical number. Errors when the range's label cannot be
    /// formatted (over 1,024 UTF-8 bytes, or a number past 32 bits).
    pub fn label(&self, page: usize) -> Result<String, OrganizeError> {
        if page >= self.pages {
            return Err(OrganizeError::NoSuchPage(page));
        }
        let covering = self.ranges.partition_point(|r| r.start <= page);
        let label = match covering.checked_sub(1).and_then(|i| self.ranges.get(i)) {
            Some(range) => range.label(page)?,
            None => String::new(),
        };
        Ok(if label.is_empty() { (page + 1).to_string() } else { label })
    }

    /// Every page's label, in page order, one at a time. Reading is lenient: a label that cannot
    /// be formatted shows its physical page number, and the other ranges keep their labels.
    pub fn iter(&self) -> impl Iterator<Item = String> + '_ {
        (0..self.pages).map(|page| self.label(page).unwrap_or_else(|_| (page + 1).to_string()))
    }
}

/// Every page's label, as a viewer shows it. This builds one string per page; for large
/// documents, read single labels or stream them through [`PageLabels`].
pub fn page_labels(doc: &Document) -> Result<Vec<String>, OrganizeError> {
    Ok(PageLabels::new(doc)?.iter().collect())
}

/// Strict check for an edit: every label that the sorted, unique `ranges` produce over `pages`
/// pages must fit. A range is checked from its largest number, so the work follows the ranges
/// (and at most a thousand numbers at each end of a Roman range), not the pages.
fn check_spans(ranges: &[LabelRange], pages: usize) -> Result<(), OrganizeError> {
    let Some(last_page) = pages.checked_sub(1) else { return Ok(()) };
    for (i, r) in ranges.iter().enumerate() {
        // Prefix-only labels have no number to check; a range past the last page is never shown.
        if r.start > last_page || r.style == LabelStyle::None {
            continue;
        }
        let end = ranges.get(i + 1).map_or(last_page, |next| next.start.saturating_sub(1).min(last_page));
        let lo = u64::from(r.first);
        let hi = lo.saturating_add(u64::try_from(end.saturating_sub(r.start)).unwrap_or(u64::MAX));
        let prefix = u64::try_from(r.prefix.len()).unwrap_or(u64::MAX);
        let remaining = (MAX_LABEL_BYTES as u64).checked_sub(prefix).ok_or_else(|| invalid_label("a prefix exceeds 1024 UTF-8 bytes"))?;
        // Numbers past u32::MAX are never formatted, so a label that is too long comes first.
        if longest_numeral(r.style, lo, hi.min(u64::from(u32::MAX))) > remaining {
            return Err(invalid_label("a label exceeds 1024 UTF-8 bytes"));
        }
        if hi > u64::from(u32::MAX) {
            return Err(invalid_label("a page number overflows"));
        }
    }
    Ok(())
}

/// The longest numeral `style` writes for the numbers `lo..=hi` (`1 <= lo <= hi`), in bytes.
fn longest_numeral(style: LabelStyle, lo: u64, hi: u64) -> u64 {
    match style {
        // Decimal and alphabetic numerals only grow with the number, so the last one is longest.
        LabelStyle::Decimal => u64::from(hi.checked_ilog10().unwrap_or(0)) + 1,
        LabelStyle::UpperAlpha | LabelStyle::LowerAlpha => hi.saturating_sub(1) / 26 + 1,
        LabelStyle::UpperRoman | LabelStyle::LowerRoman => longest_roman(lo, hi),
        LabelStyle::None => 0,
    }
}

/// The byte length of [`roman`]'s numeral for `n`: `n / 1000` copies of `m`, then the hundreds,
/// tens and units, each a table lookup.
fn roman_len(n: u64) -> u64 {
    // Lengths of i, ii, iii, iv, v, vi, vii, viii, ix (and the same pattern for tens and hundreds).
    const UNITS: [u64; 10] = [0, 1, 2, 3, 2, 1, 2, 3, 4, 2];
    let digit = |place: u64| UNITS.get(usize::try_from(place % 10).unwrap_or_default()).copied().unwrap_or_default();
    n / 1000 + digit(n / 100) + digit(n / 10) + digit(n)
}

/// The longest Roman numeral among `lo..=hi`, in bytes. Within one thousand the longest numeral is
/// the one ending in 888, and every whole thousand between the ends contains it, so only the
/// partial thousands at either end are read.
fn longest_roman(lo: u64, hi: u64) -> u64 {
    let (first, last) = (lo / 1000, hi / 1000);
    if first == last {
        return (lo..=hi).map(roman_len).max().unwrap_or_default();
    }
    let head = (lo..=first.saturating_mul(1000).saturating_add(999)).map(roman_len).max().unwrap_or_default();
    let tail = (last.saturating_mul(1000)..=hi).map(roman_len).max().unwrap_or_default();
    // Each whole thousand between the ends peaks at its thousand plus 12 (for 888); the last is largest.
    let middle = if last.saturating_sub(first) >= 2 { last.saturating_sub(1).saturating_add(12) } else { 0 };
    head.max(tail).max(middle)
}

/// Replace all label ranges (an empty list removes `/PageLabels`).
pub fn set_page_label_ranges(doc: &mut Document, ranges: &[LabelRange]) -> Result<(), OrganizeError> {
    pages_root(doc)?;
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    if ranges.is_empty() {
        doc.update_dict(root, |c| {
            c.remove(b"PageLabels");
        })?;
        return Ok(());
    }
    if ranges.len() >= MAX_LABEL_TREE_WORK {
        return Err(invalid_label("the number tree has too many entries"));
    }
    // Validate every range and all effective labels before changing any document object.
    for r in ranges {
        if r.first == 0 || i64::try_from(r.start).is_err() {
            return Err(invalid_label("range starts and first numbers must be valid positive PDF integers"));
        }
        r.label(r.start)?;
    }
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|r| r.start);
    if sorted.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a.start == b.start)) {
        return Err(invalid_label("range start pages must be unique"));
    }
    check_spans(&sorted, page_count(doc)?)?;
    let mut nums = Vec::new();
    for r in &sorted {
        let mut spec = Dict::new();
        if let Some(s) = r.style.name() {
            spec.set(b"S".to_vec(), Object::name(s));
        }
        if !r.prefix.is_empty() {
            spec.set(b"P".to_vec(), Object::String(PdfString::text(&r.prefix)));
        }
        if r.first != 1 {
            spec.set(b"St".to_vec(), Object::Int(i64::from(r.first)));
        }
        nums.push(Object::Int(r.start as i64));
        // Indirect, as Acrobat writes them: a direct dictionary is valid too, but some readers
        // (pdfannots2json, through unipdf) skip it and show physical page numbers (#1763).
        nums.push(Object::Ref(doc.add(Object::Dict(spec))));
    }
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(nums));
    let tree = doc.add(Object::Dict(tree));
    doc.update_dict(root, |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree)))?;
    Ok(())
}

/// After `deleted` pages (0-based, as they were numbered before the delete) are gone, move the label
/// ranges with the pages they label: a range starts as many pages earlier as were deleted before
/// it, and keeps its first number, so numbering continues over the gap as in Acrobat. A range whose
/// first page went starts on the next page left; one left with no pages gives way to the next.
/// A tree that can't be read is left as it is: the delete still goes ahead.
pub(crate) fn drop_deleted_pages(doc: &mut Document, deleted: &[usize]) -> Result<(), OrganizeError> {
    let Ok(old) = checked_ranges(doc) else { return Ok(()) };
    let mut deleted = deleted.to_vec();
    deleted.sort_unstable();
    deleted.dedup();
    let mut ranges: Vec<LabelRange> = Vec::with_capacity(old.len());
    for r in &old {
        let start = r.start.saturating_sub(deleted.partition_point(|d| *d < r.start));
        match ranges.last_mut() {
            // The later range is the one on the first page left; ranges stay sorted by start.
            Some(last) if last.start == start => *last = LabelRange { start, ..r.clone() },
            _ => ranges.push(LabelRange { start, ..r.clone() }),
        }
    }
    if ranges == old {
        return Ok(());
    }
    set_page_label_ranges(doc, &ranges)
}

/// Acrobat's "Number pages": label pages `from..=to` (0-based) with `style`, `prefix` and
/// starting number `first`; pages after `to` keep the labels they had.
pub fn number_pages(doc: &mut Document, from: usize, to: usize, style: LabelStyle, prefix: &str, first: u32) -> Result<(), OrganizeError> {
    if prefix.len() > MAX_LABEL_BYTES {
        return Err(invalid_label("a prefix exceeds 1024 UTF-8 bytes"));
    }
    let n = page_count(doc)?;
    if to >= n {
        return Err(OrganizeError::NoSuchPage(to));
    }
    if from > to {
        return Err(OrganizeError::NoSuchPage(from));
    }
    let mut old = checked_ranges(doc)?;
    if old.first().is_none_or(|r| r.start > 0) {
        old.insert(0, default_range());
    }
    // `old` starts at page 0, so a range is always found.
    let at = |p: usize| old.iter().rev().find(|r| r.start <= p).cloned().unwrap_or_else(default_range);
    let mut ranges: Vec<LabelRange> = old.iter().filter(|r| r.start < from).cloned().collect();
    ranges.push(LabelRange { start: from, style, prefix: prefix.to_string(), first: first.max(1) });
    if to + 1 < n {
        let r = at(to + 1);
        let first = r.number(to + 1)?;
        ranges.push(LabelRange { start: to + 1, first, ..r });
        ranges.extend(old.iter().filter(|r| r.start > to + 1).cloned());
    }
    // Drop ranges that only continue the previous one.
    let mut merged: Vec<LabelRange> = Vec::new();
    for r in ranges {
        let continues = merged.last().is_some_and(|p| {
            p.style == r.style
                && p.prefix == r.prefix
                && r.start.checked_sub(p.start).and_then(|n| u32::try_from(n).ok()).and_then(|n| p.first.checked_add(n)) == Some(r.first)
        });
        if !continues {
            merged.push(r);
        }
    }
    // Plain 1, 2, 3 everywhere is what a document without labels shows: remove them.
    if merged.len() == 1 && merged[0] == default_range() {
        merged.clear();
    }
    set_page_label_ranges(doc, &merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No limit on the whole sequence: 4,097 labels of 1,024 bytes (over 4 MiB in total) are all
    /// readable, one at a time or by page.
    #[test]
    fn label_view_has_no_aggregate_limit() {
        let range = LabelRange { start: 0, style: LabelStyle::None, prefix: "x".repeat(MAX_LABEL_BYTES), first: u32::MAX };
        let view = PageLabels { ranges: vec![range], pages: 4_097 };
        assert_eq!(view.iter().map(|label| label.len()).sum::<usize>(), 4_097 * MAX_LABEL_BYTES);
        assert_eq!(view.label(4_096).unwrap(), "x".repeat(MAX_LABEL_BYTES));
    }

    /// The view keeps the ranges, not one label per page: a billion-page document costs no more to
    /// view than a short one, and a late label is built without the labels before it.
    #[test]
    fn label_view_does_not_grow_with_the_page_count() {
        let prefix = "x".repeat(1000);
        let range = LabelRange { start: 0, style: LabelStyle::Decimal, prefix: prefix.clone(), first: 1 };
        let view = PageLabels { ranges: vec![range], pages: 1_000_000_000 };
        assert_eq!(view.label(999_999_999).unwrap(), format!("{prefix}1000000000"));
        assert_eq!(view.iter().take(2).collect::<Vec<_>>(), [format!("{prefix}1"), format!("{prefix}2")]);
        assert_eq!(view.label(1_000_000_000), Err(OrganizeError::NoSuchPage(1_000_000_000)));
    }

    /// The numeral length formula and the bounded Roman search agree with the formatter.
    #[test]
    fn numeral_bounds_match_the_formatters() {
        for n in (1..=3_000u64).chain([9_999, 10_000, 999_999, 1_000_000, 1_012_888, 1_013_888, u64::from(u32::MAX)]) {
            match roman(n, MAX_LABEL_BYTES) {
                Some(numeral) => assert_eq!(roman_len(n), numeral.len() as u64, "{n}"),
                None => assert!(roman_len(n) > MAX_LABEL_BYTES as u64, "{n}"),
            }
        }
        assert_eq!(longest_numeral(LabelStyle::UpperRoman, 888, 888), 12);
        assert_eq!(longest_numeral(LabelStyle::Decimal, 1, 1_000), 4);
        assert_eq!(longest_numeral(LabelStyle::LowerAlpha, 1, 27), 2);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..400 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let lo = 1 + (seed >> 33) % 3_000_000;
            let hi = lo + (seed >> 40) % 2_500;
            assert_eq!(longest_roman(lo, hi), (lo..=hi).map(roman_len).max().unwrap_or_default(), "{lo}..={hi}");
        }
    }

    /// The edit check accepts exactly the ranges whose every label the per-page check accepts,
    /// including spans that cross a thousand, numbers past 32 bits and long prefixes.
    #[test]
    fn span_check_agrees_with_per_label_checks() {
        let styles = [LabelStyle::None, LabelStyle::Decimal, LabelStyle::UpperRoman, LabelStyle::LowerAlpha];
        let firsts = [1, 3, 990, 999, 1_012_880, 1_013_990, 1_014_000, u32::MAX - 30, u32::MAX];
        let prefixes = [0, 500, 1000, 1010, 1020, 1024];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |bound: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            usize::try_from((seed >> 33) % bound as u64).unwrap_or_default()
        };
        for _ in 0..3_000 {
            let pages = next(60);
            let count = 1 + next(3);
            let mut starts: Vec<usize> = (0..count).map(|_| next(pages + 3)).collect();
            starts.sort_unstable();
            starts.dedup();
            let mut ranges = Vec::new();
            for start in starts {
                let style = styles[next(styles.len())];
                let prefix = "p".repeat(prefixes[next(prefixes.len())]);
                let first = firsts[next(firsts.len())];
                ranges.push(LabelRange { start, style, prefix, first });
            }
            let per_label = (0..pages).all(|p| ranges.iter().rev().find(|r| r.start <= p).is_none_or(|r| r.label(p).is_ok()));
            assert_eq!(check_spans(&ranges, pages).is_ok(), per_label, "{ranges:?} over {pages} pages");
        }
    }

    #[test]
    fn styles_format_like_the_spec() {
        assert_eq!(LabelStyle::LowerRoman.format(14), "xiv");
        assert_eq!(LabelStyle::UpperRoman.format(1994), "MCMXCIV");
        assert_eq!(LabelStyle::UpperAlpha.format(1), "A");
        assert_eq!(LabelStyle::UpperAlpha.format(27), "AA");
        assert_eq!(LabelStyle::LowerAlpha.format(53), "aaa");
        assert_eq!(LabelStyle::None.format(5), "");
    }
}
