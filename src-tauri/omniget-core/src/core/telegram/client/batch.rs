use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde_json::json;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::api::{self, TelegramMediaItem};
use super::auth::TelegramSessionHandle;
use super::events::{self, DownloadReporter};
use super::template::{self, NameCtx};

#[derive(Debug, Clone)]
pub struct Planned {
    pub chat_id: i64,
    pub chat_type: String,
    pub message_id: i32,
    pub file_name: String,
    pub file_size: u64,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub ok: u32,
    pub skipped: u32,
    pub failed: u32,
    pub deleted: u32,
    pub duplicates: u32,
    pub cancelled: bool,
    pub paths: Vec<PathBuf>,
}

pub fn plan_item(
    root: &std::path::Path,
    template: Option<&str>,
    default: Option<&str>,
    chat_id: i64,
    chat_type: &str,
    chat_title: &str,
    item: &TelegramMediaItem,
) -> Planned {
    let ctx = NameCtx {
        chat: chat_title.to_string(),
        chat_id,
        msg_id: item.message_id,
        date: item.date,
        file_name: item.file_name.clone(),
        media_type: item.media_type.clone(),
        caption: item.caption.clone(),
        grouped_id: item.grouped_id,
    };
    Planned {
        chat_id,
        chat_type: chat_type.to_string(),
        message_id: item.message_id,
        file_name: item.file_name.clone(),
        file_size: item.file_size,
        path: template::resolve(root, template, default, &ctx),
    }
}

pub fn dedupe_paths(items: &mut [Planned]) {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for it in items.iter_mut() {
        if !seen.insert(it.path.clone()) {
            let base = it.path.clone();
            it.path = (1u32..)
                .map(|n| template::numbered_path(&base, n))
                .find(|candidate| !seen.contains(candidate))
                .unwrap_or(base);
            seen.insert(it.path.clone());
        }
    }
}

