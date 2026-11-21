use super::*;

/// Only actual protocol replies and explicitly selected cancellation traffic
/// may use the reserve. A method name is never a classification rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteClass {
    Ordinary,
    ProtocolReplyCancel,
}

/// Engineering policy for facades without an upstream allocation lease.
/// Per-image input is capped at 20 MiB; 128 MiB accommodates base64/JSON
/// expansion and 9 MiB text, but is NOT a promise of unlimited image counts.
pub(crate) const LEGACY_ALLOCATION_LIMIT: usize = 128 * 1024 * 1024;

struct ActualLifetime {
    _owner: Arc<dyn Send + Sync>,
}
impl std::fmt::Debug for ActualLifetime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("controlled IO lifetime")
    }
}
#[derive(Debug)]
pub(crate) struct OwnedFrame {
    pub(crate) bytes: Vec<u8>,
    /// Finite upstream encoded-allocation budget, including framing/capacity.
    /// This must come from caller policy, not from the current frame's size.
    pub(crate) allocation_limit: usize,
    lifetime: Option<ActualLifetime>,
}
impl OwnedFrame {
    pub(crate) fn new(bytes: Vec<u8>, allocation_limit: usize) -> Self {
        Self {
            bytes,
            allocation_limit,
            lifetime: None,
        }
    }
    pub(crate) fn retain_lifetime(&mut self, owner: Arc<dyn Send + Sync>) {
        if self.lifetime.is_none() {
            self.lifetime = Some(ActualLifetime { _owner: owner });
        }
    }
    pub(crate) fn legacy(bytes: Vec<u8>) -> Self {
        Self::new(bytes, LEGACY_ALLOCATION_LIMIT)
    }
}

/// Streaming JSON encoder: growth is checked before allocation or copying,
/// including the trailing newline. Caller-created Values/base64 are outside
/// this codec's ownership and must have their own upstream budgets.
struct BoundedEncoder {
    bytes: Vec<u8>,
    limit: usize,
    overflow: io::ErrorKind,
}
impl Write for BoundedEncoder {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= self.limit)
            .ok_or_else(|| {
                io::Error::new(
                    self.overflow,
                    "runtime encoded allocation budget exceeded before dispatch",
                )
            })?;
        if length > self.bytes.capacity() {
            let capacity = length
                .max(self.bytes.capacity().saturating_mul(2))
                .max(1024)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        "runtime encoded allocation unavailable",
                    )
                })?;
            if self.bytes.capacity() > self.limit {
                return Err(io::Error::new(
                    self.overflow,
                    "runtime encoded capacity exceeds finite budget",
                ));
            }
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl OwnedFrame {
    /// Failure leaves the borrowed source untouched; no complete encoded frame
    /// exists on codec failure. Only try_enqueue can dispatch an owned frame.
    fn encode_json(
        value: &impl serde::Serialize,
        allocation_limit: usize,
        encoding_limit: usize,
        overflow: io::ErrorKind,
    ) -> io::Result<Self> {
        let mut encoder = BoundedEncoder {
            bytes: Vec::new(),
            limit: encoding_limit,
            overflow,
        };
        serde_json::to_writer(&mut encoder, value).map_err(|error| {
            io::Error::new(
                error.io_error_kind().unwrap_or(io::ErrorKind::InvalidInput),
                "runtime JSON encoding failed before dispatch",
            )
        })?;
        encoder.write_all(b"\n")?;
        Ok(Self::new(encoder.bytes, allocation_limit))
    }
}

