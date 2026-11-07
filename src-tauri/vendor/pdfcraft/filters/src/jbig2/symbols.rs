//! JBIG2 symbol coding, lossless: each distinct mark of an image (a group of black pixels that
//! touch, side by side or at a corner) is stored once, as a symbol in a symbol dictionary, and
//! text regions place a copy of it wherever it occurs. Marks too large to be symbols (rules,
//! frames, pictures) are left to a generic region beside them, and so, in [`encode`]'s second
//! plan, are marks that occur only once. Only exact copies share a symbol, so the image decodes to the
//! same bits. Written from the committee draft's 6.4 (the text region decoding procedure), 6.5
//! (the symbol dictionary decoding procedure), 7.4.2 and 7.4.3 (their segments) and Annex A (the
//! arithmetic integer coders).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use super::{at_pixels, code_generic, generic_region, page_information, region_information, segment, segment_of};
use crate::mq::{Context, MqEncoder};

/// The longest side of a mark stored as a symbol. A larger one is left to the generic region.
const MAX_SYMBOL_SIDE: u32 = 256;

/// The most black runs looked at. Past it (halftones, noise) the image is left to generic coding,
/// which suits it better anyway; this bounds the memory the marks take.
const MAX_RUNS: usize = 1 << 20;

/// The most distinct marks sorted into symbols, and the most pixels their bitmaps may hold
/// together: past either the image is left to generic coding. These bound the memory they take.
const MAX_SYMBOLS: usize = 1 << 16;
const MAX_SYMBOL_PIXELS: usize = 1 << 24;

/// The most symbols one plan codes. The decoder PdfCraft renders JBIG2 with (`hayro-jbig2`)
/// refuses a dictionary whose new and exported symbols pass 65,535 together, and a plan exports
/// every symbol it codes, so it refuses 32,768 or more; others may too, so a plan with more isn't
/// made.
const MAX_CODED_SYMBOLS: usize = (1 << 15) - 1;

/// The most symbol instances one text region places. The decoder PdfCraft renders JBIG2 with
/// refuses a text region of more than 10,000 (a page it then draws blank), so a page with more is
/// placed by several regions, each over a band of rows, all referring to the one dictionary.
const MAX_REGION_INSTANCES: usize = 10_000;

/// The most symbols a group's shared dictionary holds ([`encode_shared`]): half of what a page may
/// place, so each page keeps room for its own.
const MAX_SHARED_SYMBOLS: usize = MAX_CODED_SYMBOLS / 2;

// Segment numbers stay at most 256, so a referred-to one takes a byte (7.2.5): the shared
// dictionary, the page, its own dictionary, a text region for each MAX_REGION_INSTANCES of at most
// MAX_RUNS marks, and the generic region.
const _: () = assert!(MAX_RUNS.div_ceil(MAX_REGION_INSTANCES) + 4 <= 256);

/// The integer coders' ranges (Table A.1): how many 1 bits pick the range, how many bits give the
/// offset into it, and its first value. The last range has no 0 after its prefix.
const RANGES: [(u32, u32, u32); 6] = [(0, 2, 0), (1, 4, 4), (2, 6, 20), (3, 8, 84), (4, 12, 340), (5, 32, 4436)];

/// A run of black pixels on row `y`, from `x0` up to but not including `x1`.
#[derive(Clone, Copy, Debug)]
struct Run {
    y: u32,
    x0: u32,
    x1: u32,
}

/// A mark: its box (`x1` and `y1` just past it) and its runs, as a range of the run order
/// [`marks`] gives.
#[derive(Debug)]
struct Mark {
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    runs: Range<usize>,
}

/// A distinct mark's bitmap, a byte a pixel (1 black), and how many marks are exact copies of it.
#[derive(Debug)]
struct Symbol {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    uses: u32,
}

/// An image's marks, sorted out: its black runs, its marks with the run order they index, the
/// distinct marks of at most [`MAX_SYMBOL_SIDE`] a side as symbols, each such mark as (its
/// symbol, the mark), and the marks too large to be symbols.
struct Page {
    runs: Vec<Run>,
    marks: Vec<Mark>,
    order: Vec<u32>,
    symbols: Vec<Symbol>,
    copies: Vec<(u32, u32)>,
    large: Vec<u32>,
}

/// A symbol instance (6.4.5): the symbol, and where its bottom-left pixel goes on the page, as S
/// (the column) and T (the row). `width` and `height` are the symbol's.
#[derive(Clone, Copy, Debug)]
struct Instance {
    id: u32,
    s: u32,
    t: u32,
    width: u32,
    height: u32,
}

/// A group of images coded by [`encode_shared`]: the globals' data, and each image's stream.
type SharedCoding = (Vec<u8>, Vec<Option<Vec<u8>>>);

/// The symbols of a group's shared dictionary, as one image's [`plan`] places them: the ID each of
/// the image's symbols has there, if it is there, and how many symbols that dictionary exports.
struct Shared {
    ids: Vec<Option<u32>>,
    count: u32,
}

/// `rows` rows of `columns` 1-bit PDF samples (black 0), symbol coded: the smaller of two plans,
/// every mark of at most [`MAX_SYMBOL_SIDE`] a side as a symbol, or only those with a copy.
/// `None` when neither plan has from two to [`MAX_CODED_SYMBOLS`] symbols, or the image is too
/// large or too busy to look at.
pub(super) fn encode(samples: &[u8], columns: u32, rows: u32) -> Option<Vec<u8>> {
    let page = page(samples, columns)?;
    [1, 2].into_iter().filter_map(|copies| plan(&page, columns, rows, copies, None, None)).min_by_key(Vec::len)
}

/// Images, each as [`encode`] takes one, symbol coded sharing one dictionary: the globals' data, a
/// symbol dictionary segment associated with no page (number 0) holding the distinct marks found
/// in more than one image, and each image's stream placing them from there, the smaller of
/// [`encode`]'s two plans for its other marks. The group's marks are held within the limits one
/// image's are ([`MAX_SYMBOLS`], [`MAX_SYMBOL_PIXELS`]): past them, later marks aren't shared. Past
/// [`MAX_SHARED_SYMBOLS`] shared marks, those in the most images are kept.
///
/// An image too large or too busy to look at, or that places none of the shared marks, gets `None`;
/// `None` overall when fewer than two marks are shared.
pub(super) fn encode_shared(images: &[(&[u8], u32, u32)]) -> Option<SharedCoding> {
    // The group's distinct marks, each with how many images hold it (as its uses), looked at an
    // image at a time.
    let mut table: Vec<Symbol> = Vec::new();
    let mut by_hash: HashMap<u64, Vec<u32>> = HashMap::new();
    let mut held = 0usize;
    for (samples, columns, _) in images {
        let Some(page) = page(samples, *columns) else { continue };
        // An image's symbols are distinct, so each counts an image once.
        for symbol in page.symbols {
            match find(&table, &by_hash, &symbol) {
                Some(t) => {
                    if let Some(s) = table.get_mut(t as usize) {
                        s.uses = s.uses.saturating_add(1);
                    }
                }
                None => {
                    let Some(more) = held.checked_add(symbol.pixels.len()).filter(|h| *h <= MAX_SYMBOL_PIXELS && table.len() < MAX_SYMBOLS) else {
                        continue;
                    };
                    held = more;
                    let t = u32::try_from(table.len()).ok()?;
                    by_hash.entry(fingerprint(&symbol)).or_default().push(t);
                    table.push(Symbol { uses: 1, ..symbol });
                }
            }
        }
    }
    let uses = |t: &u32| table.get(*t as usize).map_or(0, |s| s.uses);
    let mut shared: Vec<u32> = (0..u32::try_from(table.len()).ok()?).filter(|t| uses(t) >= 2).collect();
    if shared.len() < 2 {
        return None;
    }
    shared.sort_by_key(|t| std::cmp::Reverse(uses(t)));
    shared.truncate(MAX_SHARED_SYMBOLS);
    // The dictionary's order, as `plan` keeps it.
    shared.sort_by_key(|t| table.get(*t as usize).map(|s| (s.height, s.width)));
    let mut global = vec![None; table.len()];
    for (k, t) in shared.iter().enumerate() {
        if let Some(slot) = global.get_mut(*t as usize) {
            *slot = u32::try_from(k).ok();
        }
    }
    let symbols: Vec<&Symbol> = shared.iter().filter_map(|t| table.get(*t as usize)).collect();
    let mut globals = Vec::new();
    segment_of(&mut globals, 0, 0, 0, &[], true, &symbol_dictionary(&symbols)?);
    let count = u32::try_from(symbols.len()).ok()?;
    let streams = images
        .iter()
        .map(|(samples, columns, rows)| {
            let page = page(samples, *columns)?;
            let ids = page.symbols.iter().map(|s| find(&table, &by_hash, s).and_then(|t| global.get(t as usize).copied().flatten())).collect();
            let shared = Shared { ids, count };
            [1, 2].into_iter().filter_map(|copies| plan(&page, *columns, *rows, copies, None, Some(&shared))).min_by_key(Vec::len)
        })
        .collect();
    Some((globals, streams))
}

