//! Bounded workers serialize one directory journal; tracing callers never perform file IO.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const QUEUE_RECORDS: usize = 512;
const QUEUE_BYTES: usize = 2 * 1024 * 1024;
const RECORD_BYTES: usize = 256 * 1024;
const FILE_BYTES: u64 = 8 * 1024 * 1024;
const RETAIN_FILES: usize = 7;
const RETAIN_BYTES: u64 = FILE_BYTES * RETAIN_FILES as u64;
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLogHealth {
    /// Includes in-progress formatters and the in-flight disk write, not only the channel.
    pub queued_records: u64,
    pub queued_bytes: u64,
    /// Rejected or unconfirmed records; a failed write may have written a prefix.
    pub dropped_records: u64,
    pub failed_writes: u64,
    pub closed: bool,
}

#[derive(Default)]
struct Health {
    records: AtomicUsize,
    bytes: AtomicUsize,
    dropped: AtomicU64,
    failures: AtomicU64,
    closed: AtomicBool,
    completion: Mutex<Option<bool>>,
    completed: Condvar,
}
struct WorkerCompletion {
    health: Arc<Health>,
    success: bool,
}
impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.health.failures.fetch_add(1, Ordering::Relaxed);
        }
        self.health.closed.store(true, Ordering::Release);
        *self
            .health
            .completion
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            Some(self.success && !std::thread::panicking());
        self.health.completed.notify_all();
    }
}
struct Reservation {
    health: Arc<Health>,
    bytes: usize,
    has_record: bool,
    saved: bool,
}
impl Reservation {
    fn grow(&mut self, bytes: usize) -> io::Result<()> {
        self.health
            .bytes
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                (used.saturating_add(bytes) <= QUEUE_BYTES).then_some(used + bytes)
            })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "local diagnostics byte queue full",
                )
            })?;
        self.bytes += bytes;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if self.has_record && !self.saved {
            self.health.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.health.bytes.fetch_sub(self.bytes, Ordering::Relaxed);
        self.health.records.fetch_sub(1, Ordering::Relaxed);
    }
}
struct Job {
    bytes: Vec<u8>,
    receipt: Option<mpsc::SyncSender<io::Result<()>>>,
    reservation: Reservation,
}
struct Inner {
    sender: mpsc::SyncSender<Job>,
    health: Arc<Health>,
    admission: Mutex<()>,
}
#[derive(Clone)]
pub(crate) struct LocalWriter(Arc<Inner>);

impl LocalWriter {
    pub(crate) fn new(directory: PathBuf) -> io::Result<Self> {
        let files = LogFiles::new(directory)?;
        let health = Arc::new(Health::default());
        let worker_health = health.clone();
        let (sender, receiver) = mpsc::sync_channel::<Job>(QUEUE_RECORDS);
        std::thread::Builder::new()
            .name("velune-local-logs".into())
            .spawn(move || own_disk(receiver, files, worker_health))?;
        Ok(Self(Arc::new(Inner {
            sender,
            health,
            admission: Mutex::new(()),
        })))
    }