/// No byte has been dispatched. All retry inputs retain their original identity.
#[derive(Debug)]
pub(crate) struct NotAdmitted {
    pub(crate) frame: OwnedFrame,
    pub(crate) class: WriteClass,
    pub(crate) deadline: Instant,
    pub(crate) error: io::Error,
}
impl NotAdmitted {
    pub(crate) fn new(
        frame: OwnedFrame,
        class: WriteClass,
        deadline: Instant,
        error: io::Error,
    ) -> Self {
        Self {
            frame,
            class,
            deadline,
            error,
        }
    }
    pub(crate) fn into_error(self) -> io::Error {
        let Self {
            frame,
            class,
            deadline,
            error,
        } = self;
        drop((frame, class, deadline));
        error
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WriteLimits {
    pub(crate) ordinary_count: usize,
    pub(crate) ordinary_bytes: usize,
    pub(crate) control_count: usize,
    pub(crate) control_bytes: usize,
    pub(crate) total_count: usize,
    pub(crate) oversized_allocation: usize,
}
impl Default for WriteLimits {
    fn default() -> Self {
        Self {
            ordinary_count: 12,
            ordinary_bytes: 8 * 1024 * 1024,
            control_count: 4,
            control_bytes: 1024 * 1024,
            total_count: 16,
            oversized_allocation: LEGACY_ALLOCATION_LIMIT,
        }
    }
}
#[derive(Default, Debug)]
pub(super) struct Usage {
    failed: bool,
    pub(super) ordinary_count: usize,
    pub(super) ordinary_bytes: usize,
    pub(super) control_count: usize,
    pub(super) control_bytes: usize,
}
struct Reservation {
    usage: Arc<Mutex<Usage>>,
    class: WriteClass,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut usage = self.usage.lock().expect("pipe admission reservation");
        match self.class {
            WriteClass::Ordinary => {
                usage.ordinary_count -= 1;
                usage.ordinary_bytes -= self.bytes;
            }
            WriteClass::ProtocolReplyCancel => {
                usage.control_count -= 1;
                usage.control_bytes -= self.bytes;
            }
        }
    }
}
struct WriteJob {
    frame: OwnedFrame,
    deadline: Instant,
    completion: mpsc::SyncSender<WriteOutcome>,
    reservation: Reservation,
    // Separate from frame: survives actual frame/reservation disposal AND the
    // physical result publication, even if the receipt owner disappeared.
    lifetime: Option<ActualLifetime>,
}

impl WriteOutcome {
    /// Keep the original physical fact in the receipt. Returned observations
    /// preserve error kind, OS code (when present), and diagnostic reason.
    fn snapshot(&self) -> Self {
        let copy_error = |error: &io::Error| match error.raw_os_error() {
            Some(code) => io::Error::from_raw_os_error(code),
            None => io::Error::new(error.kind(), error.to_string()),
        };
        match self {
            Self::Written(completion) => Self::Written(*completion),
            Self::NotWritten(error) => Self::NotWritten(copy_error(error)),
            Self::Indeterminate(error) => Self::Indeterminate(copy_error(error)),
            Self::ObservationTimeout {
                deadline,
                observed_at,
            } => Self::ObservationTimeout {
                deadline: *deadline,
                observed_at: *observed_at,
            },
        }
    }

    fn into_result(self) -> io::Result<()> {
        match self {
            Self::Written(completion) if completion.met_deadline() => Ok(()),
            Self::Written(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime write flushed after deadline",
            )),
            Self::ObservationTimeout {
                deadline,
                observed_at,
            } => {
                debug_assert!(observed_at >= deadline);
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "runtime write outcome not yet observed",
                ))
            }
            Self::NotWritten(error) | Self::Indeterminate(error) => Err(error),
        }
    }
}

