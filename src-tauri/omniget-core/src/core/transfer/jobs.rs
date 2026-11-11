use serde::Serialize;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::watch;

use super::ops::Transferred;
use super::TransferError;

pub const MAX_RUNNING: usize = 4;
pub const MAX_KEPT: usize = 50;
pub const DEFAULT_WAIT_SECONDS: u64 = 20;
pub const MAX_WAIT_SECONDS: u64 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub transfer_id: String,
    pub direction: &'static str,
    pub status: Status,
    pub bytes: u64,
    pub total: Option<u64>,
    pub remote: String,
    pub local: PathBuf,
}

#[derive(Debug, Clone)]
struct State {
    snapshot: Snapshot,
    failure: Option<TransferError>,
}

#[derive(Clone)]
pub struct Reporter(watch::Sender<State>);

impl Reporter {
    pub fn total(&self, total: Option<u64>) {
        self.0.send_modify(|state| state.snapshot.total = total);
    }

    pub fn bytes(&self, bytes: u64) {
        self.0.send_modify(|state| state.snapshot.bytes = bytes);
    }
}

static JOBS: LazyLock<Mutex<VecDeque<watch::Receiver<State>>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

fn registry(
) -> Result<std::sync::MutexGuard<'static, VecDeque<watch::Receiver<State>>>, TransferError> {
    JOBS.lock()
        .map_err(|_| TransferError::Storage("the transfer list is unavailable".into()))
}

pub fn start<F>(
    direction: &'static str,
    remote: &str,
    local: PathBuf,
    work: impl FnOnce(Reporter) -> F,
) -> Result<String, TransferError>
where
    F: Future<Output = Result<Transferred, TransferError>> + Send + 'static,
{
    let id = uuid::Uuid::new_v4().simple().to_string();
    let (sender, receiver) = watch::channel(State {
        snapshot: Snapshot {
            transfer_id: id.clone(),
            direction,
            status: Status::Running,
            bytes: 0,
            total: None,
            remote: remote.to_string(),
            local,
        },
        failure: None,
    });
    {
        let mut jobs = registry()?;
        let running = jobs
            .iter()
            .filter(|job| job.borrow().snapshot.status == Status::Running)
            .count();
        if running >= MAX_RUNNING {
            return Err(TransferError::Busy(running));
        }
        while jobs.len() >= MAX_KEPT {
            let Some(at) = jobs
                .iter()
                .position(|job| job.borrow().snapshot.status != Status::Running)
            else {
                break;
            };
            jobs.remove(at);
        }
        jobs.push_back(receiver);
    }
    let work = work(Reporter(sender.clone()));
    tokio::spawn(async move {
        let outcome = work.await;
        sender.send_modify(|state| match outcome {
            Ok(done) => {
                state.snapshot.status = Status::Done;
                state.snapshot.bytes = done.bytes;
                state.snapshot.remote = done.remote;
                state.snapshot.local = done.local;
            }
            Err(error) => {
                state.snapshot.status = Status::Failed;
                state.failure = Some(error);
            }
        });
    });
    Ok(id)
}

pub async fn wait(id: &str, seconds: u64) -> Result<Snapshot, TransferError> {
    let mut job = registry()?
        .iter()
        .find(|job| job.borrow().snapshot.transfer_id == id)
        .cloned()
        .ok_or_else(|| TransferError::UnknownTransfer(id.to_string()))?;
    let limit = Duration::from_secs(seconds.min(MAX_WAIT_SECONDS));
    let _ = tokio::time::timeout(
        limit,
        job.wait_for(|state| state.snapshot.status != Status::Running),
    )
    .await;
    let state = job.borrow().clone();
    match state.failure {
        Some(error) => Err(error),
        None => Ok(state.snapshot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(bytes: u64) -> Transferred {
        Transferred {
            remote: "/final.bin".into(),
            local: "/tmp/final.bin".into(),
            bytes,
        }
    }

    #[tokio::test]
    async fn a_finished_transfer_reports_its_final_numbers() {
        let id = start(
            "download",
            "/a.bin",
            "/tmp/a.bin".into(),
            |report| async move {
                report.total(Some(10));
                report.bytes(4);
                Ok(done(10))
            },
        )
        .unwrap();
        let snapshot = wait(&id, 5).await.unwrap();
        assert_eq!(snapshot.status, Status::Done);
        assert_eq!((snapshot.bytes, snapshot.total), (10, Some(10)));
        assert_eq!(snapshot.remote, "/final.bin");
        assert_eq!(snapshot.direction, "download");
    }

    #[tokio::test]
    async fn a_slow_transfer_answers_running_and_can_be_waited_on_again() {
        let (release, gate) = tokio::sync::oneshot::channel::<()>();
        let id = start(
            "upload",
            "/a.bin",
            "/tmp/a.bin".into(),
            |report| async move {
                report.bytes(3);
                let _ = gate.await;
                Ok(done(9))
            },
        )
        .unwrap();
        tokio::task::yield_now().await;
        let snapshot = wait(&id, 0).await.unwrap();
        assert_eq!(snapshot.status, Status::Running);
        assert_eq!(snapshot.bytes, 3);
        release.send(()).unwrap();
        assert_eq!(wait(&id, 5).await.unwrap().status, Status::Done);
    }

    #[tokio::test]
    async fn a_failed_transfer_returns_its_error_every_time() {
        let id = start("download", "/a.bin", "/tmp/a.bin".into(), |_| async {
            Err(TransferError::NotFound("/a.bin".into()))
        })
        .unwrap();
        for _ in 0..2 {
            assert_eq!(
                wait(&id, 5).await,
                Err(TransferError::NotFound("/a.bin".into()))
            );
        }
    }

    #[tokio::test]
    async fn an_unknown_id_is_an_error() {
        assert_eq!(
            wait("nope", 0).await,
            Err(TransferError::UnknownTransfer("nope".into()))
        );
    }

    #[tokio::test]
    async fn only_a_few_transfers_run_at_once() {
        let mut gates = Vec::new();
        let mut refused = None;
        for _ in 0..=MAX_RUNNING {
            let (release, gate) = tokio::sync::oneshot::channel::<()>();
            match start("upload", "/a", "/tmp/a".into(), |_| async move {
                let _ = gate.await;
                Ok(done(1))
            }) {
                Ok(_) => gates.push(release),
                Err(error) => refused = Some(error),
            }
        }
        assert!(matches!(refused, Some(TransferError::Busy(_))));
        for release in gates {
            let _ = release.send(());
        }
    }
}
