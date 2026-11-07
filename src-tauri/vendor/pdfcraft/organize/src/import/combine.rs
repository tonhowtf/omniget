use super::carry::{Mark, add_outline, collect_attachments, kept_bookmarks, set_attachments, source_outline, write_outline_level};
use super::{Dict, Document, FieldConflicts, HashMap, ObjRef, Object, OrganizeError, import_pages_mapped, rebuild};

/// A new document containing copies of `pages` from `src` (Extract Pages / Split).
/// Document information (title, author…) is carried over, and so is the print standard the
/// source declares (PDF/X: its output intents and identification, #263), and the bookmarks that
/// go to the copied pages.
pub fn extract_pages(src: &Document, pages: &[usize]) -> Result<Document, OrganizeError> {
    let mut out = Document::new_empty();
    // One source: nothing to deduplicate. Its fields keep their names (a source with two
    // same-named field roots stays one field, as it was), so Extract and Split never rename.
    let imported = import_pages_mapped(&mut out, src, pages, 0, FieldConflicts::Merge)?;
    // The bookmarks of these pages come along, mapped to the copies (#4020).
    let outline = kept_bookmarks(source_outline(src, imported.page_map));
    if !outline.is_empty() {
        let root = crate::outline::container(&mut out, &[])?;
        if let Some((first, last, _)) = write_outline_level(&mut out, root, &outline)? {
            out.update_dict(root, |d| {
                d.set(b"First".to_vec(), Object::Ref(first));
                d.set(b"Last".to_vec(), Object::Ref(last));
            })?;
            let top = crate::outline::children(&out, root);
            crate::outline::relink_and_recount(&mut out, root, &top)?;
        }
    }
    for key in crate::INFO_KEYS {
        if let Some(v) = crate::info(src, key) {
            crate::set_info(&mut out, key, &v)?;
        }
    }
    // After the title: the XMP packet repeats it.
    crate::pdfx::carry(&mut out, Some(src), &crate::pdfx::PrintStandard::of(src), imported.objects)?;
    // The pages share their source's resource dictionary, so the part would otherwise carry every
    // XObject the source lists — images and all (#204). Keep only what these pages draw.
    crate::prune::prune_unused_xobjects(&mut out)?;
    Ok(out)
}

/// How to split a document.
#[derive(Clone, Debug, PartialEq)]
pub enum SplitBy {
    /// Every `n` pages.
    PageCount(usize),
    /// Before each of these 0-based page indices (e.g. `[3, 7]` → 0–2, 3–6, 7–end).
    Before(Vec<usize>),
}

/// The page ranges (0-based, end-exclusive) a split produces.
pub fn split_ranges(page_count: usize, by: &SplitBy) -> Vec<std::ops::Range<usize>> {
    let mut cuts: Vec<usize> = match by {
        SplitBy::PageCount(n) => (1..).map(|i| i * (*n).max(1)).take_while(|c| *c < page_count).collect(),
        SplitBy::Before(v) => v.iter().copied().filter(|c| *c > 0 && *c < page_count).collect(),
    };
    cuts.sort_unstable();
    cuts.dedup();
    let mut ranges = Vec::new();
    let mut start = 0;
    for c in cuts.into_iter().chain(std::iter::once(page_count)) {
        if c > start {
            ranges.push(start..c);
            start = c;
        }
    }
    ranges
}

/// Split a document into several new documents.
pub fn split(src: &Document, by: &SplitBy) -> Result<Vec<Document>, OrganizeError> {
    let n = crate::page_count(src)?;
    split_ranges(n, by).into_iter().map(|r| extract_pages(src, &r.collect::<Vec<_>>())).collect()
}

/// What combining produced, beyond the document: the fields renamed to keep colliding names
/// separate (old fully-qualified name → new one), in order across all sources.
pub struct CombineOutcome {
    /// The combined document.
    pub doc: Document,
    /// Every field renamed while combining.
    pub renamed_fields: Vec<(String, String)>,
}

/// Combine whole documents, in order, into a new document. Each source gets a top-level
/// bookmark (its `title`) pointing to its first page, like Acrobat's Combine Files.
/// The source's own bookmarks are nested (collapsed) under its entry, and its document-level
/// attachments are carried over. When every source declares the same print standard (PDF/X
/// with the same output intent), so does the result.
pub fn combine(sources: &[(&str, &Document)]) -> Result<Document, OrganizeError> {
    combine_with(sources, FieldConflicts::Rename).map(|o| o.doc)
}

/// [`combine`] reporting what the conflict resolution renamed.
pub fn combine_with(sources: &[(&str, &Document)], conflicts: FieldConflicts) -> Result<CombineOutcome, OrganizeError> {
    let all: Vec<(&str, &Document, Option<&[usize]>)> = sources.iter().map(|(t, d)| (*t, *d, None)).collect();
    combine_selected_with(&all, conflicts)
}