/// Admission is not delivery. Only actual write + flush proves Written.
/// Receipt lifetime/observation never owns or releases the WriteJob lease.
pub(crate) struct WriteReceipt {
    result: mpsc::Receiver<WriteOutcome>,
    deadline: Instant,
    /// Physical terminal facts are immutable once harvested. Observation
    /// timeouts are not terminal and must never occupy this cache.
    terminal: Mutex<Option<WriteOutcome>>,
}
impl WriteReceipt {
    #[cfg(test)]
    pub(crate) fn test_pending(deadline: Instant) -> (mpsc::SyncSender<WriteOutcome>, Self) {
        let (sender, result) = mpsc::sync_channel(1);
        (
            sender,
            Self {
                result,
                deadline,
                terminal: Mutex::new(None),
            },
        )
    }
    pub(crate) fn try_outcome(&self) -> Option<WriteOutcome> {
        let mut terminal = self.terminal.lock().expect("pipe receipt outcome");
        if let Some(outcome) = terminal.as_ref() {
            return Some(outcome.snapshot());
        }
        let outcome = match self.result.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Disconnected) => WriteOutcome::Indeterminate(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "runtime write receipt disconnected",
            )),
            Err(mpsc::TryRecvError::Empty) if Instant::now() >= self.deadline => {
                return Some(WriteOutcome::ObservationTimeout {
                    deadline: self.deadline,
                    observed_at: Instant::now(),
                });
            }
            Err(mpsc::TryRecvError::Empty) => return None,
        };
        let snapshot = outcome.snapshot();
        *terminal = Some(outcome);
        Some(snapshot)
    }
    // Production owners use timestamped outcomes; regression tests retain
    // coverage of the former polling facade's mapping.
    #[cfg(test)]
    pub(crate) fn try_complete(&self) -> Option<io::Result<()>> {
        self.try_outcome().map(WriteOutcome::into_result)
    }
    pub(crate) fn wait(self) -> io::Result<()> {
        if let Some(outcome) = self.terminal.into_inner().expect("pipe receipt outcome") {
            return outcome.into_result();
        }
        self.result
            .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| {
                io::Error::new(
                    match error {
                        mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
                        mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::BrokenPipe,
                    },
                    "runtime write did not complete before deadline",
                )
            })?
            .into_result()
    }
}