/// What sorts a symbol's bitmap ([`symbols`] and [`encode_shared`] look exact copies up by it).
fn fingerprint(s: &Symbol) -> u64 {
    let mut hasher = DefaultHasher::new();
    (s.width, s.height, &s.pixels).hash(&mut hasher);
    hasher.finish()
}

/// The index in `table` of the symbol with `s`'s bitmap, looked up by its [`fingerprint`].
fn find(table: &[Symbol], by_hash: &HashMap<u64, Vec<u32>>, s: &Symbol) -> Option<u32> {
    by_hash
        .get(&fingerprint(s))?
        .iter()
        .copied()
        .find(|t| table.get(*t as usize).is_some_and(|t| (t.width, t.height) == (s.width, s.height) && t.pixels == s.pixels))
}

/// The image's marks and symbols; `None` past the limits above.
fn page(samples: &[u8], columns: u32) -> Option<Page> {
    let width = usize::try_from(columns).ok()?;
    let stride = width.div_ceil(8);
    if stride == 0 {
        return None;
    }
    let runs = black_runs(samples, width, stride)?;
    let (marks, order) = marks(&runs);
    let mut page = Page { runs, marks, order, symbols: Vec::new(), copies: Vec::new(), large: Vec::new() };
    symbols(&mut page)?;
    Some(page)
}

/// The black runs of each row (0 samples), row by row and left to right; `None` past [`MAX_RUNS`].
fn black_runs(samples: &[u8], width: usize, stride: usize) -> Option<Vec<Run>> {
    let black = |row: &[u8], x: usize| row.get(x / 8).is_some_and(|b| b & (0x80 >> (x % 8)) == 0);
    let mut runs = Vec::new();
    for (y, row) in samples.chunks_exact(stride).enumerate() {
        let y = u32::try_from(y).ok()?;
        let mut x = 0;
        while x < width {
            // Whole white bytes at once.
            if x % 8 == 0 && row.get(x / 8) == Some(&0xFF) {
                x += 8;
                continue;
            }
            if !black(row, x) {
                x += 1;
                continue;
            }
            let start = x;
            while x < width && black(row, x) {
                x += 1;
            }
            if runs.len() == MAX_RUNS {
                return None;
            }
            runs.push(Run { y, x0: u32::try_from(start).ok()?, x1: u32::try_from(x).ok()? });
        }
    }
    Some(runs)
}

/// The marks the runs make, in the order their first runs come (so from the top), and the run
/// order: the runs' indexes, mark by mark, each mark's in row order.
fn marks(runs: &[Run]) -> (Vec<Mark>, Vec<u32>) {
    // Union–find over the runs; a set's root is its first run, as the larger root always joins
    // the smaller. Runs on adjacent rows join when they overlap, counting a corner.
    let mut parent: Vec<u32> = (0..runs.len()).map(|i| i as u32).collect();
    let (mut above, mut above_at, mut above_y): (&[Run], usize, Option<u32>) = (&[], 0, None);
    let mut at = 0;
    for row in runs.chunk_by(|a, b| a.y == b.y) {
        let y = row.first().map(|r| r.y);
        if above_y.and_then(|a| a.checked_add(1)) == y {
            let mut i = 0;
            for (j, b) in row.iter().enumerate() {
                while above.get(i).is_some_and(|a| a.x1 < b.x0) {
                    i += 1;
                }
                let mut k = i;
                while above.get(k).is_some_and(|a| a.x0 <= b.x1) {
                    union(&mut parent, above_at + k, at + j);
                    k += 1;
                }
            }
        }
        (above, above_at, above_y) = (row, at, y);
        at += row.len();
    }
    let mut mark_of = vec![0u32; runs.len()];
    let mut marks: Vec<Mark> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        let root = find_root(&mut parent, i);
        let m = if root == i {
            marks.push(Mark { x0: r.x0, x1: r.x1, y0: r.y, y1: r.y.saturating_add(1), runs: 0..0 });
            counts.push(0);
            marks.len() - 1
        } else {
            mark_of.get(root).map_or(0, |m| *m as usize)
        };
        if let Some(slot) = mark_of.get_mut(i) {
            *slot = m as u32;
        }
        if let (Some(mark), Some(count)) = (marks.get_mut(m), counts.get_mut(m)) {
            mark.x0 = mark.x0.min(r.x0);
            mark.x1 = mark.x1.max(r.x1);
            mark.y1 = mark.y1.max(r.y.saturating_add(1));
            *count += 1;
        }
    }
    let mut next = 0;
    for (mark, count) in marks.iter_mut().zip(&counts) {
        mark.runs = next..next + count;
        next += count;
    }
    let mut order = vec![0u32; runs.len()];
    let mut fill: Vec<usize> = marks.iter().map(|m| m.runs.start).collect();
    for (i, m) in mark_of.iter().enumerate() {
        if let Some(n) = fill.get_mut(*m as usize) {
            if let Some(slot) = order.get_mut(*n) {
                *slot = i as u32;
            }
            *n += 1;
        }
    }
    (marks, order)
}

/// The root of `i`'s set, halving the path to it as it goes. Parents are never above their
/// children, so the walk ends.
fn find_root(parent: &mut [u32], mut i: usize) -> usize {
    loop {
        let Some(&p) = parent.get(i) else { return i };
        let p = p as usize;
        if p == i {
            return i;
        }
        let grandparent = parent.get(p).copied().unwrap_or(p as u32);
        if let Some(slot) = parent.get_mut(i) {
            *slot = grandparent;
        }
        i = grandparent as usize;
    }
}

fn union(parent: &mut [u32], a: usize, b: usize) {
    let (a, b) = (find_root(parent, a), find_root(parent, b));
    if let Some(slot) = parent.get_mut(a.max(b)) {
        *slot = a.min(b) as u32;
    }
}

/// Each mark's own pixels in its box: those of its runs, not of other marks the box overlaps.
fn pixels(mark: &Mark, runs: &[Run], order: &[u32]) -> Vec<u8> {
    // A mark's box holds its runs, so these don't go below 0; a symbol is at most 256 a side.
    let width = mark.x1.saturating_sub(mark.x0) as usize;
    let height = mark.y1.saturating_sub(mark.y0) as usize;
    let mut pixels = vec![0u8; width * height];
    for r in order.get(mark.runs.clone()).unwrap_or_default().iter().filter_map(|i| runs.get(*i as usize)) {
        let row = r.y.saturating_sub(mark.y0) as usize * width;
        let from = row + r.x0.saturating_sub(mark.x0) as usize;
        let to = row + r.x1.saturating_sub(mark.x0) as usize;
        pixels.get_mut(from..to).unwrap_or_default().fill(1);
    }
    pixels
}

/// Sorts the marks into symbols: exact copies share one. `None` past [`MAX_SYMBOLS`] or
/// [`MAX_SYMBOL_PIXELS`].
fn symbols(page: &mut Page) -> Option<()> {
    let mut by_hash: HashMap<u64, Vec<u32>> = HashMap::new();
    let mut kept = 0usize;
    for (m, mark) in page.marks.iter().enumerate() {
        let m = u32::try_from(m).ok()?;
        let (width, height) = (mark.x1.saturating_sub(mark.x0), mark.y1.saturating_sub(mark.y0));
        if width > MAX_SYMBOL_SIDE || height > MAX_SYMBOL_SIDE {
            page.large.push(m);
            continue;
        }
        let symbol = Symbol { width, height, pixels: pixels(mark, &page.runs, &page.order), uses: 0 };
        let symbols = &mut page.symbols;
        let id = match find(symbols, &by_hash, &symbol) {
            Some(id) => id,
            None => {
                kept = kept.checked_add(symbol.pixels.len())?;
                if kept > MAX_SYMBOL_PIXELS || symbols.len() == MAX_SYMBOLS {
                    return None;
                }
                let id = u32::try_from(symbols.len()).ok()?;
                by_hash.entry(fingerprint(&symbol)).or_default().push(id);
                symbols.push(symbol);
                id
            }
        };
        if let Some(s) = symbols.get_mut(id as usize) {
            s.uses = s.uses.saturating_add(1);
        }
        page.copies.push((id, m));
    }
    Some(())
}

