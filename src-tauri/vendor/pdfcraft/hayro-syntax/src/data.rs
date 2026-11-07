use crate::object::ObjectIdentifier;
use crate::reader::ReaderContext;
use crate::sync::HashMap;
use crate::sync::{Arc, Mutex, MutexExt};
use crate::util::SegmentList;
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt::{Debug, Formatter};

/// A container for the bytes of a PDF file.
#[derive(Clone)]
pub struct PdfData {
    #[cfg(feature = "std")]
    inner: Arc<dyn AsRef<[u8]> + Send + Sync>,
    #[cfg(not(feature = "std"))]
    inner: Arc<dyn AsRef<[u8]>>,
}

impl Debug for PdfData {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "PdfData {{ ... }}")
    }
}

impl AsRef<[u8]> for PdfData {
    fn as_ref(&self) -> &[u8] {
        (*self.inner).as_ref()
    }
}

#[cfg(feature = "std")]
impl<T: AsRef<[u8]> + Send + Sync + 'static> From<Arc<T>> for PdfData {
    fn from(data: Arc<T>) -> Self {
        Self { inner: data }
    }
}

#[cfg(not(feature = "std"))]
impl<T: AsRef<[u8]> + 'static> From<Arc<T>> for PdfData {
    fn from(data: Arc<T>) -> Self {
        Self { inner: data }
    }
}

impl From<Vec<u8>> for PdfData {
    fn from(data: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(data),
        }
    }
}

/// A structure for storing the data of the PDF.
// To explain further: This crate uses a zero-parse approach, meaning that objects like
// dictionaries or arrays always store the underlying data and parse objects lazily as needed,
// instead of allocating the data and storing it in an owned way. However, the problem is that
// not all data is readily available in the original data of the PDF: Objects can also be
// stored in an object streams, in which case we first need to decode the stream before we can
// access the data.
//
// The purpose of `Data` is to allow us to access the original data as well as maybe decoded data
// by faking the same lifetime, so that we don't run into lifetime issues when dealing with
// PDF objects that actually stem from different data sources.
pub(crate) struct Data {
    data: PdfData,
    // 32 segments are more than enough as we can't have more objects than this.
    decoded: SegmentList<Option<Vec<u8>>, 32>,
    map: Mutex<HashMap<ObjectIdentifier, usize>>,
    loading: LoadGate,
}

impl Debug for Data {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Data {{ ... }}")
    }
}

impl Data {
    /// Create a new `Data` structure.
    pub(crate) fn new(data: PdfData) -> Self {
        Self {
            data,
            decoded: SegmentList::new(),
            map: Mutex::new(HashMap::new()),
            loading: LoadGate::default(),
        }
    }

    /// Get access to the original data of the PDF.
    pub(crate) fn get(&self) -> &PdfData {
        &self.data
    }

    /// PdfCraft patch: (hayro-syntax (33)) enter the gate for what an object stream's index depends
    /// on, once its data is read and [`Self::get_with`] has let go of it.
    pub(crate) fn enter_loading(&self, id: ObjectIdentifier) -> Option<LoadGuard<'_>> {
        self.loading.enter(id)
    }

    /// Get access to the data of a decoded object stream.
    pub(crate) fn get_with(&self, id: ObjectIdentifier, ctx: &ReaderContext<'_>) -> Option<&[u8]> {
        // PdfCraft patch: (#307, see vendor/README.md hayro-syntax (10)) register exactly once:
        // racing misses must not allocate different slots
        // for the same id (or reuse the resulting hole for another stream).
        let idx = {
            let mut map = self.map.get();
            let next = map.len();
            *map.entry(id).or_insert(next)
        };
        if let Some(decoded) = self.decoded.get(idx) {
            return decoded.as_deref();
        }
        // Only cold initialization is serialized. Nested streams may depend on
        // one another; the gate rejects cycles without blocking on our own cell.
        let _loading = self.loading.enter(id)?;
        self.decoded
            .get_or_init(idx, || {
                // PdfCraft patch: (hayro-syntax (33)) only ever an object stream.
                let stream = ctx.xref().object_stream(id, ctx)?;
                stream.decoded().ok().map(Cow::into_owned)
            })?
            .as_deref()
    }
}