pub(crate) struct PipeWriter {
    sender: mpsc::SyncSender<WriteJob>,
    pub(super) usage: Arc<Mutex<Usage>>,
    limits: WriteLimits,
}
impl PipeWriter {
    pub(crate) fn new(pipe: ChildStdin) -> io::Result<Self> {
        Self::with_limits(pipe, WriteLimits::default())
    }
    pub(crate) fn with_limits(mut pipe: ChildStdin, limits: WriteLimits) -> io::Result<Self> {
        if limits
            .ordinary_count
            .checked_add(limits.control_count)
            .is_none_or(|sum| sum > limits.total_count)
            || limits.total_count == 0
            || limits.ordinary_count == 0
            || limits.control_count == 0
            || limits.ordinary_bytes == 0
            || limits.control_bytes == 0
            || limits.oversized_allocation == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime write limits must be finite and positive",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel::<WriteJob>(limits.total_count);
        thread::Builder::new()
            .name("velune-runtime-writer".into())
            .spawn(move || {
                let mut failed = false;
                for job in &receiver {
                    let started_at = Instant::now();
                    let outcome = if failed {
                        WriteOutcome::NotWritten(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "runtime writer failed before dispatch",
                        ))
                    } else if started_at >= job.deadline {
                        // Expiring one queued frame does not corrupt the stream.
                        WriteOutcome::NotWritten(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "runtime write expired in queue",
                        ))
                    } else {
                        match pipe.write_all(&job.frame.bytes).and_then(|()| pipe.flush()) {
                            Ok(()) => WriteOutcome::Written(WriteCompletion {
                                started_at,
                                completed_at: Instant::now(),
                                deadline: job.deadline,
                            }),
                            Err(error) => {
                                failed = true;
                                job.reservation
                                    .usage
                                    .lock()
                                    .expect("pipe admission reservation")
                                    .failed = true;
                                WriteOutcome::Indeterminate(error)
                            }
                        }
                    };
                    // Actual encoded allocation is gone before returning its lease.
                    drop(job.frame);
                    drop(job.reservation);
                    let _ = job.completion.send(outcome);
                    if failed {
                        // Disconnect admission and settle every already queued job.
                        while let Ok(job) = receiver.try_recv() {
                            drop(job.frame);
                            drop(job.reservation);
                            let _ = job.completion.send(WriteOutcome::NotWritten(io::Error::new(
                                io::ErrorKind::BrokenPipe,
                                "runtime writer failed before dispatch",
                            )));
                        }
                        break;
                    }
                }
            })?;
        Ok(Self {
            sender,
            usage: Arc::new(Mutex::new(Usage::default())),
            limits,
        })
    }

    pub(crate) fn write(&self, bytes: Vec<u8>, deadline: Instant) -> io::Result<()> {
        self.enqueue(bytes, deadline)?.wait()
    }
    pub(crate) fn enqueue(&self, bytes: Vec<u8>, deadline: Instant) -> io::Result<WriteReceipt> {
        self.try_enqueue(OwnedFrame::legacy(bytes), WriteClass::Ordinary, deadline)
            .map_err(NotAdmitted::into_error)
    }

    /// A non-reserving preflight. Final admission remains atomic and may
    /// refuse after another producer wins the race; the encoded frame is then
    /// returned unchanged by try_enqueue, never hidden in a second queue.
    fn encoding_limit(
        &self,
        class: WriteClass,
        allocation_limit: usize,
        deadline: Instant,
    ) -> io::Result<usize> {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime write deadline",
            ));
        }
        if allocation_limit == 0 || allocation_limit > self.limits.oversized_allocation {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime finite allocation policy exceeds writer maximum",
            ));
        }
        let usage = self.usage.lock().expect("pipe admission reservation");
        if usage.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "runtime writer failed",
            ));
        }
        let remaining = if usage.ordinary_count + usage.control_count >= self.limits.total_count {
            0
        } else {
            match class {
                WriteClass::Ordinary if usage.ordinary_count >= self.limits.ordinary_count => 0,
                WriteClass::Ordinary if usage.ordinary_count == 0 => self
                    .limits
                    .oversized_allocation
                    .max(self.limits.ordinary_bytes),
                WriteClass::Ordinary => self
                    .limits
                    .ordinary_bytes
                    .saturating_sub(usage.ordinary_bytes),
                WriteClass::ProtocolReplyCancel
                    if usage.control_count >= self.limits.control_count =>
                {
                    0
                }
                WriteClass::ProtocolReplyCancel => self
                    .limits
                    .control_bytes
                    .saturating_sub(usage.control_bytes),
            }
        };
        if remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "runtime write count/byte budget full before encoding",
            ));
        }
        Ok(allocation_limit.min(remaining))
    }

    pub(crate) fn encode_json(
        &self,
        value: &impl serde::Serialize,
        class: WriteClass,
        allocation_limit: usize,
        deadline: Instant,
    ) -> io::Result<OwnedFrame> {
        let limit = self.encoding_limit(class, allocation_limit, deadline)?;
        OwnedFrame::encode_json(
            value,
            allocation_limit,
            limit,
            if limit < allocation_limit {
                io::ErrorKind::WouldBlock
            } else {
                io::ErrorKind::InvalidInput
            },
        )
    }

    /// One atomic admission point for queued AND in-flight allocation/count.
    /// Sending while locked preserves the same FIFO order as reservation.
    #[allow(clippy::result_large_err)] // Rejection deliberately returns ownership, not a copied payload.
    pub(crate) fn try_enqueue(
        &self,
        frame: OwnedFrame,
        class: WriteClass,
        deadline: Instant,
    ) -> Result<WriteReceipt, NotAdmitted> {
        let reject = |frame, kind, message| {
            NotAdmitted::new(frame, class, deadline, io::Error::new(kind, message))
        };
        if Instant::now() >= deadline {
            return Err(reject(
                frame,
                io::ErrorKind::TimedOut,
                "runtime write deadline",
            ));
        }
        let bytes = frame.bytes.capacity();
        if frame.allocation_limit == 0
            || bytes > frame.allocation_limit
            || frame.allocation_limit > self.limits.oversized_allocation
        {
            return Err(reject(
                frame,
                io::ErrorKind::InvalidInput,
                "runtime encoded allocation exceeds finite budget",
            ));
        }
        let mut usage = self.usage.lock().expect("pipe admission reservation");
        if usage.failed {
            return Err(reject(
                frame,
                io::ErrorKind::BrokenPipe,
                "runtime writer failed",
            ));
        }
        let total = usage.ordinary_count.saturating_add(usage.control_count);
        let admitted = total < self.limits.total_count
            && match class {
                WriteClass::Ordinary => {
                    usage.ordinary_count < self.limits.ordinary_count
                        && if bytes > self.limits.ordinary_bytes {
                            usage.ordinary_count == 0
                        } else {
                            usage.ordinary_bytes <= self.limits.ordinary_bytes
                                && bytes
                                    <= self
                                        .limits
                                        .ordinary_bytes
                                        .saturating_sub(usage.ordinary_bytes)
                        }
                }
                WriteClass::ProtocolReplyCancel => {
                    usage.control_count < self.limits.control_count
                        && bytes
                            <= self
                                .limits
                                .control_bytes
                                .saturating_sub(usage.control_bytes)
                }
            };
        if !admitted {
            return Err(reject(
                frame,
                io::ErrorKind::WouldBlock,
                "runtime write count/byte budget full",
            ));
        }
        match class {
            WriteClass::Ordinary => {
                usage.ordinary_count += 1;
                usage.ordinary_bytes += bytes;
            }
            WriteClass::ProtocolReplyCancel => {
                usage.control_count += 1;
                usage.control_bytes += bytes;
            }
        }
        let (completion, receipt) = mpsc::sync_channel(1);
        let mut frame = frame;
        let lifetime = frame.lifetime.take();
        let job = WriteJob {
            frame,
            deadline,
            completion,
            reservation: Reservation {
                usage: self.usage.clone(),
                class,
                bytes,
            },
            lifetime,
        };
        let sent = self.sender.try_send(job);
        drop(usage);
        if let Err(error) = sent {
            let (mut job, kind) = match error {
                mpsc::TrySendError::Full(job) => (job, io::ErrorKind::WouldBlock),
                mpsc::TrySendError::Disconnected(job) => (job, io::ErrorKind::BrokenPipe),
            };
            drop(job.reservation);
            job.frame.lifetime = job.lifetime.take();
            return Err(reject(job.frame, kind, "runtime writer unavailable"));
        }
        Ok(WriteReceipt {
            result: receipt,
            deadline,
            terminal: Mutex::new(None),
        })
    }
}