/// One plan's stream: the symbols with at least `copies` uses in a symbol dictionary, text
/// regions placing them (in strips of 2^`log_strips` rows, or whichever size codes smallest), and
/// the other marks in a generic region. `None` with fewer than two such symbols (their IDs would
/// take no bits at all, SBSYMCODELEN 0, a corner of the standard left alone here) or more than
/// [`MAX_CODED_SYMBOLS`].
///
/// With `shared`, the symbols a group's dictionary holds are placed from there, whatever their uses
/// here, and the page's own dictionary holds only the others (none, when it has none), as
/// [`encode_shared`] writes them; `None` too for a page that places none of the shared ones.
fn plan(page: &Page, columns: u32, rows: u32, copies: u32, log_strips: Option<u32>, shared: Option<&Shared>) -> Option<Vec<u8>> {
    let symbol = |s: u32| page.symbols.get(s as usize);
    let global = |s: u32| shared.and_then(|g| g.ids.get(s as usize).copied().flatten());
    let globals = shared.map_or(0, |g| g.count);
    // The dictionary's order (6.5.5): height classes from the shortest, each narrowest first.
    let mut kept: Vec<u32> =
        (0..page.symbols.len() as u32).filter(|s| global(*s).is_none() && symbol(*s).is_some_and(|s| s.uses >= copies)).collect();
    let count = usize::try_from(globals).ok()?.checked_add(kept.len())?;
    if !(2..=MAX_CODED_SYMBOLS).contains(&count) {
        return None;
    }
    kept.sort_by_key(|s| symbol(*s).map(|s| (s.height, s.width)));
    // The shared symbols' IDs come first, as their dictionary is referred to first (Table 31's
    // SBSYMS).
    let mut id: Vec<Option<u32>> = (0..page.symbols.len() as u32).map(global).collect();
    for (k, s) in kept.iter().enumerate() {
        if let Some(slot) = id.get_mut(*s as usize) {
            *slot = u32::try_from(k).ok().and_then(|k| k.checked_add(globals));
        }
    }
    let mut instances = Vec::new();
    let mut left = page.large.clone();
    for (s, m) in &page.copies {
        match (id.get(*s as usize).copied().flatten(), page.marks.get(*m as usize), symbol(*s)) {
            (Some(id), Some(mark), Some(symbol)) => {
                instances.push(Instance { id, s: mark.x0, t: mark.y1.saturating_sub(1), width: symbol.width, height: symbol.height })
            }
            _ => left.push(*m),
        }
    }
    if shared.is_some() && !instances.iter().any(|i| i.id < globals) {
        return None;
    }
    let kept: Vec<&Symbol> = kept.iter().filter_map(|s| symbol(*s)).collect();
    let dictionary = if kept.is_empty() { None } else { Some(symbol_dictionary(&kept)?) };
    let count = u32::try_from(count).ok()?;
    // Bands of at most MAX_REGION_INSTANCES, from the top.
    instances.sort_by_key(|i| (i.t, i.s));
    let mut regions = Vec::new();
    for band in instances.chunks_mut(MAX_REGION_INSTANCES) {
        let top = band.iter().map(|i| i.t.saturating_add(1).saturating_sub(i.height)).min()?;
        let height = band.iter().map(|i| i.t).max()?.checked_sub(top)?.checked_add(1)?;
        regions.push(match log_strips {
            Some(l) => text_region(band, count, columns, top, height, l)?,
            None => (0..4).filter_map(|l| text_region(band, count, columns, top, height, l)).min_by_key(Vec::len)?,
        });
    }
    let mut out = Vec::new();
    // The shared dictionary is segment 0, in the globals, and a segment refers only to lower
    // numbers (7.2.5): the page's own follow it.
    let mut number = u32::from(shared.is_some());
    segment(&mut out, number, 48, &[], false, &page_information(columns, rows));
    number += 1;
    let mut dictionaries: Vec<u8> = shared.map(|_| 0).into_iter().collect();
    if let Some(dictionary) = &dictionary {
        segment(&mut out, number, 0, &[], true, dictionary);
        dictionaries.push(u8::try_from(number).ok()?);
        number += 1;
    }
    for (k, text) in regions.iter().enumerate() {
        // The dictionaries stay retained for the next region, until the last (7.2.4).
        let refers_to: Vec<(u8, bool)> = dictionaries.iter().map(|n| (*n, k + 1 < regions.len())).collect();
        segment(&mut out, number, 7, &refers_to, false, text);
        number += 1;
    }
    if !left.is_empty() {
        segment(&mut out, number, 39, &[], false, &leftover_region(&left, page)?);
    }
    Some(out)
}

/// A symbol dictionary segment's data (7.4.2) defining `symbols` (in dictionary order) and
/// exporting them all: arithmetic coded, no refinement or aggregation, each bitmap a generic
/// region with template 0, its nominal AT pixels and no typical prediction (Table 16), the
/// bitmaps sharing one set of contexts.
fn symbol_dictionary(symbols: &[&Symbol]) -> Option<Vec<u8>> {
    let count = u32::try_from(symbols.len()).ok()?;
    let mut data = vec![0, 0];
    at_pixels(&mut data);
    data.extend_from_slice(&count.to_be_bytes());
    data.extend_from_slice(&count.to_be_bytes());
    let mut mq = MqEncoder::new();
    let mut contexts: Vec<Context> = vec![0; 1 << 16];
    let (mut iadh, mut iadw, mut iaex) = (IntCoder::new(), IntCoder::new(), IntCoder::new());
    let mut height = 0;
    for class in symbols.chunk_by(|a, b| a.height == b.height) {
        let h = class.first()?.height;
        iadh.encode(&mut mq, Some(int(i64::from(h) - i64::from(height))?));
        height = h;
        let mut width = 0;
        for s in class {
            iadw.encode(&mut mq, Some(int(i64::from(s.width) - i64::from(width))?));
            width = s.width;
            let w = s.width as usize;
            code_generic(&mut mq, &mut contexts, w, s.height as usize, false, |y, row| {
                let line = s.pixels.get(y * w..).unwrap_or_default();
                row.iter_mut().zip(line).for_each(|(p, v)| *p = *v);
            });
        }
        iadw.encode(&mut mq, None);
    }
    // The export flags (6.5.10): a run of no symbols not exported, then a run of all of them.
    iaex.encode(&mut mq, Some(0));
    iaex.encode(&mut mq, Some(int(i64::from(count))?));
    data.extend_from_slice(&mq.finish());
    Some(data)
}