/// Combine Files with chosen pages: each source contributes `pages` (0-based, in that order;
/// `None` for all of them). Bookmarks that point at pages left out lose their destination.
pub fn combine_selected(sources: &[(&str, &Document, Option<&[usize]>)]) -> Result<Document, OrganizeError> {
    combine_selected_with(sources, FieldConflicts::Rename).map(|o| o.doc)
}

/// [`combine_selected`] reporting what the conflict resolution renamed.
pub fn combine_selected_with(sources: &[(&str, &Document, Option<&[usize]>)], conflicts: FieldConflicts) -> Result<CombineOutcome, OrganizeError> {
    let mut combiner = Combiner::with_field_conflicts(conflicts);
    for (title, src, chosen) in sources {
        combiner.add(title, src, *chosen)?;
    }
    combiner.finish_with_renames()
}

/// Combine Files one source at a time, with the same result as [`combine_selected`]. [`Combiner::add`]
/// copies a source's pages into the result and keeps only what the rest of the combine needs: its
/// bookmarks (read and mapped to the copied pages), its attachments (copied), and its document only
/// when it declares output intents. So each source can be dropped as soon as it is added, and a long
/// list holds one source at a time rather than all of them.
pub struct Combiner {
    out: Document,
    marks: Vec<Mark>,
    attachments: Vec<(Vec<u8>, Object)>,
    /// The first source's print standard, and what its pages brought along.
    standard: Option<Standard>,
    /// Whether every source so far declares the first source's print standard.
    agreed: bool,
    /// How colliding field names are resolved as each source is added.
    field_conflicts: FieldConflicts,
    /// What adding sources renamed (old → new fully-qualified field name), in order.
    renamed_fields: Vec<(String, String)>,
}

/// The first source's print standard (see [`Combiner`]).
struct Standard {
    declared: crate::pdfx::PrintStandard,
    /// The source objects the copied pages brought along, and their copies.
    objects: HashMap<ObjRef, ObjRef>,
    /// The source itself, kept only for its output intents, which are copied from it.
    source: Option<Document>,
}

impl Default for Combiner {
    fn default() -> Self {
        Self::new()
    }
}

impl Combiner {
    /// A combine with no sources yet, renaming colliding field names as sources are added.
    pub fn new() -> Self {
        Self::with_field_conflicts(FieldConflicts::Rename)
    }

    /// [`Combiner::new`] resolving colliding field names as `conflicts` says.
    pub fn with_field_conflicts(field_conflicts: FieldConflicts) -> Self {
        Self {
            out: Document::new_empty(),
            marks: Vec::new(),
            attachments: Vec::new(),
            standard: None,
            agreed: true,
            field_conflicts,
            renamed_fields: Vec::new(),
        }
    }

    /// What adding sources has renamed so far (old → new fully-qualified field name).
    pub fn renamed_fields(&self) -> &[(String, String)] {
        &self.renamed_fields
    }

    /// Add `src`'s `pages` (0-based, in that order; `None` for all of them) after the pages added so far,
    /// under a bookmark titled `title`. `src` need not outlive the call.
    pub fn add(&mut self, title: &str, src: &Document, pages: Option<&[usize]>) -> Result<(), OrganizeError> {
        let n = crate::page_count(src)?;
        let pages: Vec<usize> = match pages {
            Some(p) => {
                if let Some(bad) = p.iter().find(|i| **i >= n) {
                    return Err(OrganizeError::NoSuchPage(*bad));
                }
                p.to_vec()
            }
            None => (0..n).collect(),
        };
        let at = crate::page_count(&self.out)?;
        let imported = import_pages_mapped(&mut self.out, src, &pages, at, self.field_conflicts)?;
        self.renamed_fields.extend(imported.renamed_fields);
        if let Some(first) = imported.pages.first() {
            let outline = source_outline(src, imported.page_map);
            self.marks.push(Mark { title: title.to_string(), page: *first, outline });
        }
        collect_attachments(&mut self.out, src, &mut self.attachments)?;
        let declared = crate::pdfx::PrintStandard::of(src);
        match self.standard.as_ref().map(|first| first.declared.same_as(&declared)) {
            Some(same) => self.agreed &= same,
            None => {
                let source = declared.declares_output_intents().then(|| src.clone());
                self.standard = Some(Standard { declared, objects: imported.objects, source });
            }
        }
        Ok(())
    }

    /// The combined document: the pages as added, the print standard of the first source when every
    /// source declares the same one, shared resources stored once, then the bookmarks and attachments.
    pub fn finish(self) -> Result<Document, OrganizeError> {
        self.finish_with_renames().map(|o| o.doc)
    }