pub fn already_there(it: &Planned) -> bool {
    std::fs::metadata(&it.path)
        .map(|m| m.len() > 0 && (it.file_size == 0 || m.len() == it.file_size))
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
pub async fn start(
    handle: TelegramSessionHandle,
    active: super::ActiveDownloads,
    mut items: Vec<Planned>,
    title: String,
    root: PathBuf,
    concurrent: usize,
    skip_same: bool,
    dedupe: bool,
) -> (u64, JoinHandle<Outcome>) {
    dedupe_paths(&mut items);
    let batch_id = events::next_id();
    let cancel = CancellationToken::new();
    active
        .lock()
        .await
        .insert(batch_id, (format!("tg-batch:{}", batch_id), cancel.clone()));
    let total_files = items.len().max(1) as u32;
    let total_bytes: u64 = items.iter().map(|i| i.file_size).sum();

    let join = tokio::spawn(async move {
        let mut agg = DownloadReporter::new(
            batch_id,
            &title,
            "batch",
            None,
            Some(total_bytes).filter(|b| *b > 0),
            Some(root.to_string_lossy().into()),
        );
        agg.progress(0.0);
        let agg = Arc::new(tokio::sync::Mutex::new(agg));
        let sem = Arc::new(Semaphore::new(concurrent.max(1)));
        let done = Arc::new(AtomicU32::new(0));
        let ok = Arc::new(AtomicU32::new(0));
        let skipped = Arc::new(AtomicU32::new(0));
        let failed = Arc::new(AtomicU32::new(0));
        let deleted = Arc::new(AtomicU32::new(0));
        let duplicates = Arc::new(AtomicU32::new(0));
        let paths = Arc::new(tokio::sync::Mutex::new(Vec::<PathBuf>::new()));
        let mut handles = Vec::new();

        for mut item in items {
            let handle = handle.clone();
            let sem = sem.clone();
            let cancel = cancel.clone();
            let agg = agg.clone();
            let (done, ok, skipped, failed, deleted, duplicates, paths) = (
                done.clone(),
                ok.clone(),
                skipped.clone(),
                failed.clone(),
                deleted.clone(),
                duplicates.clone(),
                paths.clone(),
            );
            handles.push(tokio::spawn(async move {
                let finish = |agg: Arc<tokio::sync::Mutex<DownloadReporter>>| async move {
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    agg.lock()
                        .await
                        .progress(n as f64 / total_files as f64 * 100.0);
                };
                if skip_same && already_there(&item) {
                    skipped.fetch_add(1, Ordering::Relaxed);
                    events::batch_file_status(batch_id, item.message_id, "skipped", 100.0, None);
                    finish(agg).await;
                    return;
                }
                let claim = template::claim_unique_path(&item.path);
                item.path = claim.path().to_path_buf();
                let _permit = tokio::select! {
                    p = sem.acquire_owned() => match p { Ok(p) => p, Err(_) => return },
                    _ = cancel.cancelled() => return,
                };
                if cancel.is_cancelled() {
                    return;
                }
                let mut rep = DownloadReporter::new(
                    events::next_id(),
                    &item.file_name,
                    "item",
                    Some(batch_id),
                    Some(item.file_size).filter(|s| *s > 0),
                    Some(item.path.to_string_lossy().into()),
                );
                events::batch_file_status(batch_id, item.message_id, "downloading", 0.0, None);
                let (tx, mut rx) = mpsc::channel::<f64>(32);
                let mid = item.message_id;
                let pf = tokio::spawn(async move {
                    while let Some(p) = rx.recv().await {
                        rep.progress(p);
                        events::batch_file_status(batch_id, mid, "downloading", p, None);
                    }
                    rep
                });
                let result = Box::pin(api::download_media_with_retry_ex(
                    &handle,
                    item.chat_id,
                    &item.chat_type,
                    item.message_id,
                    &item.path,
                    tx,
                    &cancel,
                    dedupe,
                ))
                .await;
                let rep = pf.await.ok();
                match result {
                    Ok(size) => {
                        ok.fetch_add(1, Ordering::Relaxed);
                        paths.lock().await.push(item.path.clone());
                        super::bandwidth::record_bytes(
                            &crate::core::telegram::host::data_dir(),
                            size,
                        );
                        events::batch_file_status(batch_id, item.message_id, "done", 100.0, None);
                        if let Some(r) = rep {
                            r.complete(
                                true,
                                None,
                                Some(item.path.to_string_lossy().into()),
                                Some(size),
                                Some(1),
                                None,
                            );
                        }
                    }
                    Err(e) if super::dedupe::duplicate_of(&e.to_string()).is_some() => {
                        let msg = e.to_string();
                        let existing = super::dedupe::duplicate_of(&msg).unwrap_or_default();
                        duplicates.fetch_add(1, Ordering::Relaxed);
                        events::batch_file_duplicate(batch_id, item.message_id, existing);
                        if let Some(r) = rep {
                            r.complete(true, None, Some(existing.to_string()), None, Some(0), None);
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let upper = msg.to_uppercase();
                        let status = if msg.contains("cancelled") {
                            "cancelled"
                        } else if upper.contains("MESSAGE_ID_INVALID")
                            || upper.contains("MESSAGE_DELETED")
                            || msg.contains("not found")
                        {
                            deleted.fetch_add(1, Ordering::Relaxed);
                            "skipped_deleted"
                        } else {
                            failed.fetch_add(1, Ordering::Relaxed);
                            "error"
                        };
                        events::batch_file_status(
                            batch_id,
                            item.message_id,
                            status,
                            0.0,
                            Some(msg.clone()),
                        );
                        if let Some(r) = rep {
                            r.complete(false, Some(msg), None, None, None, None);
                        }
                    }
                }
                finish(agg).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }
        active.lock().await.remove(&batch_id);
        let outcome = Outcome {
            ok: ok.load(Ordering::Relaxed),
            skipped: skipped.load(Ordering::Relaxed),
            failed: failed.load(Ordering::Relaxed),
            deleted: deleted.load(Ordering::Relaxed),
            duplicates: duplicates.load(Ordering::Relaxed),
            cancelled: cancel.is_cancelled(),
            paths: std::mem::take(&mut *paths.lock().await),
        };
        let mut errs = Vec::new();
        if outcome.cancelled {
            errs.push("Batch cancelled".to_string());
        }
        if outcome.failed > 0 {
            errs.push(format!("{} files failed", outcome.failed));
        }
        if outcome.deleted > 0 {
            errs.push(format!(
                "{} messages deleted/invalid (skipped)",
                outcome.deleted
            ));
        }
        agg.lock().await.complete_with(
            outcome.failed == 0 && !outcome.cancelled,
            (!errs.is_empty()).then(|| errs.join("; ")),
            Some(root.to_string_lossy().into()),
            None,
            Some(outcome.ok + outcome.skipped),
            (outcome.deleted > 0).then_some(outcome.deleted),
            json!({ "duplicate_count": outcome.duplicates }),
        );
        outcome
    });
    (batch_id, join)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(id: i32, name: &str) -> Planned {
        Planned {
            chat_id: 1,
            chat_type: "channel".into(),
            message_id: id,
            file_name: name.into(),
            file_size: 10,
            path: PathBuf::from("/d").join(name),
        }
    }

    #[test]
    fn same_name_in_a_batch_is_numbered_from_one() {
        let mut items = vec![
            planned(50, "1 - Aula.mp4"),
            planned(51, "1 - Aula.mp4"),
            planned(52, "1 - Aula.mp4"),
        ];
        dedupe_paths(&mut items);
        let names: Vec<String> = items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["1 - Aula.mp4", "1 - Aula (1).mp4", "1 - Aula (2).mp4"]
        );
    }

    #[test]
    fn numbering_skips_a_name_already_taken_in_the_batch() {
        let mut items = vec![
            planned(50, "Aula.mp4"),
            planned(51, "Aula (1).mp4"),
            planned(52, "Aula.mp4"),
        ];
        dedupe_paths(&mut items);
        assert_eq!(items[2].path, PathBuf::from("/d/Aula (2).mp4"));
    }
}