/// An immediate lossless text region segment's data (7.4.3) placing `instances` in the band of
/// `height` rows from row `top` (which holds them): arithmetic coded, no refinement, strips of
/// 2^`log_strips` rows, each symbol's bottom-left pixel at (S, T) (REFCORNER BOTTOMLEFT, so a
/// line's letters share their T), not transposed, OR, default pixel 0, SBDSOFFSET 0. Sorts
/// `instances` into strips, each left to right.
fn text_region(instances: &mut [Instance], symbols: u32, columns: u32, top: u32, height: u32, log_strips: u32) -> Option<Vec<u8>> {
    // LOGSBSTRIPS has two bits: strips of 1, 2, 4 or 8 rows.
    if log_strips > 3 {
        return None;
    }
    let strips = 1i64 << log_strips;
    // T counts from the band's top row.
    let t = |i: &Instance| i.t.saturating_sub(top);
    instances.sort_by_key(|i| (t(i) >> log_strips, i.s));
    let mut data = Vec::new();
    region_information(&mut data, columns, height, 0, top);
    data.extend_from_slice(&((log_strips as u16) << 2).to_be_bytes());
    data.extend_from_slice(&u32::try_from(instances.len()).ok()?.to_be_bytes());
    // SBSYMCODELEN (Table 31): ⌈log2 SBNUMSYMS⌉.
    let length = symbols.checked_next_power_of_two()?.trailing_zeros();
    let mut ids: Vec<Context> = vec![0; 1 << length];
    let (mut iadt, mut iafs, mut iads, mut iait) = (IntCoder::new(), IntCoder::new(), IntCoder::new(), IntCoder::new());
    let mut mq = MqEncoder::new();
    // The initial STRIPT (6.4.5 step 2) is 0, so each strip's delta T is its first row over the
    // strip size, from the strip before.
    iadt.encode(&mut mq, Some(0));
    let (mut strip_t, mut first_s) = (0i64, 0i64);
    for strip in instances.chunk_by(|a, b| t(a) >> log_strips == t(b) >> log_strips) {
        let first = i64::from(t(strip.first()?) >> log_strips << log_strips);
        iadt.encode(&mut mq, Some(int((first - strip_t) / strips)?));
        strip_t = first;
        let mut cur_s = 0;
        for (k, instance) in strip.iter().enumerate() {
            let s = i64::from(instance.s);
            if k == 0 {
                iafs.encode(&mut mq, Some(int(s - first_s)?));
                first_s = s;
            } else {
                iads.encode(&mut mq, Some(int(s - cur_s)?));
            }
            if strips > 1 {
                iait.encode(&mut mq, Some(int(i64::from(t(instance)) - strip_t)?));
            }
            encode_id(&mut mq, &mut ids, length, instance.id);
            // The next S counts from this symbol's last column (6.4.5 step 3c x).
            cur_s = s + i64::from(instance.width) - 1;
        }
        iads.encode(&mut mq, None);
    }
    data.extend_from_slice(&mq.finish());
    Some(data)
}

/// A generic region segment's data holding the marks `left`, over their box.
fn leftover_region(left: &[u32], page: &Page) -> Option<Vec<u8>> {
    let left: Vec<&Mark> = left.iter().filter_map(|m| page.marks.get(*m as usize)).collect();
    let x0 = left.iter().map(|m| m.x0).min()?;
    let y0 = left.iter().map(|m| m.y0).min()?;
    let x1 = left.iter().map(|m| m.x1).max()?;
    let y1 = left.iter().map(|m| m.y1).max()?;
    let (width, height) = (x1.saturating_sub(x0) as usize, y1.saturating_sub(y0) as usize);
    let stride = width.div_ceil(8);
    // Packed, 1 for black: no larger than the image's own samples.
    let mut packed = vec![0u8; stride.checked_mul(height)?];
    for mark in &left {
        for r in page.order.get(mark.runs.clone()).unwrap_or_default().iter().filter_map(|i| page.runs.get(*i as usize)) {
            let row = r.y.saturating_sub(y0) as usize * stride;
            for x in r.x0.saturating_sub(x0) as usize..r.x1.saturating_sub(x0) as usize {
                if let Some(b) = packed.get_mut(row + x / 8) {
                    *b |= 0x80 >> (x % 8);
                }
            }
        }
    }
    let mut mq = MqEncoder::new();
    let mut contexts: Vec<Context> = vec![0; 1 << 16];
    code_generic(&mut mq, &mut contexts, width, height, true, |y, row| {
        let line = packed.get(y * stride..).unwrap_or_default();
        for (x, p) in row.iter_mut().enumerate() {
            *p = u8::from(line.get(x / 8).is_some_and(|b| b & (0x80 >> (x % 8)) != 0));
        }
    });
    Some(generic_region(u32::try_from(width).ok()?, u32::try_from(height).ok()?, x0, y0, &mq.finish()))
}

fn int(v: i64) -> Option<i32> {
    i32::try_from(v).ok()
}

/// An arithmetic integer coder other than IAID (A.2): its 512 contexts, indexed by PREV.
struct IntCoder([Context; 512]);

impl IntCoder {
    fn new() -> Self {
        IntCoder([0; 512])
    }

    /// Codes `value`, or OOB for `None`.
    fn encode(&mut self, mq: &mut MqEncoder, value: Option<i32>) {
        for (cx, d) in decisions(value) {
            if let Some(state) = self.0.get_mut(cx) {
                mq.encode(state, d);
            }
        }
    }
}

/// The decisions that code `value`, or OOB for `None`, with their contexts (A.2, Table A.1): the
/// sign, the prefix that picks the range, then the offset into it, most significant bit first;
/// OOB is a negative 0. Each decision's context is PREV: a leading 1 and the decisions before it,
/// at most the last eight once there are more.
fn decisions(value: Option<i32>) -> Vec<(usize, bool)> {
    let (negative, magnitude) = value.map_or((true, 0), |v| (v < 0, v.unsigned_abs()));
    let (ones, bits, first) =
        RANGES.into_iter().find(|(_, bits, first)| *bits == 32 || u64::from(magnitude) < u64::from(*first) + (1 << bits)).unwrap_or(RANGES[5]);
    let offset = magnitude - first;
    let mut d = vec![negative];
    d.extend(std::iter::repeat_n(true, ones as usize));
    if ones < 5 {
        d.push(false);
    }
    d.extend((0..bits).rev().map(|i| offset >> i & 1 == 1));
    let mut prev = 1usize;
    d.into_iter()
        .map(|bit| {
            let cx = prev;
            prev = if prev < 256 { prev << 1 | usize::from(bit) } else { ((prev << 1 | usize::from(bit)) & 511) | 256 };
            (cx, bit)
        })
        .collect()
}