#[cfg(test)]
mod codec_tests {
    use super::*;

    #[test]
    fn bounded_encoder_refuses_large_fragment_before_copy_and_includes_newline() {
        let text = "x".repeat(1024 * 1024);
        let mut encoder = BoundedEncoder {
            bytes: Vec::new(),
            limit: 1024,
            overflow: io::ErrorKind::InvalidInput,
        };
        encoder.write_all(b"prefix").unwrap();
        assert_eq!(
            encoder.write_all(text.as_bytes()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(encoder.bytes.capacity() <= 1024);
        assert_eq!(encoder.bytes, b"prefix");
        assert_eq!(
            OwnedFrame::encode_json(&text, 1024, 1024, io::ErrorKind::InvalidInput)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        // JSON string is 5 bytes; framing must not exceed the lease either.
        assert!(OwnedFrame::encode_json(&"abc", 5, 5, io::ErrorKind::InvalidInput).is_err());
        let frame = OwnedFrame::encode_json(&"abc", 6, 6, io::ErrorKind::InvalidInput).unwrap();
        assert_eq!(frame.bytes, b"\"abc\"\n");
        assert_eq!(frame.bytes.capacity(), 6);
    }

    // Same real blocked-stdin pattern as transport/writer_tests.rs, kept here
    // to exercise the new private OwnedFrame lifetime without widening APIs.
    #[cfg(unix)]
    struct ActualPipe {
        child: crate::process_tree::ProcessTree,
        root: std::path::PathBuf,
        output: mpsc::Receiver<String>,
    }
    #[cfg(unix)]
    impl ActualPipe {
        fn new(fail: bool) -> (Self, PipeWriter) {
            use std::io::BufRead;
            use std::os::unix::fs::PermissionsExt;
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let root = std::env::temp_dir().join(format!(
                "velune-writer-lifetime-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut command = crate::process::command("/usr/bin/python3");
            command
                .arg("-u")
                .arg("-c")
                .arg(
                    r#"
import pathlib,sys,time
root=pathlib.Path(sys.argv[1])
print('ready',flush=True)
first=sys.stdin.buffer.read(1)
print('actual-first-byte',flush=True)
while not (root/'release').exists(): time.sleep(.001)
if sys.argv[2]=='fail': sys.exit(0)
rest=sys.stdin.buffer.readline()
print(len(first)+len(rest),flush=True)
"#,
                )
                .arg(&root)
                .arg(if fail { "fail" } else { "success" })
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            let mut child = crate::process_tree::ProcessTree::spawn(&mut command).unwrap();
            let writer = PipeWriter::new(child.take_stdin().unwrap()).unwrap();
            let mut stdout = std::io::BufReader::new(child.take_stdout().unwrap());
            let (sender, output) = mpsc::channel();
            thread::spawn(move || {
                let mut line = String::new();
                while stdout.read_line(&mut line).unwrap_or(0) != 0 {
                    if sender.send(line.trim().to_owned()).is_err() {
                        break;
                    }
                    line.clear();
                }
            });
            let gate = Self {
                child,
                root,
                output,
            };
            assert_eq!(gate.line(), "ready");
            (gate, writer)
        }
        fn line(&self) -> String {
            self.output
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
        }
        fn release(&self) {
            std::fs::write(self.root.join("release"), []).unwrap();
        }
    }
    #[cfg(unix)]
    impl Drop for ActualPipe {
        fn drop(&mut self) {
            self.child.stop();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[cfg(unix)]
    struct LifetimeSignal(mpsc::Sender<()>);
    #[cfg(unix)]
    impl Drop for LifetimeSignal {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    #[cfg(unix)]
    #[test]
    fn actual_lifetime_survives_blocked_write_observation_timeout_and_receipt_drop() {
        for fail in [false, true] {
            let (gate, writer) = ActualPipe::new(fail);
            let mut bytes = Vec::with_capacity(10 * 1024 * 1024);
            bytes.resize(9 * 1024 * 1024, b'x');
            bytes.push(b'\n');
            let length = bytes.len();
            let (released, lifetime) = mpsc::channel();
            let mut frame = OwnedFrame::new(bytes, 16 * 1024 * 1024);
            frame.retain_lifetime(Arc::new(LifetimeSignal(released)));
            let mut receipt = writer
                .try_enqueue(
                    frame,
                    WriteClass::Ordinary,
                    Instant::now() + std::time::Duration::from_secs(5),
                )
                .unwrap();
            assert_eq!(
                gate.line(),
                "actual-first-byte",
                "actual write_all entered the full pipe"
            );
            assert!(receipt.try_outcome().is_none());
            // Advance only observation's deadline after real IO entry. No
            // fabricated physical receipt or production timeout switch.
            receipt.deadline = Instant::now() - std::time::Duration::from_millis(1);
            assert!(matches!(
                receipt.try_outcome(),
                Some(WriteOutcome::ObservationTimeout { .. })
            ));
            drop(receipt);
            assert!(matches!(
                lifetime.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            assert_eq!(writer.usage.lock().unwrap().ordinary_count, 1);
            gate.release();
            if !fail {
                assert_eq!(gate.line(), length.to_string());
            }
            lifetime
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let usage = writer.usage.lock().unwrap();
            assert_eq!(usage.ordinary_count, 0);
            assert_eq!(usage.ordinary_bytes, 0);
            assert_eq!(
                usage.failed, fail,
                "release or real IO error reached physical terminal"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn not_admitted_returns_the_original_buffer_and_lifetime_until_caller_disposes_it() {
        let (_gate, writer) = ActualPipe::new(false);
        let (released, lifetime) = mpsc::channel();
        let mut frame = OwnedFrame::new(b"not-dispatched\n".to_vec(), 1024);
        let pointer = frame.bytes.as_ptr();
        let capacity = frame.bytes.capacity();
        frame.retain_lifetime(Arc::new(LifetimeSignal(released)));
        let rejected = match writer.try_enqueue(
            frame,
            WriteClass::Ordinary,
            Instant::now() - std::time::Duration::from_millis(1),
        ) {
            Ok(_) => panic!("expired admission unexpectedly dispatched"),
            Err(rejected) => rejected,
        };
        assert_eq!(rejected.error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(rejected.frame.bytes.as_ptr(), pointer);
        assert_eq!(rejected.frame.bytes.capacity(), capacity);
        assert!(matches!(
            lifetime.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert_eq!(writer.usage.lock().unwrap().ordinary_count, 0);
        drop(rejected);
        lifetime
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }
}