    /// [`Combiner::finish`] reporting what the combine renamed.
    pub fn finish_with_renames(self) -> Result<CombineOutcome, OrganizeError> {
        let Combiner { mut out, marks, attachments, standard, agreed, renamed_fields, .. } = self;
        if let Some(first) = standard.filter(|_| agreed) {
            crate::pdfx::carry(&mut out, first.source.as_ref(), &first.declared, first.objects)?;
        }
        // Sources often share fonts, images and profiles (or are the same file): store them once.
        let all: Vec<ObjRef> = out.object_numbers().into_iter().map(|n| ObjRef::new(n, out.generation(n))).collect();
        crate::dedupe::dedupe_resources(&mut out, &all, false);
        add_outline(&mut out, &marks)?;
        set_attachments(&mut out, attachments)?;
        Ok(CombineOutcome { doc: out, renamed_fields })
    }
}

/// One run of pages for [`combine_grouped`]: the file it comes from (`group`: runs with the same
/// group are one file split around, and give the same document), its title, the document, and
/// its pages (0-based, in that order; `None` for all of them).
pub type Run<'a> = (usize, &'a str, &'a Document, Option<&'a [usize]>);

/// Combine Files where a file's pages may be split into several runs with other files' pages
/// between them. Each file (group) is copied once, all its pages together, then the pages are put
/// in the runs' order: its links between its own pages, its form fields and its attachments stay
/// whole, and it gets one top-level bookmark (the title of its first run) at its first page, its
/// own bookmarks nested. Otherwise as [`combine_selected`] (print standard, shared resources).
pub fn combine_grouped(runs: &[Run<'_>]) -> Result<Document, OrganizeError> {
    combine_grouped_with(runs, FieldConflicts::Rename).map(|o| o.doc)
}

/// [`combine_grouped`] reporting what the conflict resolution renamed.
pub fn combine_grouped_with(runs: &[Run<'_>], conflicts: FieldConflicts) -> Result<CombineOutcome, OrganizeError> {
    // Each file's pages, in the order its runs list them, and each run's place in that list.
    struct File<'a> {
        title: &'a str,
        src: &'a Document,
        pages: Vec<usize>,
    }
    let mut files: Vec<(usize, File<'_>)> = Vec::new();
    let mut slices: Vec<(usize, usize, usize)> = Vec::with_capacity(runs.len());
    for (group, title, src, chosen) in runs {
        let at = match files.iter().position(|(g, _)| g == group) {
            Some(at) => at,
            None => {
                files.push((*group, File { title, src, pages: Vec::new() }));
                files.len() - 1
            }
        };
        let Some((_, file)) = files.get_mut(at) else { continue };
        let n = crate::page_count(file.src)?;
        let pages: Vec<usize> = match chosen {
            Some(p) => {
                if let Some(bad) = p.iter().find(|i| **i >= n) {
                    return Err(OrganizeError::NoSuchPage(*bad));
                }
                p.to_vec()
            }
            None => (0..n).collect(),
        };
        slices.push((at, file.pages.len(), pages.len()));
        file.pages.extend(pages);
    }
    let mut out = Document::new_empty();
    let mut marks = Vec::new();
    let mut attachments = Vec::new();
    let mut copies: Vec<Vec<ObjRef>> = Vec::with_capacity(files.len());
    let mut renamed_fields = Vec::new();
    let mut standard: Option<(crate::pdfx::PrintStandard, &Document, HashMap<ObjRef, ObjRef>)> = None;
    let mut agreed = true;
    for (_, file) in &files {
        let at = crate::page_count(&out)?;
        let imported = import_pages_mapped(&mut out, file.src, &file.pages, at, conflicts)?;
        copies.push(imported.pages.clone());
        renamed_fields.extend(imported.renamed_fields);
        if let Some(first) = imported.pages.first() {
            marks.push(Mark { title: file.title.to_string(), page: *first, outline: source_outline(file.src, imported.page_map) });
        }
        collect_attachments(&mut out, file.src, &mut attachments)?;
        let declared = crate::pdfx::PrintStandard::of(file.src);
        match &standard {
            None => standard = Some((declared, file.src, imported.objects)),
            Some((first, _, _)) => agreed &= first.same_as(&declared),
        }
    }
    // The pages in the runs' order. A file's first run comes before its others, and its pages
    // were copied in run order, so its bookmark (its first copied page) is its first page shown.
    let mut order: Vec<(ObjRef, Dict)> = Vec::new();
    for (file, start, len) in slices {
        let run = copies.get(file).and_then(|c| c.get(start..start.saturating_add(len))).unwrap_or_default();
        order.extend(run.iter().map(|r| (*r, Dict::new())));
    }
    rebuild(&mut out, &order)?;
    if let Some((declared, src, objects)) = standard.filter(|_| agreed) {
        crate::pdfx::carry(&mut out, Some(src), &declared, objects)?;
    }
    let all: Vec<ObjRef> = out.object_numbers().into_iter().map(|n| ObjRef::new(n, out.generation(n))).collect();
    crate::dedupe::dedupe_resources(&mut out, &all, false);
    add_outline(&mut out, &marks)?;
    set_attachments(&mut out, attachments)?;
    Ok(CombineOutcome { doc: out, renamed_fields })
}