/// Codes symbol ID `id` in `length` bits, most significant first, through IAID (A.3): each
/// decision's context is a leading 1 and all the bits before it.
fn encode_id(mq: &mut MqEncoder, contexts: &mut [Context], length: u32, id: u32) {
    let mut prev = 1usize;
    for i in (0..length).rev() {
        let d = id >> i & 1 == 1;
        if let Some(state) = contexts.get_mut(prev) {
            mq.encode(state, d);
        }
        prev = prev << 1 | usize::from(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FilterError;
    use crate::jbig2::tests::{decode, decode_with, samples, unpack};
    use crate::jbig2::{encode_jbig2, encode_jbig2_generic, encode_jbig2_shared};

    type Bitmap = Vec<Vec<u8>>;

    /// A small linear congruential generator, so the pictures are the same every run.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as usize % n.max(1)
        }
    }

    /// `count` different letter-like shapes: three strokes two pixels thick in a box of 6 to 18
    /// pixels a side, each after the first starting on one before it, so a shape is one mark;
    /// trimmed to its pixels.
    fn glyphs(count: usize, seed: u64) -> Vec<Bitmap> {
        let mut r = Lcg(seed);
        let mut out: Vec<Bitmap> = Vec::new();
        while out.len() < count {
            let (w, h) = (6 + r.below(13), 6 + r.below(13));
            let mut g = vec![vec![0u8; w]; h];
            let (mut x, mut y) = (r.below(w), r.below(h));
            for _ in 0..3 {
                let (dx, dy) = [(1, 0), (0, 1), (1, 1)][r.below(3)];
                let mut stroke = Vec::new();
                while x < w && y < h {
                    g[y][x] = 1;
                    g[y][(x + 1).min(w - 1)] = 1;
                    stroke.push((x, y));
                    x += dx;
                    y += dy;
                }
                (x, y) = stroke[r.below(stroke.len())];
            }
            let rows: Vec<usize> = (0..h).filter(|y| g[*y].contains(&1)).collect();
            let columns: Vec<usize> = (0..w).filter(|x| g.iter().any(|row| row[*x] == 1)).collect();
            let g: Bitmap = g[rows[0]..=rows[rows.len() - 1]].iter().map(|row| row[columns[0]..=columns[columns.len() - 1]].to_vec()).collect();
            if !out.contains(&g) {
                out.push(g);
            }
        }
        out
    }

    /// Draws `glyph` with its top-left pixel at (`x`, `y`), clipped to the page, by OR.
    fn draw(page: &mut Bitmap, glyph: &Bitmap, x: i64, y: i64) {
        for (gy, row) in glyph.iter().enumerate() {
            for (gx, p) in row.iter().enumerate() {
                let (px, py) = (x + gx as i64, y + gy as i64);
                if *p == 1
                    && px >= 0
                    && py >= 0
                    && let Some(cell) = page.get_mut(py as usize).and_then(|r| r.get_mut(px as usize))
                {
                    *cell = 1;
                }
            }
        }
    }

    /// Lines of text: glyphs side by side, one to four pixels apart, each line's glyphs on its
    /// baseline, the lines 30 rows apart; the first glyphs are the most common, as letters are.
    fn text(columns: usize, rows: usize, glyphs: &[Bitmap], seed: u64) -> Bitmap {
        let mut page = vec![vec![0u8; columns]; rows];
        let mut r = Lcg(seed);
        for baseline in (24..rows).step_by(30) {
            let mut x = 4 + r.below(8);
            loop {
                let common = r.below(glyphs.len()) + 1;
                let g = &glyphs[r.below(common)];
                if x + g[0].len() > columns {
                    break;
                }
                draw(&mut page, g, x as i64, (baseline + 1 - g.len()) as i64);
                x += g[0].len() + 1 + r.below(4);
            }
        }
        page
    }

    /// `n` glyphs anywhere, some over the edges, some over each other.
    fn scatter(columns: usize, rows: usize, glyphs: &[Bitmap], seed: u64, n: usize) -> Bitmap {
        let mut page = vec![vec![0u8; columns]; rows];
        let mut r = Lcg(seed);
        for _ in 0..n {
            let g = &glyphs[r.below(glyphs.len())];
            draw(&mut page, g, r.below(columns + 12) as i64 - 6, r.below(rows + 12) as i64 - 6);
        }
        page
    }

    /// A segment's number and type, whether it is retained, and the segments it refers to, each
    /// with its retain bit.
    type Segment = (u32, u8, bool, Vec<(u8, bool)>);

    /// The segments of a stream in the embedded organisation, from their headers (7.2).
    fn segments(data: &[u8]) -> Vec<Segment> {
        let (mut segments, mut at) = (Vec::new(), 0);
        while at < data.len() {
            let flags = data[at + 5];
            let refers = usize::from(flags >> 5);
            let length_at = at + 6 + refers + 1;
            let length = u32::from_be_bytes(data[length_at..length_at + 4].try_into().unwrap()) as usize;
            let referred = data[at + 6..at + 6 + refers].iter().enumerate().map(|(k, n)| (*n, flags >> (k + 1) & 1 == 1)).collect();
            let number = u32::from_be_bytes(data[at..at + 4].try_into().unwrap());
            segments.push((number, data[at + 4] & 0x3F, flags & 1 == 1, referred));
            at = length_at + 4 + length;
        }
        segments
    }

    fn segment_types(data: &[u8]) -> Vec<u8> {
        segments(data).into_iter().map(|(_, kind, ..)| kind).collect()
    }

    /// Every plan, at every strip size, decodes to the picture; returns how many plans there were.
    fn every_plan_round_trips(picture: &Bitmap) -> usize {
        let (columns, rows) = (picture[0].len() as u32, picture.len() as u32);
        let data = samples(picture);
        let page = page(&data, columns).unwrap();
        let mut plans = 0;
        for copies in [1, 2] {
            for log_strips in 0..4 {
                if let Some(coded) = plan(&page, columns, rows, copies, Some(log_strips), None) {
                    assert_eq!(decode(&coded), unpack(&data, columns, rows), "{copies} copies, strips of {}", 1 << log_strips);
                    plans += 1;
                }
            }
        }
        plans
    }

    /// Annex A.2's example (12 through IADW: the decisions 0101000, in contexts 1, 10, 101, …)
    /// and Table A.1's codes.
    #[test]
    fn integers_are_coded_as_annex_a_says() {
        assert_eq!(decisions(Some(12)), [(1, false), (2, true), (5, false), (10, true), (21, false), (42, false), (84, false)]);
        let bits = |v| decisions(v).iter().map(|(_, d)| u8::from(*d)).collect::<Vec<u8>>();
        assert_eq!(bits(None), [1, 0, 0, 0], "OOB");
        assert_eq!(bits(Some(0)), [0, 0, 0, 0]);
        assert_eq!(bits(Some(-1)), [1, 0, 0, 1]);
        assert_eq!(bits(Some(-3)), [1, 0, 1, 1]);
        assert_eq!(bits(Some(4)), [0, 1, 0, 0, 0, 0, 0]);
        assert_eq!(bits(Some(-19)), [1, 1, 0, 1, 1, 1, 1]);
        assert_eq!(bits(Some(83)), [0, 1, 1, 0, 1, 1, 1, 1, 1, 1]);
        assert_eq!(bits(Some(4435))[..6], [0, 1, 1, 1, 1, 0]);
        let big = bits(Some(4436));
        assert_eq!((big.len(), &big[..6]), (6 + 32, &[0, 1, 1, 1, 1, 1][..]));
        assert!(big[6..].iter().all(|b| *b == 0));
        // PREV keeps a leading 1 and the last eight decisions, so it stays below 512, also when
        // those eight are all 0 (PREV exactly 256), as in 4436's offset of 0.
        for value in [Some(i32::MIN), Some(4436), Some(-4436), Some(4436 + 0x100), Some(i32::MAX), None] {
            let contexts: Vec<usize> = decisions(value).iter().map(|(cx, _)| *cx).collect();
            assert!(contexts.iter().take(8).all(|cx| *cx < 256) && contexts.iter().skip(8).all(|cx| (256..512).contains(cx)), "{value:?}");
        }
        assert!(decisions(Some(4436)).iter().any(|(cx, _)| *cx == 256));
    }

    /// Strips and first columns thousands of pixels apart need Annex A's 32-bit range: a mark at
    /// the far right of a 12,000-pixel page, then one at the left below it (S back by 11,000), and
    /// lines 5,000 rows apart. Each shape occurs twice, so both plans are made.
    #[test]
    fn coordinates_past_4435_round_trip() {
        let shapes = glyphs(3, 4);
        let mut picture = vec![vec![0u8; 12_000]; 40];
        for (k, x) in [11_000, 0, 5_000, 11_980].into_iter().enumerate() {
            draw(&mut picture, &shapes[k % 2], x, 2 + 9 * k as i64);
        }
        assert_eq!(every_plan_round_trips(&picture), 8);
        let mut tall = vec![vec![0u8; 30]; 10_050];
        for (k, y) in [0, 5_000, 10_030].into_iter().enumerate() {
            draw(&mut tall, &shapes[k % 3], 3, y);
            draw(&mut tall, &shapes[(k + 1) % 3], 15, y);
        }
        assert_eq!(every_plan_round_trips(&tall), 8);
    }

    /// The padding bits of a row whose width isn't a multiple of 8 are ignored whatever they are:
    /// black (0) padding, as many scans have, neither joins the marks nor stops symbol coding.
    #[test]
    fn black_padding_bits_are_ignored() {
        for columns in [13, 241, 901] {
            let picture = text(columns, 120, &glyphs(12, 6), 8);
            let mut data = samples(&picture);
            let stride = columns.div_ceil(8);
            for row in data.chunks_mut(stride) {
                row[stride - 1] &= 0xFF << (8 * stride - columns);
            }
            let (c, r) = (columns as u32, 120);
            let page = page(&data, c).unwrap();
            assert!(page.marks.iter().all(|m| m.x1 as usize <= columns), "{columns} columns");
            for copies in [1, 2] {
                if let Some(coded) = plan(&page, c, r, copies, None, None) {
                    assert_eq!(decode(&coded), unpack(&data, c, r), "{columns} columns");
                }
            }
            if columns > 13 {
                assert_eq!(segment_types(&encode_jbig2(&data, c, r).unwrap())[1], 0, "{columns} columns: symbol coded");
            }
        }
    }

    /// Pixels touching only at a corner, down to the right or down to the left, are one mark
    /// (8-connected); side by side with a white column between, two. A mark of 256 pixels a side
    /// is a symbol; one 257 wide, or 257 tall, is left to the generic region. The diagonal and the
    /// dot occur twice, so both plans are made.
    #[test]
    fn marks_join_at_corners_and_large_ones_are_left_out() {
        let mut picture = vec![vec![0u8; 300]; 300];
        for i in 0..6 {
            picture[10 + i][10 + i] = 1;
            picture[10 + i][100 + i] = 1;
            picture[10 + i][60 - i] = 1;
        }
        for row in &mut picture[0..257] {
            row[280] = 1;
        }
        picture[30][10] = 1;
        picture[30][12] = 1;
        picture[40][0..256].fill(1);
        picture[60][0..257].fill(1);
        for row in &mut picture[44..300] {
            row[290] = 1;
        }
        let data = samples(&picture);
        let page = page(&data, 300).unwrap();
        let boxes: Vec<(u32, u32, u32, u32)> = page.marks.iter().map(|m| (m.x0, m.y0, m.x1 - m.x0, m.y1 - m.y0)).collect();
        assert_eq!(
            boxes,
            [
                (280, 0, 1, 257),
                (10, 10, 6, 6),
                (55, 10, 6, 6),
                (100, 10, 6, 6),
                (10, 30, 1, 1),
                (12, 30, 1, 1),
                (0, 40, 256, 1),
                (290, 44, 1, 256),
                (0, 60, 257, 1)
            ]
        );
        let large: Vec<(u32, u32)> = page.large.iter().map(|m| (page.marks[*m as usize].x0, page.marks[*m as usize].y0)).collect();
        assert_eq!(large, [(280, 0), (0, 60)]);
        assert_eq!(every_plan_round_trips(&picture), 8);
    }

    /// Past 2^20 black runs the image is left to generic coding; at the cap it is still looked at.
    #[test]
    fn busy_images_are_left_to_generic_coding() {
        // 0x55: four black pixels a byte, each its own run, under the same ones on the next row.
        let columns = 1 << 20;
        let row = vec![0x55u8; columns / 8];
        assert!(page(&row.repeat(2), columns as u32).is_some(), "2^20 runs");
        assert!(page(&row.repeat(3), columns as u32).is_none(), "1.5 × 2^20 runs");
    }

    /// A plan codes at most 32,767 symbols, which an independent decoder reads back; with more,
    /// the second plan can still code the repeated ones. Past 65,536 distinct marks, or 2^24
    /// pixels of them, the image is left to generic coding; at either cap it is still looked at.
    #[test]
    fn too_many_symbols_are_left_to_generic_coding() {
        // `n` distinct marks of 17 × 3 (full top and bottom rows joined by a code of 1 to n, in
        // binary, on the row between), then `again` more with the codes 1 and 2 in turn.
        let codes = |n: usize, again: usize| {
            let (columns, rows) = (256 * 18, (n + again).div_ceil(256) * 4);
            let mut picture = vec![vec![0u8; columns]; rows];
            for k in 0..n + again {
                let (x, y) = (k % 256 * 18, k / 256 * 4);
                let code = if k < n { k + 1 } else { (k - n) % 2 + 1 };
                picture[y][x..x + 17].fill(1);
                picture[y + 2][x..x + 17].fill(1);
                for b in (0..17).filter(|b| code >> b & 1 == 1) {
                    picture[y + 1][x + b] = 1;
                }
            }
            (samples(&picture), columns as u32, rows as u32)
        };
        let (data, columns, rows) = codes(MAX_CODED_SYMBOLS, 0);
        let at_cap = page(&data, columns).unwrap();
        assert_eq!(decode(&plan(&at_cap, columns, rows, 1, None, None).unwrap()), unpack(&data, columns, rows));
        // One more distinct mark, each once: neither plan is made.
        let (data, columns, rows) = codes(MAX_CODED_SYMBOLS + 1, 0);
        let past = page(&data, columns).unwrap();
        assert!(plan(&past, columns, rows, 1, None, None).is_none() && plan(&past, columns, rows, 2, None, None).is_none());
        assert_eq!(encode_jbig2(&data, columns, rows).unwrap(), encode_jbig2_generic(&data, columns, rows).unwrap());
        // With two of them repeated, the second plan codes those two.
        let (data, columns, rows) = codes(MAX_CODED_SYMBOLS + 1, 200);
        let past = page(&data, columns).unwrap();
        assert!(plan(&past, columns, rows, 1, None, None).is_none());
        assert_eq!(decode(&plan(&past, columns, rows, 2, None, None).unwrap()), unpack(&data, columns, rows));
        // Above a page of text whose letters repeat, that second plan is the one `encode_jbig2`
        // keeps: the first, of every mark, would code too many symbols.
        let mut picture: Bitmap = vec![vec![0u8; 256 * 18]; 512];
        for k in 0..=MAX_CODED_SYMBOLS {
            let (x, y) = (k % 256 * 18, k / 256 * 4);
            picture[y][x..x + 17].fill(1);
            picture[y + 2][x..x + 17].fill(1);
            for b in (0..17).filter(|b| (k + 1) >> b & 1 == 1) {
                picture[y + 1][x + b] = 1;
            }
        }
        picture.extend(text(256 * 18, 600, &glyphs(40, 7), 11));
        let data = samples(&picture);
        let (columns, rows) = (picture[0].len() as u32, picture.len() as u32);
        let coded = encode_jbig2(&data, columns, rows).unwrap();
        assert_eq!(segment_types(&coded)[1], 0, "symbol coded");
        assert_eq!(decode(&coded), unpack(&data, columns, rows));
        let (data, columns, _) = codes(MAX_SYMBOLS, 0);
        assert_eq!(page(&data, columns).map(|p| p.symbols.len()), Some(MAX_SYMBOLS));
        let (data, columns, _) = codes(MAX_SYMBOLS + 1, 0);
        assert!(page(&data, columns).is_none());
        // `n` black squares of 256 × 256, each with a white pixel in a different place: 2^16
        // pixels a symbol, so 256 of them are exactly the cap.
        let squares = |n: usize| {
            let (columns, rows) = (16 * 258, n.div_ceil(16) * 258);
            let mut picture = vec![vec![0u8; columns]; rows];
            for k in 0..n {
                let (x, y) = (k % 16 * 258, k / 16 * 258);
                for row in &mut picture[y..y + 256] {
                    row[x..x + 256].fill(1);
                }
                picture[y + 1 + k / 254][x + 1 + k % 254] = 0;
            }
            (samples(&picture), columns as u32, rows as u32)
        };
        let (data, columns, rows) = squares(256);
        let at_cap = page(&data, columns).unwrap();
        assert_eq!(at_cap.symbols.len(), 256);
        assert_eq!(decode(&plan(&at_cap, columns, rows, 1, None, None).unwrap()), unpack(&data, columns, rows));
        let (data, columns, _) = squares(257);
        assert!(page(&data, columns).is_none());
    }

    /// More than 10,000 marks are placed by several text regions, each of at most 10,000 (more
    /// than the decoder PdfCraft renders with accepts in one), over bands of rows, all referring to
    /// the dictionary, which each region but the last keeps retained; the generic region comes
    /// after them, every segment numbered one more than the one before.
    #[test]
    fn more_than_10000_marks_are_placed_by_several_regions() {
        // 25,000 copies of two 17 × 3 shapes, 256 a line, the lines 5 rows apart, and a bar too
        // tall to be a symbol beside them.
        let mut picture = vec![vec![0u8; 256 * 18 + 100]; 25_000usize.div_ceil(256) * 5];
        for k in 0..25_000 {
            let (x, y) = (k % 256 * 18, k / 256 * 5);
            picture[y][x..x + 17].fill(1);
            picture[y + 2][x..x + 17].fill(1);
            picture[y + 1][x + k % 2] = 1;
        }
        for row in &mut picture[0..300] {
            row[256 * 18 + 50] = 1;
        }
        let data = samples(&picture);
        let (columns, rows) = (picture[0].len() as u32, picture.len() as u32);
        let coded = encode_jbig2(&data, columns, rows).unwrap();
        let region = |number, kept| (number, 7, false, vec![(1, kept)]);
        assert_eq!(
            segments(&coded),
            [(0, 48, false, vec![]), (1, 0, true, vec![]), region(2, true), region(3, true), region(4, false), (5, 39, false, vec![])]
        );
        assert_eq!(decode(&coded), unpack(&data, columns, rows));
        assert_eq!(every_plan_round_trips(&picture), 8);
    }

    /// A page of text whose letters repeat exactly, as rendered text does, keeps each letter once:
    /// a symbol dictionary and a text region, much smaller than the generic coding.
    #[test]
    fn repeated_marks_are_stored_once() {
        let picture = text(1700, 400, &glyphs(40, 7), 11);
        let data = samples(&picture);
        let coded = encode_jbig2(&data, 1700, 400).unwrap();
        let generic = encode_jbig2_generic(&data, 1700, 400).unwrap();
        // The dictionary is retained for the text region, which refers to it (7.2.4, 7.2.5).
        assert_eq!(segments(&coded), [(0, 48, false, vec![]), (1, 0, true, vec![]), (2, 7, false, vec![(1, false)])]);
        assert_eq!(decode(&coded), unpack(&data, 1700, 400));
        assert!(coded.len() * 2 < generic.len(), "symbols {} bytes, generic {}", coded.len(), generic.len());
        assert_eq!(every_plan_round_trips(&picture), 8);
    }

    /// Marks too large to be symbols (a frame, a rule) go to a generic region over their box, and
    /// with the second plan so do marks that occur once; the page still decodes to its bits.
    #[test]
    fn large_and_lone_marks_are_left_to_a_generic_region() {
        let mut picture = text(900, 300, &glyphs(25, 3), 5);
        picture[10][100..800].fill(1);
        picture[290][100..800].fill(1);
        for row in &mut picture[10..=290] {
            row[100] = 1;
            row[799] = 1;
        }
        // A lone blob, unlike any letter.
        for (y, row) in picture[150..170].iter_mut().enumerate() {
            row[850 + y % 7] = 1;
        }
        let data = samples(&picture);
        let coded = encode_jbig2(&data, 900, 300).unwrap();
        assert_eq!(segment_types(&coded), [48, 0, 7, 39]);
        assert_eq!(decode(&coded), unpack(&data, 900, 300));
        assert_eq!(every_plan_round_trips(&picture), 8);
    }

    /// Marks on every edge, marks touching only at a corner (one mark), a mark inside another's
    /// box, and marks over each other's boxes.
    #[test]
    fn marks_at_the_edges_and_corners_round_trip() {
        let dot = vec![vec![1u8, 1], vec![1, 1]];
        let ell = vec![vec![1u8, 0, 0], vec![1, 0, 0], vec![1, 1, 1]];
        let mut picture = vec![vec![0u8; 41]; 23];
        for (x, y) in [(0, 0), (39, 0), (0, 21), (39, 21), (20, 0), (0, 10), (39, 10), (20, 21)] {
            draw(&mut picture, &dot, x, y);
        }
        for (x, y) in [(10, 5), (28, 14), (12, 14)] {
            draw(&mut picture, &ell, x, y);
        }
        // Corners touching: one mark (`marks_join_at_corners_and_large_ones_are_left_out` checks it).
        picture[3][6] = 1;
        picture[4][7] = 1;
        picture[5][8] = 1;
        // A ring with a dot inside, and an ell over the ring's box.
        for i in 0..7 {
            picture[8][20 + i] = 1;
            picture[14][20 + i] = 1;
            picture[8 + i][20] = 1;
            picture[8 + i][26] = 1;
        }
        draw(&mut picture, &dot, 22, 10);
        draw(&mut picture, &ell, 25, 15);
        assert_eq!(every_plan_round_trips(&picture), 8);
    }

    /// One shape, however often, would need symbol IDs of no bits: that image is coded as a generic
    /// region, as are a blank page and pure noise.
    #[test]
    fn one_shape_blank_pages_and_noise_are_coded_generically() {
        let letter = &glyphs(1, 9)[..];
        let mut r = Lcg(1);
        let noise: Bitmap = (0..60).map(|_| (0..200).map(|_| u8::from(r.below(2) == 1)).collect()).collect();
        for picture in [text(600, 200, letter, 2), vec![vec![0u8; 300]; 40], noise] {
            let (columns, rows) = (picture[0].len() as u32, picture.len() as u32);
            let data = samples(&picture);
            assert_eq!(encode_jbig2(&data, columns, rows).unwrap(), encode_jbig2_generic(&data, columns, rows).unwrap());
        }
    }

    /// Each segment's page association (7.2.6): the byte after the numbers it refers to.
    fn associations(data: &[u8]) -> Vec<u8> {
        let (mut pages, mut at) = (Vec::new(), 0);
        while at < data.len() {
            let refers = usize::from(data[at + 5] >> 5);
            pages.push(data[at + 6 + refers]);
            let length_at = at + 6 + refers + 1;
            at = length_at + 4 + u32::from_be_bytes(data[length_at..length_at + 4].try_into().unwrap()) as usize;
        }
        pages
    }

    /// `pictures`, `columns` × `rows` each, coded sharing a dictionary; each image's stream, which
    /// has to be there, decodes with the globals to its picture.
    fn shared_round_trip(pictures: &[Bitmap]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let (columns, rows) = (pictures[0][0].len() as u32, pictures[0].len() as u32);
        let data: Vec<Vec<u8>> = pictures.iter().map(|p| samples(p)).collect();
        let images: Vec<(&[u8], u32, u32)> = data.iter().map(|d| (&d[..], columns, rows)).collect();
        let shared = encode_jbig2_shared(&images).unwrap().unwrap();
        let (globals, coded) = (shared.globals, shared.images);
        let coded: Vec<Vec<u8>> = coded.into_iter().map(Option::unwrap).collect();
        for (d, c) in data.iter().zip(&coded) {
            assert_eq!(decode_with(c, Some(&globals)), unpack(d, columns, rows));
        }
        (globals, coded)
    }

    /// `font` with a stem 20 pixels tall down each glyph's left side: a font sharing no glyph with
    /// any [`glyphs`] gives, whose glyphs are at most 18 pixels tall.
    fn stemmed(font: &[Bitmap]) -> Vec<Bitmap> {
        font.iter()
            .map(|g| {
                (0..g.len().max(20)).map(|y| std::iter::once(1).chain(g.get(y).cloned().unwrap_or_else(|| vec![0; g[0].len()])).collect()).collect()
            })
            .collect()
    }

    /// Mark `k` of a set of distinct ones: a stroke 33 pixels long with a pixel above every other
    /// column for each 1 bit of `k`, below 2^17.
    fn comb(k: usize) -> Bitmap {
        let mut g = vec![vec![0u8; 33]; 2];
        g[1].fill(1);
        for bit in 0..17 {
            g[0][2 * bit] = u8::from(k >> bit & 1 == 1);
        }
        g
    }

    /// The combs `ks`, 120 a row, the rows four pixels apart.
    fn combs(ks: impl ExactSizeIterator<Item = usize>) -> Bitmap {
        let mut page = vec![vec![0u8; 120 * 36]; ks.len().div_ceil(120) * 4];
        for (i, k) in ks.enumerate() {
            draw(&mut page, &comb(k), (i % 120 * 36) as i64, (i / 120 * 4) as i64);
        }
        page
    }

    /// The symbols a symbol dictionary segment exports (SDNUMEXSYMS, 7.4.2.1.4), in a stream
    /// that is that segment alone: after its header, flags and adaptive pixels.
    fn exported(globals: &[u8]) -> usize {
        u32::from_be_bytes(globals[21..25].try_into().unwrap()) as usize
    }

    /// Pages in one font share its letters: one symbol dictionary, segment 0, associated with no
    /// page and retained (7.2.4, 7.2.6), which each page's text region refers to first, its own
    /// segments numbered after it (7.2.5). Together they take less than they take alone.
    #[test]
    fn pages_in_one_font_share_one_dictionary() {
        let font = glyphs(40, 7);
        let pictures: Vec<Bitmap> = (0..4).map(|seed| text(1200, 300, &font, 20 + seed)).collect();
        let (globals, coded) = shared_round_trip(&pictures);
        assert_eq!(segments(&globals), [(0, 0, true, vec![])]);
        assert_eq!(associations(&globals), [0]);
        let (mut shared, mut alone) = (globals.len(), 0);
        for (picture, c) in pictures.iter().zip(&coded) {
            let segments = segments(c);
            assert_eq!(segments.first().map(|s| s.0), Some(1), "the page's own segments follow");
            assert!(segments.iter().filter(|s| s.1 == 7).all(|s| s.3.first() == Some(&(0, false))), "{segments:?}");
            assert!(associations(c).iter().all(|page| *page == 1));
            shared += c.len();
            alone += encode_jbig2(&samples(picture), 1200, 300).unwrap().len();
        }
        assert!(shared < alone, "shared {shared} bytes, alone {alone}");
    }

    /// Two pages alike place every mark from the shared dictionary and have none of their own; a
    /// page with letters of its own as well keeps them in its own dictionary, referred to after the
    /// shared one (its symbols' IDs follow), and the page with nothing shared has no stream.
    #[test]
    fn what_a_page_has_alone_stays_its_own() {
        let (font, other) = (glyphs(30, 7), stemmed(&glyphs(20, 99)));
        let same = text(900, 240, &font, 3);
        let mut more = text(900, 240, &font, 4);
        more.extend(text(900, 120, &other, 5));
        let (globals, coded) = shared_round_trip(&[same.clone(), same.clone()]);
        assert!(!globals.is_empty());
        assert!(coded.iter().all(|c| segment_types(c) == [48, 7]), "{:?}", coded.iter().map(|c| segment_types(c)).collect::<Vec<_>>());
        let with_more = |p: &Bitmap| p.iter().cloned().chain(std::iter::repeat_n(vec![0u8; 900], 120)).collect::<Bitmap>();
        let (_, coded) = shared_round_trip(&[with_more(&same), more]);
        assert_eq!(segment_types(&coded[1])[..3], [48, 0, 7]);
        assert_eq!(segments(&coded[1])[2].3, [(0, false), (2, false)]);
        let alone = text(900, 360, &other, 6);
        let data: Vec<Vec<u8>> = [&same, &same, &alone].iter().map(|p| samples(p)).collect();
        let images: Vec<(&[u8], u32, u32)> = vec![(&data[0], 900, 240), (&data[1], 900, 240), (&data[2], 900, 360)];
        let (_, coded) = encode_shared(&images).unwrap();
        assert!(coded[0].is_some() && coded[1].is_some() && coded[2].is_none());
    }

    /// Images that share fewer than two marks share nothing; two are enough.
    #[test]
    fn images_with_nothing_in_common_share_nothing() {
        let font = glyphs(30, 7);
        let (a, b) = (samples(&text(600, 200, &font, 1)), samples(&text(600, 200, &stemmed(&font), 1)));
        assert!(encode_shared(&[(&a, 600, 200), (&b, 600, 200)]).is_none());
        assert!(encode_shared(&[(&a, 600, 200)]).is_none(), "one image");
        assert!(encode_shared(&[]).is_none());
        // The two commonest letters of the first page, below the last line of the second.
        let mut two = text(600, 200, &stemmed(&font), 1);
        draw(&mut two, &font[0], 10, 180);
        draw(&mut two, &font[1], 100, 180);
        let two = samples(&two);
        let (globals, coded) = encode_shared(&[(&a, 600, 200), (&two, 600, 200)]).unwrap();
        assert_eq!(exported(&globals), 2);
        assert_eq!(decode_with(coded[1].as_ref().unwrap(), Some(&globals)), unpack(&two, 600, 200));
    }

    /// The images of a group are checked as `encode_jbig2_generic` checks one, and an image it
    /// would refuse refuses the group.
    #[test]
    fn a_group_with_an_image_the_encoder_refuses_is_refused() {
        let page = samples(&text(600, 200, &glyphs(30, 7), 1));
        assert!(matches!(encode_jbig2_shared(&[(&page, 600, 200), (&page[..10], 600, 200)]), Err(FilterError::Corrupt { .. })));
        assert!(matches!(encode_jbig2_shared(&[(&page, 600, 200), (&page, 0, 200)]), Err(FilterError::Unsupported(_))));
        assert!(encode_jbig2_shared(&[(&page, 600, 200), (&page, 600, 200)]).unwrap().is_some());
    }

    /// A group holds its distinct marks within what one image's are held to: past 65,536 a mark
    /// first seen later isn't held, so isn't shared, while those held count every image they're
    /// in. Ten marks of the first image are in two images after it, with 464 first seen there.
    #[test]
    fn past_the_marks_a_group_holds_later_ones_arent_shared() {
        let (a, b) = (samples(&combs(0..40_000)), samples(&combs(40_000..65_536)));
        let late = combs((0..10).chain(65_536..66_000).collect::<Vec<_>>().into_iter());
        let (rows, late) = (late.len() as u32, samples(&late));
        let (columns, a_rows, b_rows) = (120 * 36, 40_000u32.div_ceil(120) * 4, 25_536u32.div_ceil(120) * 4);
        let images = [(&a[..], columns, a_rows), (&b[..], columns, b_rows), (&late[..], columns, rows), (&late[..], columns, rows)];
        let (globals, coded) = encode_shared(&images).unwrap();
        assert_eq!(exported(&globals), 10);
        for c in &coded[2..] {
            assert_eq!(decode_with(c.as_ref().unwrap(), Some(&globals)), unpack(&late, columns, rows));
        }
    }

    /// Past what one dictionary shares, it keeps the marks the most images hold: 20,000 shared,
    /// the first 10,000 in all three images and the rest in two. The third image then places all
    /// its marks from it, and the other two code the rest themselves (in a dictionary of their own
    /// or a generic region, whichever is smaller).
    #[test]
    fn past_the_shared_limit_the_most_widely_held_marks_are_shared() {
        let (both, first) = (combs(0..20_000), combs(0..10_000));
        let rows = both.len();
        let first: Bitmap = first.iter().cloned().chain(std::iter::repeat_n(vec![0u8; 120 * 36], rows - first.len())).collect();
        let (globals, coded) = shared_round_trip(&[both.clone(), both, first]);
        assert_eq!(exported(&globals), MAX_SHARED_SYMBOLS);
        assert_eq!(segment_types(&coded[2]), [48, 7]);
        assert!(segment_types(&coded[0]).iter().any(|t| *t == 0 || *t == 39), "{:?}", segment_types(&coded[0]));
    }

    proptest::proptest! {
        /// Glyphs scattered over pages of one font: whatever is shared, every page with a stream
        /// decodes with the globals to its picture.
        #[test]
        fn shared_groups_round_trip(
            columns in 1usize..200,
            rows in 1usize..100,
            shapes in 2usize..8,
            n in 0usize..60,
            pages in 2usize..5,
            seed in proptest::num::u64::ANY,
        ) {
            let font = glyphs(shapes, seed);
            let data: Vec<Vec<u8>> = (0..pages).map(|p| samples(&scatter(columns, rows, &font, seed ^ p as u64, n))).collect();
            let (c, r) = (columns as u32, rows as u32);
            let images: Vec<(&[u8], u32, u32)> = data.iter().map(|d| (&d[..], c, r)).collect();
            if let Some((globals, coded)) = encode_shared(&images) {
                for (d, coded) in data.iter().zip(coded) {
                    if let Some(coded) = coded {
                        proptest::prop_assert_eq!(decode_with(&coded, Some(&globals)), unpack(d, c, r));
                    }
                }
            }
        }

        /// Glyphs anywhere, over the edges and each other, with stray pixels: every plan decodes to
        /// the picture, and `encode_jbig2` is never larger than the generic coding.
        #[test]
        fn scattered_marks_round_trip(
            columns in 1usize..240,
            rows in 1usize..120,
            shapes in 2usize..10,
            n in 0usize..80,
            seed in proptest::num::u64::ANY,
            stray in proptest::collection::vec((0usize..240, 0usize..120), 0..20),
        ) {
            let mut picture = scatter(columns, rows, &glyphs(shapes, seed), seed, n);
            for (x, y) in stray {
                if let Some(cell) = picture.get_mut(y).and_then(|r| r.get_mut(x)) {
                    *cell ^= 1;
                }
            }
            every_plan_round_trips(&picture);
            let data = samples(&picture);
            let (c, r) = (columns as u32, rows as u32);
            let coded = encode_jbig2(&data, c, r).unwrap();
            proptest::prop_assert!(coded.len() <= encode_jbig2_generic(&data, c, r).unwrap().len());
            proptest::prop_assert_eq!(decode(&coded), unpack(&data, c, r));
        }

        /// Whatever it is given, it returns.
        #[test]
        fn any_input_returns(data in proptest::collection::vec(0u8..=255, 0..600), columns in 0u32..5000, rows in 0u32..80) {
            let _ = encode_jbig2(&data, columns, rows);
            let _ = encode(&data, columns, rows);
            let _ = encode_shared(&[(&data, columns, rows), (&data, columns, rows.saturating_sub(1))]);
        }
    }
}