    fn reserve(&self, bytes: usize, has_record: bool) -> io::Result<Reservation> {
        let _admission = self.0.admission.lock().unwrap_or_else(|e| e.into_inner());
        let health = &self.0.health;
        let result = (|| {
            if health.closed.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "local diagnostics writer closed",
                ));
            }
            if bytes > RECORD_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "local diagnostic record exceeds byte budget",
                ));
            }
            health
                .records
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    (used < QUEUE_RECORDS).then_some(used + 1)
                })
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "local diagnostics record queue full",
                    )
                })?;
            let mut reservation = Reservation {
                health: health.clone(),
                bytes: 0,
                has_record: false,
                saved: false,
            };
            reservation.grow(bytes)?;
            reservation.has_record = has_record;
            Ok(reservation)
        })();
        if result.is_err() && has_record {
            health.dropped.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn enqueue(
        &self,
        bytes: Vec<u8>,
        receipt: Option<mpsc::SyncSender<io::Result<()>>>,
    ) -> io::Result<()> {
        // Charge capacity, not just length: a short Vec can retain a huge allocation.
        let reservation = self.reserve(bytes.capacity(), !bytes.is_empty())?;
        self.submit(Job {
            bytes,
            receipt,
            reservation,
        })
    }

    fn submit(&self, job: Job) -> io::Result<()> {
        // Close and admission are ordered without ever holding a lock over IO.
        let _admission = self.0.admission.lock().unwrap_or_else(|e| e.into_inner());
        if self.0.health.closed.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "local diagnostics writer closed",
            ));
        }
        self.0.sender.try_send(job).map_err(|error| {
            io::Error::new(
                match error {
                    mpsc::TrySendError::Full(_) => io::ErrorKind::WouldBlock,
                    mpsc::TrySendError::Disconnected(_) => io::ErrorKind::BrokenPipe,
                },
                "local diagnostics queue unavailable",
            )
        })
    }

    /// Success acknowledges the disk owner's write and flush, not just enqueue.
    /// This does not imply fsync durability.
    /// A timed-out receipt is indeterminate and must not be automatically replayed.
    pub(crate) fn persist(&self, bytes: Vec<u8>) -> io::Result<()> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.enqueue(bytes, Some(sender))?;
        receiver.recv_timeout(RECEIPT_TIMEOUT).map_err(|error| {
            io::Error::new(
                match error {
                    mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
                    mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::BrokenPipe,
                },
                "local diagnostic persistence was not confirmed; not retried",
            )
        })?
    }
    pub(crate) fn flush(&self) -> io::Result<()> {
        self.persist(Vec::new())
    }
    pub(crate) fn close(&self) {
        let _admission = self.0.admission.lock().unwrap_or_else(|e| e.into_inner());
        self.0.health.closed.store(true, Ordering::Release);
    }
    /// Wait for the existing owner to drain and close; a stalled disk does not
    /// authorize a replacement worker or make an unconfirmed write replayable.
    pub(crate) fn shutdown(&self) -> io::Result<()> {
        self.close();
        let health = &self.0.health;
        let completion = health.completion.lock().unwrap_or_else(|e| e.into_inner());
        let (completion, _) = health
            .completed
            .wait_timeout_while(completion, RECEIPT_TIMEOUT, |completed| completed.is_none())
            .unwrap_or_else(|e| e.into_inner());
        match *completion {
            Some(true) => Ok(()),
            Some(false) => Err(io::Error::other("local diagnostic disk owner failed")),
            None => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "local diagnostic shutdown not confirmed; owner retained",
            )),
        }
    }
    pub(crate) fn health(&self) -> LocalLogHealth {
        let h = &self.0.health;
        LocalLogHealth {
            queued_records: h.records.load(Ordering::Relaxed) as u64,
            queued_bytes: h.bytes.load(Ordering::Relaxed) as u64,
            dropped_records: h.dropped.load(Ordering::Relaxed),
            failed_writes: h.failures.load(Ordering::Relaxed),
            closed: h.closed.load(Ordering::Acquire),
        }
    }
    pub(crate) fn record(&self) -> Record {
        Record {
            writer: self.clone(),
            bytes: Vec::new(),
            reservation: None,
            rejected: false,
        }
    }
}