/// PdfCraft patch: (#307) one cold initializer at a time prevents cross-thread dependency deadlocks.
/// The owner may recursively load different streams, up to a fixed depth. No
/// state mutex is held while parsing or decoding input. no_std is single-threaded
/// (its Arc/locks are Rc/RefCell), so it needs the same cycle/depth checks only.
#[derive(Default)]
struct LoadGate {
    state: Mutex<LoadState>,
    #[cfg(feature = "std")]
    ready: std::sync::Condvar,
}

#[derive(Default)]
struct LoadState {
    active: Vec<ObjectIdentifier>,
    #[cfg(feature = "std")]
    owner: Option<std::thread::ThreadId>,
    failed: bool,
}

impl LoadGate {
    fn enter(&self, id: ObjectIdentifier) -> Option<LoadGuard<'_>> {
        let mut state = self.state.get();
        #[cfg(feature = "std")]
        while !state.failed
            && state
                .owner
                .is_some_and(|owner| owner != std::thread::current().id())
        {
            state = self.ready.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        if state.failed || state.active.len() >= 64 || state.active.contains(&id) {
            return None;
        }
        #[cfg(feature = "std")]
        {
            state.owner = Some(std::thread::current().id());
        }
        state.active.push(id);
        Some(LoadGuard(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_loads_reject_cycles_and_limit_depth() {
        let gate = LoadGate::default();
        let mut guards = Vec::new();
        for n in 0..64 {
            guards.push(
                gate.enter(ObjectIdentifier::new(n, 0))
                    .expect("nested distinct stream"),
            );
        }
        assert!(gate.enter(ObjectIdentifier::new(64, 0)).is_none());
        assert!(gate.enter(ObjectIdentifier::new(0, 0)).is_none());
        while guards.pop().is_some() {}
        assert!(gate.enter(ObjectIdentifier::new(0, 0)).is_some());
    }

    #[cfg(feature = "std")]
    #[test]
    fn competing_dependency_chains_cannot_wait_on_each_other() {
        let gate = Arc::new(LoadGate::default());
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let jobs: Vec<_> = (0..16)
            .map(|n| {
                let gate = gate.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let (a, b) = (
                        ObjectIdentifier::new(n % 2, 0),
                        ObjectIdentifier::new(1 - n % 2, 0),
                    );
                    let _a = gate.enter(a).unwrap();
                    let _b = gate.enter(b).unwrap();
                    assert!(gate.enter(a).is_none());
                })
            })
            .collect();
        for job in jobs {
            job.join().unwrap();
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn panicking_initializer_wakes_waiters_and_prevents_retries() {
        use std::{
            panic::{AssertUnwindSafe, catch_unwind},
            sync::mpsc,
            time::Duration,
        };
        let gate = Arc::new(LoadGate::default());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let mut jobs = Vec::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = gate.enter(ObjectIdentifier::new(1, 0)).unwrap();
            for _ in 0..8 {
                let (gate, ready, done) = (gate.clone(), ready_tx.clone(), done_tx.clone());
                jobs.push(std::thread::spawn(move || {
                    ready.send(()).unwrap();
                    done.send(gate.enter(ObjectIdentifier::new(2, 0)).is_none())
                        .unwrap();
                }));
            }
            for _ in 0..8 {
                ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            }
            panic!("injected decode failure");
        }));
        assert!(result.is_err());
        for _ in 0..8 {
            assert!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap());
        }
        for job in jobs {
            job.join().unwrap();
        }
        assert!(gate.enter(ObjectIdentifier::new(1, 0)).is_none());
        assert!(gate.enter(ObjectIdentifier::new(3, 0)).is_none());
    }
}

pub(crate) struct LoadGuard<'a>(&'a LoadGate);

impl Drop for LoadGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.get();
        state.active.pop();
        #[cfg(feature = "std")]
        {
            // OnceLock itself permits a retry after a panic. The parser does not:
            // wake all waiters and fail closed until the caller replaces the Pdf.
            state.failed |= std::thread::panicking();
            if state.active.is_empty() {
                state.owner = None;
            }
            self.0.ready.notify_all();
        }
    }
}
