use std::path::{Path, PathBuf};

use pdfcraft_cos::Document;
use pdfcraft_organize::{combine_grouped, extract_pages, insert_blank_page, rotate_pages, Run};
use serde::Deserialize;

use super::PdfError;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PageSpec {
    Page {
        source: usize,
        page: usize,
        #[serde(default)]
        rotate: i64,
    },
    Blank {
        width: f64,
        height: f64,
    },
}

pub struct Source<'a> {
    pub title: &'a str,
    pub doc: &'a Document,
}

fn runs(specs: &[PageSpec]) -> Vec<(usize, Vec<usize>)> {
    let mut runs: Vec<(usize, Vec<usize>)> = Vec::new();
    for spec in specs {
        if let PageSpec::Page { source, page, .. } = spec {
            match runs.last_mut() {
                Some((s, pages)) if s == source => pages.push(*page),
                _ => runs.push((*source, vec![*page])),
            }
        }
    }
    runs
}

pub fn assemble(sources: &[Source<'_>], specs: &[PageSpec]) -> Result<Document, PdfError> {
    if specs.is_empty() {
        return Err(PdfError::Empty);
    }
    for spec in specs {
        match spec {
            PageSpec::Page { source, rotate, .. } => {
                if *source >= sources.len() {
                    return Err(PdfError::NoSuchPage(*source));
                }
                if rotate % 90 != 0 {
                    return Err(PdfError::Invalid(
                        "rotation must be a multiple of 90 degrees",
                    ));
                }
            }
            PageSpec::Blank { width, height } => {
                if !(width.is_finite() && height.is_finite() && *width >= 1.0 && *height >= 1.0) {
                    return Err(PdfError::Invalid("blank page size"));
                }
            }
        }
    }
    let runs = runs(specs);
    let mut out = match runs.as_slice() {
        [] => Document::new_empty(),
        [(source, pages)] => extract_pages(sources[*source].doc, pages)?,
        _ => {
            let list: Vec<Run<'_>> = runs
                .iter()
                .map(|(s, pages)| {
                    (
                        *s,
                        sources[*s].title,
                        sources[*s].doc,
                        Some(pages.as_slice()),
                    )
                })
                .collect();
            combine_grouped(&list)?
        }
    };
    for (at, spec) in specs.iter().enumerate() {
        if let PageSpec::Blank { width, height } = spec {
            insert_blank_page(&mut out, at, *width, *height)?;
        }
    }
    for (at, spec) in specs.iter().enumerate() {
        if let PageSpec::Page { rotate, .. } = spec {
            if rotate.rem_euclid(360) != 0 {
                rotate_pages(&mut out, &[at], *rotate)?;
            }
        }
    }
    Ok(out)
}

pub fn write_outputs(
    sources: &[PathBuf],
    outputs: &[(PathBuf, Vec<PageSpec>)],
    overwrite: bool,
) -> Result<Vec<(PathBuf, u64)>, PdfError> {
    let used: Vec<bool> = (0..sources.len())
        .map(|i| {
            outputs.iter().any(|(_, specs)| {
                specs
                    .iter()
                    .any(|s| matches!(s, PageSpec::Page { source, .. } if *source == i))
            })
        })
        .collect();
    let docs = sources
        .iter()
        .zip(&used)
        .map(|(path, used)| {
            if *used {
                super::files::open(path)
            } else {
                Ok(Document::new_empty())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let titles: Vec<String> = sources
        .iter()
        .map(|p| {
            p.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    let list: Vec<Source<'_>> = docs
        .iter()
        .zip(&titles)
        .map(|(doc, title)| Source { title, doc })
        .collect();
    let mut planned = Vec::with_capacity(outputs.len());
    for (path, specs) in outputs {
        planned.push((path.as_path(), assemble(&list, specs)?));
    }
    let mut written = Vec::with_capacity(planned.len());
    for (path, doc) in planned {
        written.push(super::files::save(&doc, Path::new(path), overwrite)?);
    }
    Ok(written)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pdfcraft_cos::{Dict, Object, Stream};
    use std::sync::Arc;

    pub(crate) fn numbered(count: usize, width: f64) -> Document {
        let mut doc = Document::new_empty();
        for i in 0..count {
            let page = insert_blank_page(&mut doc, i, width, 400.0 + i as f64).unwrap();
            let content = doc.add(Object::Stream(Stream::flate(
                Dict::new(),
                format!("0 0 {} 10 re f", i + 1).as_bytes(),
            )));
            doc.update_dict(page, |d| d.set(b"Contents".to_vec(), Object::Ref(content)))
                .unwrap();
        }
        doc
    }

    fn reopen(doc: &Document) -> Document {
        Document::open(Arc::new(super::super::files::to_bytes(doc).unwrap())).unwrap()
    }

    fn heights(doc: &Document) -> Vec<f64> {
        pdfcraft_organize::page_boxes(doc)
            .unwrap()
            .into_iter()
            .map(|b| b[0][3] - b[0][1])
            .collect()
    }

    fn page(source: usize, page: usize, rotate: i64) -> PageSpec {
        PageSpec::Page {
            source,
            page,
            rotate,
        }
    }

    #[test]
    fn reorders_rotates_and_drops_pages_of_one_file() {
        let a = numbered(4, 300.0);
        let out = assemble(
            &[Source {
                title: "a",
                doc: &a,
            }],
            &[page(0, 3, 0), page(0, 0, 90), page(0, 2, -90)],
        )
        .unwrap();
        let out = reopen(&out);
        assert_eq!(heights(&out), vec![403.0, 400.0, 402.0]);
        let rotations: Vec<i64> = (0..3)
            .map(|i| pdfcraft_organize::page_rotation(&out, i).unwrap())
            .collect();
        assert_eq!(rotations, vec![0, 90, 270]);
    }

    #[test]
    fn interleaves_pages_from_several_files_and_inserts_blanks() {
        let a = numbered(2, 300.0);
        let b = numbered(3, 500.0);
        let specs = [
            page(1, 2, 0),
            PageSpec::Blank {
                width: 200.0,
                height: 100.0,
            },
            page(0, 0, 0),
            page(1, 0, 180),
            page(0, 1, 0),
        ];
        let out = assemble(
            &[
                Source {
                    title: "a",
                    doc: &a,
                },
                Source {
                    title: "b",
                    doc: &b,
                },
            ],
            &specs,
        )
        .unwrap();
        let out = reopen(&out);
        assert_eq!(heights(&out), vec![402.0, 100.0, 400.0, 400.0, 401.0]);
        let widths: Vec<f64> = pdfcraft_organize::page_boxes(&out)
            .unwrap()
            .into_iter()
            .map(|b| b[0][2] - b[0][0])
            .collect();
        assert_eq!(widths, vec![500.0, 200.0, 300.0, 500.0, 300.0]);
        assert_eq!(pdfcraft_organize::page_rotation(&out, 3).unwrap(), 180);
    }

    #[test]
    fn the_same_page_can_appear_twice() {
        let a = numbered(2, 300.0);
        let out = assemble(
            &[Source {
                title: "a",
                doc: &a,
            }],
            &[page(0, 1, 0), page(0, 1, 90)],
        )
        .unwrap();
        let out = reopen(&out);
        assert_eq!(heights(&out), vec![401.0, 401.0]);
        assert_eq!(pdfcraft_organize::page_rotation(&out, 0).unwrap(), 0);
        assert_eq!(pdfcraft_organize::page_rotation(&out, 1).unwrap(), 90);
    }

    #[test]
    fn rejects_bad_requests() {
        let a = numbered(1, 300.0);
        let sources = [Source {
            title: "a",
            doc: &a,
        }];
        assert!(matches!(assemble(&sources, &[]), Err(PdfError::Empty)));
        assert!(matches!(
            assemble(&sources, &[page(0, 5, 0)]),
            Err(PdfError::NoSuchPage(5))
        ));
        assert!(matches!(
            assemble(&sources, &[page(0, 0, 45)]),
            Err(PdfError::Invalid(_))
        ));
        assert!(matches!(
            assemble(&sources, &[page(3, 0, 0)]),
            Err(PdfError::NoSuchPage(3))
        ));
    }

    #[test]
    fn page_specs_parse_from_the_frontend_shape() {
        let specs: Vec<PageSpec> = serde_json::from_str(
            r#"[{"kind":"page","source":1,"page":4,"rotate":270},{"kind":"page","source":0,"page":0},{"kind":"blank","width":595,"height":842}]"#,
        )
        .unwrap();
        assert_eq!(
            specs,
            vec![
                page(1, 4, 270),
                page(0, 0, 0),
                PageSpec::Blank {
                    width: 595.0,
                    height: 842.0
                }
            ]
        );
    }

    #[test]
    fn split_writes_one_file_per_range_without_touching_sources() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.pdf");
        super::super::files::save(&numbered(3, 300.0), &a, false).unwrap();
        super::super::files::save(&numbered(2, 500.0), &b, false).unwrap();
        let before = std::fs::read(&a).unwrap();
        let outputs = vec![
            (a.clone(), vec![page(0, 0, 0), page(0, 1, 0)]),
            (
                dir.path().join("rest.pdf"),
                vec![page(0, 2, 0), page(1, 1, 0)],
            ),
        ];
        let written = write_outputs(&[a.clone(), b], &outputs, false).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), before);
        assert_eq!(written[0].0, dir.path().join("a (1).pdf"));
        assert_eq!(written[1].0, dir.path().join("rest.pdf"));
        let counts: Vec<usize> = written
            .iter()
            .map(|(p, _)| super::super::files::inspect(p).pages)
            .collect();
        assert_eq!(counts, vec![2, 2]);
    }

    #[test]
    fn unused_sources_are_never_opened() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        super::super::files::save(&numbered(1, 300.0), &a, false).unwrap();
        let outputs = vec![(dir.path().join("out.pdf"), vec![page(0, 0, 90)])];
        let sources = [a, dir.path().join("gone.pdf")];
        assert_eq!(write_outputs(&sources, &outputs, false).unwrap().len(), 1);
    }

    #[test]
    fn a_bad_output_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        super::super::files::save(&numbered(1, 300.0), &a, false).unwrap();
        let outputs = vec![
            (dir.path().join("one.pdf"), vec![page(0, 0, 0)]),
            (dir.path().join("two.pdf"), vec![page(0, 9, 0)]),
        ];
        assert!(write_outputs(&[a], &outputs, false).is_err());
        assert!(!dir.path().join("one.pdf").exists());
    }
}