/// The formatter can issue several writes; queue only complete records, never fragments.
pub(crate) struct Record {
    writer: LocalWriter,
    bytes: Vec<u8>,
    reservation: Option<Reservation>,
    rejected: bool,
}
impl Write for Record {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() || self.rejected {
            return Ok(bytes.len());
        }
        if self.reservation.is_none() {
            match self.writer.reserve(0, true) {
                Ok(reservation) => self.reservation = Some(reservation),
                Err(_) => {
                    self.rejected = true;
                    return Ok(bytes.len());
                }
            }
        }
        let length = self.bytes.len().saturating_add(bytes.len());
        let additional = length.saturating_sub(self.bytes.capacity());
        if self.writer.0.health.closed.load(Ordering::Acquire)
            || length > RECORD_BYTES
            || self.reservation.as_mut().unwrap().grow(additional).is_err()
        {
            self.rejected = true;
            self.bytes = Vec::new();
            self.reservation.take();
        } else {
            self.bytes.reserve_exact(bytes.len());
            // reserve_exact may still return more capacity than requested.
            // Charge retained capacity before publishing or retaining the record;
            // allocation itself can transiently exceed the reservation budget.
            let excess = self
                .bytes
                .capacity()
                .saturating_sub(self.reservation.as_ref().unwrap().bytes);
            if self.bytes.capacity() > RECORD_BYTES
                || self.reservation.as_mut().unwrap().grow(excess).is_err()
            {
                self.rejected = true;
                self.bytes = Vec::new();
                self.reservation.take();
            } else {
                self.bytes.extend_from_slice(bytes);
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for Record {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return; // Do not publish a formatter's unwound, potentially partial record.
        }
        if let Some(reservation) = self.reservation.take() {
            let _ = self.writer.submit(Job {
                bytes: std::mem::take(&mut self.bytes),
                receipt: None,
                reservation,
            });
        }
    }
}

fn own_disk(receiver: mpsc::Receiver<Job>, files: LogFiles, health: Arc<Health>) {
    // The inner call owns all disk/queue resources. They are destroyed on return
    // or unwind before this outer guard publishes completion.
    let mut completion = WorkerCompletion {
        health: health.clone(),
        success: false,
    };
    completion.success = run_owner(receiver, files, &health);
}

fn run_owner(receiver: mpsc::Receiver<Job>, mut files: LogFiles, health: &Health) -> bool {
    loop {
        let mut job = if health.closed.load(Ordering::Acquire) {
            match receiver.try_recv() {
                Ok(job) => job,
                Err(_) => break,
            }
        } else {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(job) => job,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        let result = files.append(&job.bytes, job.receipt.is_some());
        let failed = result.is_err();
        job.reservation.saved = !failed;
        if failed {
            health.failures.fetch_add(1, Ordering::Relaxed);
            health.closed.store(true, Ordering::Release);
        }
        if let Some(receipt) = job.receipt.take() {
            let _ = receipt.send(result);
        }
        if failed {
            // The failed record and queued records are counted by their reservations.
            // No receipt (including one whose caller timed out) authorizes replay.
            return false;
        }
    }
    if files.flush().is_err() {
        health.failures.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    true
}

// All cooperating readers and appenders use this stable inode. Never unlink it:
// holding it covers selection, prefix retention, append/flush, and data-handle drop.
pub(crate) fn lock_directory(directory: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(directory.join(".velune-log.lock"))?;
    lock.lock()?;
    Ok(lock)
}

/// Call only while holding the directory lock, including while reading the files.
pub(crate) fn ordered_logs(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_file() && name.starts_with("velune.") && name.ends_with(".jsonl") {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

// Workers own only the directory, never a long-lived shard. The directory lock
// defines one physical journal order across workers and cooperating processes.
// Old non-cooperating writers and historical interleaving cannot be reordered;
// existing shards keep their filename replay order until normal prefix retention.
struct LogFiles {
    directory: PathBuf,
}
impl LogFiles {
    fn new(directory: PathBuf) -> io::Result<Self> {
        let _lock = lock_directory(&directory)?;
        ordered_logs(&directory)?;
        Ok(Self { directory })
    }

    fn next_path(
        files: &[PathBuf],
        directory: &Path,
        now: time::OffsetDateTime,
    ) -> io::Result<PathBuf> {
        let last = files
            .last()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str());
        let sequence = match last.and_then(|name| name.strip_prefix("velune.~")) {
            Some(suffix) => suffix
                .split('.')
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "local diagnostic sequence exhausted or invalid",
                    )
                })?,
            None => 0,
        };
        // '~' sorts after known legacy date-only and date/time/random filenames.
        // The counter, not the wall clock, orders all newly created shards.
        let name = format!("velune.~{sequence:020}.{}.jsonl", now.date());
        let path = directory.join(name);
        if files.last().is_some_and(|last| path <= *last) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot order local diagnostic shard after unknown filename",
            ));
        }
        Ok(path)
    }

    fn append(&mut self, bytes: &[u8], flush: bool) -> io::Result<()> {
        let _lock = lock_directory(&self.directory)?;
        let files = ordered_logs(&self.directory)?;
        if bytes.is_empty() {
            if flush && let Some(path) = files.last() {
                OpenOptions::new().append(true).open(path)?.flush()?;
            }
            return Ok(());
        }
        let now = time::OffsetDateTime::now_utc();
        let mut file = match files.last() {
            Some(path) => Some(OpenOptions::new().read(true).append(true).open(path)?),
            None => None,
        };
        let length = file
            .as_ref()
            .map(|file| file.metadata().map(|m| m.len()))
            .transpose()?
            .unwrap_or(0);
        // An interrupted tail is preserved but poisoned and terminated, so even
        // otherwise valid JSON without its commit newline cannot become a journal
        // event later, or swallow the next successful journal event.
        let fence = if length != 0 {
            let file = file.as_mut().unwrap();
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            file.read_exact(&mut last)?;
            if last[0] == b'\n' { 0 } else { 2 }
        } else {
            0
        };
        let current_day = files.last().is_some_and(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .ends_with(&format!(".{}.jsonl", now.date()))
                    && name.to_string_lossy().starts_with("velune.~")
            })
        });
        let rotate = !current_day || length.saturating_add(fence + bytes.len() as u64) > FILE_BYTES;
        if rotate {
            let next = Self::next_path(&files, &self.directory, now)?;
            // Close before pruning, including on Windows. Only remove a complete
            // oldest prefix: a cleanup can never disappear while its source stays.
            drop(file.take());
            Self::retain(&files, 1, FILE_BYTES)?;
            let mut options = OpenOptions::new();
            options.read(true).append(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            file = Some(options.open(&next)?);
        } else {
            Self::retain(&files, 0, fence + bytes.len() as u64)?;
        }
        let data = file.as_mut().unwrap();
        if !rotate && fence != 0 {
            data.write_all(b"\0\n")?;
        }
        data.write_all(bytes)?;
        if flush {
            data.flush()?;
        }
        drop(file); // Close the data handle before releasing the directory lock.
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.append(&[], true)
    }

    // Reserve either a new full shard or the pending append. Delete only the
    // oldest prefix, never skip a shard. A reused latest handle is never pruned.
    fn retain(files: &[PathBuf], extra_files: usize, extra_bytes: u64) -> io::Result<()> {
        let mut sizes = Vec::with_capacity(files.len());
        let mut total = extra_bytes;
        for path in files {
            let size = std::fs::metadata(path)?.len();
            total = total.saturating_add(size);
            sizes.push(size);
        }
        let mut count = files.len() + extra_files;
        for (path, size) in files.iter().zip(sizes) {
            if count <= RETAIN_FILES && total <= RETAIN_BYTES {
                return Ok(());
            }
            std::fs::remove_file(path)?;
            count -= 1;
            total = total.saturating_sub(size);
        }
        if count <= RETAIN_FILES && total <= RETAIN_BYTES {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local diagnostic append exceeds file budget",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_paths(directory: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl")
            })
            .collect()
    }

    fn pending_writer() -> (LocalWriter, mpsc::Receiver<Job>) {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_RECORDS);
        (
            LocalWriter(Arc::new(Inner {
                sender,
                health: Arc::new(Health::default()),
                admission: Mutex::new(()),
            })),
            receiver,
        )
    }

    #[test]
    fn complete_records_and_persistence_barrier_share_one_bounded_owner() {
        let home = tempfile::tempdir().unwrap();
        let writer = LocalWriter::new(home.path().into()).unwrap();
        let mut record = writer.record();
        record.write_all(b"{\"a\":").unwrap();
        record.write_all(b"1}\n").unwrap();
        drop(record);
        writer.persist(b"{\"journal\":true}\n".to_vec()).unwrap();
        let path = log_paths(home.path()).pop().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"a\":1}\n{\"journal\":true}\n"
        );
        assert!(writer.persist(vec![0; RECORD_BYTES + 1]).is_err());
        assert_eq!(writer.health().dropped_records, 1);
        writer.record().write_all(b"{\"closing\":true}\n").unwrap();
        writer.shutdown().unwrap();
        writer.shutdown().unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "{\"a\":1}\n{\"journal\":true}\n{\"closing\":true}\n"
        );
        assert_eq!(writer.health().queued_records, 0);
        assert_eq!(writer.health().queued_bytes, 0);
        assert!(writer.persist(Vec::new()).is_err());
        assert_eq!(writer.health().dropped_records, 1);
        assert!(writer.persist(b"late\n".to_vec()).is_err());
        assert_eq!(writer.health().dropped_records, 2);
    }

    #[test]
    fn producer_allocations_share_the_queue_budget_and_rejections_count_once() {
        let (writer, receiver) = pending_writer();
        let payload = vec![b'x'; RECORD_BYTES];
        let mut records = Vec::new();
        for _ in 0..QUEUE_BYTES / RECORD_BYTES {
            let mut record = writer.record();
            record.write_all(&payload[..1]).unwrap();
            record.write_all(&payload[1..]).unwrap();
            records.push(record);
        }
        assert_eq!(writer.health().queued_bytes, QUEUE_BYTES as u64);
        let mut rejected = writer.record();
        rejected.write_all(b"full").unwrap();
        rejected.write_all(b"still full").unwrap();
        assert!(rejected.bytes.is_empty());
        assert_eq!(writer.health().dropped_records, 1);
        writer.close();
        drop(records);
        assert_eq!(writer.health().queued_bytes, 0);
        assert_eq!(writer.health().queued_records, 0);
        assert_eq!(
            writer.health().dropped_records,
            1 + (QUEUE_BYTES / RECORD_BYTES) as u64
        );
        drop(receiver);

        let (writer, receiver) = pending_writer();
        let mut records = Vec::new();
        for _ in 0..QUEUE_RECORDS {
            let mut record = writer.record();
            record.write_all(b"x").unwrap();
            records.push(record);
        }
        writer.record().write_all(b"full").unwrap();
        assert_eq!(writer.health().queued_records, QUEUE_RECORDS as u64);
        assert_eq!(writer.health().dropped_records, 1);
        drop(records);
        drop(receiver);
        assert_eq!(writer.health().queued_bytes, 0);
        assert_eq!(writer.health().queued_records, 0);
        assert_eq!(writer.health().dropped_records, 1 + QUEUE_RECORDS as u64);

        let (writer, _receiver) = pending_writer();
        let mut oversized = writer.record();
        oversized.write_all(b"prefix").unwrap();
        oversized.write_all(&payload).unwrap();
        oversized.write_all(b"suffix").unwrap();
        assert_eq!(oversized.bytes.capacity(), 0);
        assert_eq!(writer.health().queued_bytes, 0);
        assert_eq!(writer.health().dropped_records, 1);
        let mut excessive_capacity = Vec::with_capacity(RECORD_BYTES + 1);
        excessive_capacity.push(b'x');
        assert_eq!(
            writer.persist(excessive_capacity).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(writer.health().dropped_records, 2);
        assert!(
            std::panic::catch_unwind(|| {
                let mut record = writer.record();
                record.write_all(b"partial").unwrap();
                panic!("synthetic formatter unwind");
            })
            .is_err()
        );
        assert_eq!(writer.health().dropped_records, 3);
        assert_eq!(writer.health().queued_records, 0);
    }

    #[test]
    fn failed_and_panicking_owners_release_resources_before_completion() {
        fn unwind_owner(receiver: mpsc::Receiver<Job>, files: LogFiles) {
            let _receiver = receiver;
            let _lock = lock_directory(&files.directory).unwrap();
            panic!("synthetic owner unwind while holding the directory lock");
        }
        for panic in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let directory = home.path().join("logs");
            std::fs::create_dir(&directory).unwrap();
            let files = LogFiles::new(directory.clone()).unwrap();
            if !panic {
                // A real missing directory makes the next append fail.
                std::fs::remove_file(directory.join(".velune-log.lock")).unwrap();
                std::fs::remove_dir(&directory).unwrap();
            }
            let (writer, receiver) = pending_writer();
            let (receipt, confirmation) = mpsc::sync_channel(1);
            writer.enqueue(b"first\n".to_vec(), Some(receipt)).unwrap();
            writer.enqueue(b"second\n".to_vec(), None).unwrap();
            let health = writer.0.health.clone();
            let owner = std::thread::spawn(move || {
                if panic {
                    let _completion = WorkerCompletion {
                        health,
                        success: false,
                    };
                    unwind_owner(receiver, files);
                } else {
                    own_disk(receiver, files, health);
                }
            });
            assert!(writer.shutdown().is_err());
            assert_eq!(writer.health().queued_records, 0);
            assert_eq!(writer.health().queued_bytes, 0);
            assert_eq!(writer.health().dropped_records, 2);
            assert_eq!(writer.health().failed_writes, 1);
            assert!(writer.health().closed);
            assert_eq!(*writer.0.health.completion.lock().unwrap(), Some(false));
            if panic {
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(directory.join(".velune-log.lock"))
                    .unwrap();
                lock.try_lock().unwrap();
            }
            assert!(confirmation.recv().map_or(true, |result| result.is_err()));
            assert_eq!(owner.join().is_err(), panic);
        }
    }

    #[test]
    fn timed_out_persistence_is_written_once_without_replay() {
        let home = tempfile::tempdir().unwrap();
        let files = LogFiles::new(home.path().into()).unwrap();
        let (writer, receiver) = pending_writer();
        assert_eq!(
            writer.persist(b"once\n".to_vec()).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(writer.health().queued_records, 1);
        writer.close();
        own_disk(receiver, files, writer.0.health.clone());
        writer.shutdown().unwrap();
        assert_eq!(writer.health().dropped_records, 0);
        assert_eq!(
            std::fs::read(log_paths(home.path()).pop().unwrap()).unwrap(),
            b"once\n"
        );
    }

    #[test]
    fn rotation_and_retention_share_one_sequence_across_threads() {
        use std::sync::Barrier;
        let home = tempfile::tempdir().unwrap();
        let anchor = LocalWriter::new(home.path().into()).unwrap();
        anchor.persist(b"anchor\n".to_vec()).unwrap();
        let original = log_paths(home.path()).pop().unwrap();
        // More workers than shards are allowed: none owns a persistent data handle.
        let writers: Vec<_> = (0..RETAIN_FILES + 1)
            .map(|_| LogFiles::new(home.path().into()).unwrap())
            .collect();
        assert_eq!(log_paths(home.path()).len(), 1);
        let start = Arc::new(Barrier::new(writers.len()));
        let owners: Vec<_> = writers
            .into_iter()
            .map(|mut files| {
                let start = start.clone();
                std::thread::spawn(move || {
                    let mut payload = vec![b't'; RECORD_BYTES];
                    *payload.last_mut().unwrap() = b'\n';
                    start.wait();
                    for _ in 0..(FILE_BYTES as usize / RECORD_BYTES + 1) {
                        files.append(&payload, false).unwrap();
                    }
                    files.flush().unwrap();
                })
            })
            .collect();
        for owner in owners {
            owner.join().unwrap();
        }
        anchor.persist(b"still live\n".to_vec()).unwrap();
        anchor.shutdown().unwrap();
        let paths = log_paths(home.path());
        assert!(!original.exists());
        assert!(paths.len() <= RETAIN_FILES);
        assert!(
            paths
                .iter()
                .map(|path| std::fs::metadata(path).unwrap().len())
                .sum::<u64>()
                <= RETAIN_BYTES
        );
        assert!(
            paths
                .iter()
                .all(|path| std::fs::metadata(path).unwrap().len() <= FILE_BYTES)
        );
        let mut ordered = paths;
        ordered.sort();
        assert!(
            std::fs::read(ordered.last().unwrap())
                .unwrap()
                .ends_with(b"still live\n")
        );
        assert!(std::fs::read_dir(home.path()).unwrap().count() <= RETAIN_FILES + 1);
    }

    #[test]
    fn shard_sequence_survives_clock_rollback_and_partial_tails_are_fenced() {
        let home = tempfile::tempdir().unwrap();
        let future = home
            .path()
            .join("velune.~00000000000000000009.9999-12-31.jsonl");
        std::fs::write(&future, b"old\n").unwrap();
        let writer = LocalWriter::new(home.path().into()).unwrap();
        writer.persist(b"first\n".to_vec()).unwrap();
        let mut paths = log_paths(home.path());
        paths.sort();
        let latest = paths.last().unwrap();
        assert!(latest > &future);
        assert!(
            latest
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("velune.~00000000000000000010.")
        );
        {
            let _lock = lock_directory(home.path()).unwrap();
            OpenOptions::new()
                .append(true)
                .open(latest)
                .unwrap()
                .write_all(b"{\"partial\":true}")
                .unwrap();
        }
        writer.persist(b"{\"confirmed\":true}\n".to_vec()).unwrap();
        writer.shutdown().unwrap();
        assert_eq!(
            std::fs::read(latest).unwrap(),
            b"first\n{\"partial\":true}\0\n{\"confirmed\":true}\n"
        );
    }
}
