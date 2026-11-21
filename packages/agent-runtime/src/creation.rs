//! Rust-only resource policy for one opaque session/new. This is NOT a wire ID
//! length rule. Reserve caller metadata capacity before arming it; never replay a
//! creation whose identity did not fit or whose physical result is unknown.
use serde_json::Value;
use std::{
    io::{self, BufRead},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug)]
pub struct CreationLimits {
    pub identity_bytes: usize,
    pub record_bytes: usize,
    pub retained_bytes: usize,
    pub records: usize,
    /// Retained known cwd/runtime/prefix copies, charged before native dispatch.
    pub known_bytes: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreationBudgetError(pub &'static str);
impl std::fmt::Display for CreationBudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for CreationBudgetError {}

struct BudgetInner {
    id: u64,
    limits: CreationLimits,
    started: AtomicBool,
    settled: AtomicBool,
    created_within_policy: AtomicBool,
    peak_record_capacity: AtomicUsize,
    lifetime: Mutex<Option<Arc<dyn Send + Sync>>>,
}
/// Cloning a policy does not allocate its maximum buffer or release its credit.
/// The connected RPC owner retains it through returned results and real cleanup.
#[derive(Clone)]
pub struct CreationBudget(Arc<BudgetInner>);
impl std::fmt::Debug for CreationBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreationBudget")
            .field("limits", &self.0.limits)
            .field("in_flight", &self.is_in_flight())
            .finish()
    }
}
impl CreationBudget {
    pub fn new(limits: CreationLimits) -> Result<Self, CreationBudgetError> {
        if limits.identity_bytes == 0
            || limits.record_bytes < 64
            || limits.records == 0
            || limits.retained_bytes < READER_BASE + RECORD_BASE
            || limits.identity_bytes > limits.record_bytes
        {
            return Err(CreationBudgetError("invalid creation resource policy"));
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| CreationBudgetError("creation policy identity exhausted"))?;
        if limits.known_bytes
            > limits
                .retained_bytes
                .saturating_sub(READER_BASE + RECORD_BASE)
        {
            return Err(CreationBudgetError(
                "known creation retention exceeds policy",
            ));
        }
        Ok(Self(Arc::new(BudgetInner {
            id,
            limits,
            started: AtomicBool::new(false),
            settled: AtomicBool::new(false),
            created_within_policy: AtomicBool::new(false),
            peak_record_capacity: AtomicUsize::new(0),
            lifetime: Mutex::new(None),
        })))
    }
    pub fn limits(&self) -> CreationLimits {
        self.0.limits
    }
    pub fn is_in_flight(&self) -> bool {
        self.0.started.load(Ordering::Acquire) && !self.0.settled.load(Ordering::Acquire)
    }
    pub fn is_settled(&self) -> bool {
        self.0.settled.load(Ordering::Acquire)
    }
    pub fn peak_record_capacity(&self) -> usize {
        self.0.peak_record_capacity.load(Ordering::Acquire)
    }
    /// The caller supplies only its controlled actual-IO lease, never Repository,
    /// Core or a live adapter. Attach immediately before native admission.
    pub fn retain_until_settled(
        &self,
        lifetime: Arc<dyn Send + Sync>,
    ) -> Result<(), CreationBudgetError> {
        let mut slot = self.0.lifetime.lock().unwrap();
        if self.0.started.load(Ordering::Acquire) || slot.is_some() {
            return Err(CreationBudgetError(
                "creation lease already attached or dispatched",
            ));
        }
        *slot = Some(lifetime);
        Ok(())
    }
    pub(crate) fn created(&self, id: &str) -> CreatedIdentity {
        let within_policy = id.len() <= self.0.limits.identity_bytes;
        self.0
            .created_within_policy
            .store(within_policy, Ordering::Release);
        CreatedIdentity {
            native_id: Arc::from(id),
            within_policy,
            budget_id: self.0.id,
            retention: None,
        }
    }
    pub fn owns_identity(&self, identity: &CreatedIdentity) -> bool {
        self.0.id == identity.budget_id
    }
    pub fn identity_retained_bound(bytes: usize) -> Option<usize> {
        bytes
            .checked_mul(192)?
            .checked_add(READER_BASE + RECORD_BASE + 128 * 1024)
    }
    pub fn cancel_before_dispatch(&self) -> Result<(), CreationBudgetError> {
        self.start()?;
        self.settle();
        Ok(())
    }
    fn start(&self) -> Result<(), CreationBudgetError> {
        let _slot = self.0.lifetime.lock().unwrap();
        self.0
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| CreationBudgetError("creation policy cannot be replayed"))
    }
    fn settle(&self) {
        self.0.settled.store(true, Ordering::Release);
        self.0.lifetime.lock().unwrap().take();
    }
    fn lifetime(&self) -> Option<Arc<dyn Send + Sync>> {
        self.0.lifetime.lock().unwrap().clone()
    }
}
#[derive(Clone)]
pub struct CreatedIdentity {
    native_id: Arc<str>,
    within_policy: bool,
    budget_id: u64,
    retention: Option<Arc<DataRetention>>,
}
impl std::fmt::Debug for CreatedIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreatedIdentity")
            .field("native_id", &self.native_id)
            .field("within_policy", &self.within_policy)
            .field("budget_id", &self.budget_id)
            .finish()
    }
}
impl PartialEq for CreatedIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.native_id == other.native_id
            && self.within_policy == other.within_policy
            && self.budget_id == other.budget_id
    }
}
impl Eq for CreatedIdentity {}
impl CreatedIdentity {
    pub(crate) fn retain_data(mut self, retention: DataRetention) -> Self {
        self.retention = Some(Arc::new(retention));
        self
    }
    // The receive scanner already reserves decoder-derived identity/catalog
    // copies. Share that origin until both the result and installed data drop.
    pub(crate) fn projection_retention(&self) -> DataRetention {
        self.retention
            .as_ref()
            .map_or_else(DataRetention::default, |owner| {
                DataRetention(owner.0.clone())
            })
    }
    pub fn native_id(&self) -> &str {
        &self.native_id
    }
    pub fn within_policy(&self) -> bool {
        self.within_policy
    }
    /// Exact-capacity target identity for the caller's precharged link skeleton.
    pub fn prefixed(&self, prefix: &str) -> Result<String, CreationBudgetError> {
        let len = prefix
            .len()
            .checked_add(self.native_id.len())
            .ok_or(CreationBudgetError("creation identity size overflow"))?;
        let mut value = String::new();
        value
            .try_reserve_exact(len)
            .map_err(|_| CreationBudgetError("creation identity allocation failed"))?;
        value.push_str(prefix);
        value.push_str(&self.native_id);
        Ok(value.into_boxed_str().into_string())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreationOutcome {
    /// Even an over-policy identity is real Created evidence, not invalid wire
    /// data or a caller envelope violation. Keep it for recovery, prompt zero.
    Created(CreatedIdentity),
    NotWritten {
        message: String,
    },
    Rejected {
        message: String,
    },
    /// No complete identity. May already exist after a Written/attempted request.
    Unknown {
        message: String,
    },
}
pub(crate) fn detail(message: impl std::fmt::Display) -> String {
    use std::fmt::Write;
    struct Bounded(String);
    impl std::fmt::Write for Bounded {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            let left = 512usize.saturating_sub(self.0.len());
            let mut end = text.len().min(left);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.0.push_str(&text[..end]);
            Ok(())
        }
    }
    let mut output = Bounded(String::new());
    let _ = write!(&mut output, "{message}");
    output.0
}

// New finite legacy receive policy, not a claim about an existing protocol cap.
// 128 MiB single records retain the existing >8 MiB large-record contract. JSON
// layout/clone costs are charged separately; no first-oversize exemption exists.
const DEFAULT_RECORD_BYTES: usize = 128 * 1024 * 1024;
const DEFAULT_RETAINED_BYTES: usize = if usize::BITS >= 64 {
    4_294_967_296u64 as usize
} else {
    usize::MAX - 16 * 1024 * 1024
};
const DEFAULT_RECORDS: usize = 512;
// Existing stderr retention + fixed BufReader/channel/bookkeeping allocation.
const READER_BASE: usize = 2 * 1024 * 1024 + 128 * 1024;
const RECORD_BASE: usize = 4096;
#[derive(Clone, Copy)]
struct ReadLimits {
    record_bytes: usize,
    retained_bytes: usize,
    records: usize,
}
impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            record_bytes: DEFAULT_RECORD_BYTES,
            retained_bytes: DEFAULT_RETAINED_BYTES,
            records: DEFAULT_RECORDS,
        }
    }
}
struct ReadUsage {
    used: usize,
    records: usize,
    data_used: usize,
    retained_limit: usize,
}

// Data can outlive its connection without keeping the execution/HOME anchor.
// Only the reader, writer jobs and protocol owner retain ReadAccount itself.
struct ReadLedger(Mutex<ReadUsage>);
struct ReadState {
    limits: ReadLimits,
    creation: Option<CreationBudget>,
    native_known: usize,
    native_lifetime: Option<Arc<dyn Send + Sync>>,
    active_raw_bytes: usize,
}
pub(crate) struct ReadAccount {
    state: Mutex<ReadState>,
    ledger: Arc<ReadLedger>,
}
impl ReadAccount {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ReadState {
                limits: ReadLimits::default(),
                creation: None,
                native_known: 0,
                native_lifetime: None,
                active_raw_bytes: 0,
            }),
            ledger: Arc::new(ReadLedger(Mutex::new(ReadUsage {
                used: READER_BASE,
                records: 0,
                data_used: 0,
                retained_limit: ReadLimits::default().retained_bytes,
            }))),
        })
    }
    pub(crate) fn arm(&self, budget: &CreationBudget) -> Result<(), CreationBudgetError> {
        let mut state = self.state.lock().unwrap();
        let mut usage = self.ledger.0.lock().unwrap();
        let limits = budget.limits();
        if state.creation.is_some()
            || state.active_raw_bytes > limits.record_bytes
            || usage
                .used
                .checked_add(limits.known_bytes)
                .is_none_or(|n| n > limits.retained_bytes)
            || usage.records > limits.records
        {
            return Err(CreationBudgetError(
                "creation receive capacity unavailable before dispatch",
            ));
        }
        budget.start()?;
        usage.used += limits.known_bytes;
        usage.retained_limit = limits.retained_bytes;
        state.limits = ReadLimits {
            record_bytes: limits.record_bytes,
            retained_bytes: limits.retained_bytes,
            records: limits.records,
        };
        state.creation = Some(budget.clone());
        Ok(())
    }
    pub(crate) fn active(&self) -> bool {
        self.state.lock().unwrap().creation.is_some()
    }
    pub(crate) fn same(&self, budget: &CreationBudget) -> bool {
        self.state
            .lock()
            .unwrap()
            .creation
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(&current.0, &budget.0))
    }
    /// Only used before dispatch or after a healthy Created response and actual
    /// writer completion. Unknown/oversize uses actual RPC-owner destruction.
    pub(crate) fn finish(
        &self,
        budget: &CreationBudget,
        undispatched: bool,
    ) -> Result<(), CreationBudgetError> {
        let mut state = self.state.lock().unwrap();
        let mut usage = self.ledger.0.lock().unwrap();
        if !state
            .creation
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(&current.0, &budget.0))
            || (!undispatched && !budget.0.created_within_policy.load(Ordering::Acquire))
        {
            return Err(CreationBudgetError(
                "creation outcome requires actual cleanup",
            ));
        }
        if !undispatched
            && (state.active_raw_bytes != 0
                || usage.records > 1
                || usage.used
                    > READER_BASE
                        .saturating_add(state.native_known)
                        .saturating_add(budget.limits().known_bytes)
                        .saturating_add(usage.data_used)
                        .saturating_add(usage.records.saturating_mul(RECORD_BASE)))
        {
            return Err(CreationBudgetError(
                "creation still has actual input/queued results",
            ));
        }
        if undispatched {
            usage.used -= budget.limits().known_bytes;
        } else {
            usage.used -= state.native_known;
            state.native_known = budget.limits().known_bytes;
            state.native_lifetime = budget.lifetime();
        }
        state.creation = None;
        state.limits = ReadLimits::default();
        usage.retained_limit = state.limits.retained_bytes;
        budget.settle();
        Ok(())
    }
    pub(crate) fn lease(self: &Arc<Self>) -> io::Result<ReadLease> {
        let state = self.state.lock().unwrap();
        let mut usage = self.ledger.0.lock().unwrap();
        if usage.records >= state.limits.records
            || RECORD_BASE > state.limits.retained_bytes.saturating_sub(usage.used)
        {
            return Err(budget_error());
        }
        usage.records += 1;
        usage.used += RECORD_BASE;
        Ok(ReadLease {
            account: Arc::downgrade(self),
            ledger: self.ledger.clone(),
            charge: RECORD_BASE,
            counted: true,
            reading: false,
        })
    }
}
impl Drop for ReadAccount {
    fn drop(&mut self) {
        // Data leases hold only the ledger. Reaching this boundary means the
        // actual reader, physical writer jobs and protocol owner are gone.
        let state = self.state.get_mut().unwrap();
        let creation = state.creation.take();
        let fixed = READER_BASE
            + state.native_known
            + creation
                .as_ref()
                .map_or(0, |budget| budget.limits().known_bytes);
        self.ledger.0.lock().unwrap().used -= fixed;
        if let Some(budget) = creation {
            budget.settle();
        }
    }
}
pub(crate) struct ReadLease {
    account: Weak<ReadAccount>,
    ledger: Arc<ReadLedger>,
    charge: usize,
    counted: bool,
    reading: bool,
}
impl ReadLease {
    fn settle_history_page(&mut self, retained: usize) {
        let Some(account) = self.account.upgrade() else {
            return;
        };
        let state = account.state.lock().unwrap();
        if state.creation.is_some() || self.counted || self.reading || retained >= self.charge {
            return;
        }
        let mut usage = self.ledger.0.lock().unwrap();
        let released = self.charge - retained;
        usage.used -= released;
        usage.data_used -= released;
        self.charge = retained;
    }
    fn clone_data_charge(&self) -> io::Result<Self> {
        let mut usage = self.ledger.0.lock().unwrap();
        let used = usage
            .used
            .checked_add(self.charge)
            .ok_or_else(budget_error)?;
        if used > usage.retained_limit {
            return Err(budget_error());
        }
        usage.used = used;
        usage.data_used += self.charge;
        Ok(Self {
            account: self.account.clone(),
            ledger: self.ledger.clone(),
            charge: self.charge,
            counted: false,
            reading: false,
        })
    }
    fn complete_record(&mut self) {
        if let Some(account) = self.account.upgrade() {
            account.state.lock().unwrap().active_raw_bytes = 0;
        }
        self.reading = false;
    }
    pub(crate) fn transfer_to_native(&mut self) {
        if self.counted {
            let mut usage = self.ledger.0.lock().unwrap();
            usage.records -= 1;
            usage.data_used += self.charge;
            self.counted = false;
        }
    }
    pub(crate) fn transfer_to_queue(&mut self) -> io::Result<()> {
        if !self.counted {
            let account = self.account.upgrade().ok_or_else(budget_error)?;
            let state = account.state.lock().unwrap();
            let mut usage = self.ledger.0.lock().unwrap();
            if usage.records >= state.limits.records {
                return Err(budget_error());
            }
            usage.records += 1;
            usage.data_used -= self.charge;
            self.counted = true;
        }
        Ok(())
    }
    fn grow(&mut self, total: usize, raw_len: usize, capacity: usize) -> io::Result<()> {
        let account = self.account.upgrade().ok_or_else(budget_error)?;
        let mut state = account.state.lock().unwrap();
        let mut usage = self.ledger.0.lock().unwrap();
        let delta = total.saturating_sub(self.charge);
        if raw_len > state.limits.record_bytes
            || delta > state.limits.retained_bytes.saturating_sub(usage.used)
        {
            return Err(budget_error());
        }
        usage.used += delta;
        if raw_len > 0 {
            state.active_raw_bytes = raw_len;
            self.reading = true;
        }
        self.charge = self.charge.max(total);
        if let Some(budget) = &state.creation {
            budget
                .0
                .peak_record_capacity
                .fetch_max(capacity, Ordering::Relaxed);
        }
        Ok(())
    }
}
impl Drop for ReadLease {
    fn drop(&mut self) {
        if self.reading
            && let Some(account) = self.account.upgrade()
        {
            account.state.lock().unwrap().active_raw_bytes = 0;
        }
        let mut usage = self.ledger.0.lock().unwrap();
        usage.used -= self.charge;
        if self.counted {
            usage.records -= 1;
        } else {
            usage.data_used -= self.charge;
        }
    }
}

/// An owned receive result together with its retained-data accounting.
/// Mapping moves the payload and its charge; it does not retain a process,
/// HOME lease, reader thread or writer job after their actual cleanup.
pub struct Retained<T> {
    value: T,
    retention: DataRetention,
}

// A single decoded record can be moved into more than one typed holder. Its
// conservative receive charge stays until every moved piece is disposed of.
/// Ownership of receive charges, separate from execution/HOME lifetimes.
/// An empty default is for sources outside this RPC receiver only; it is not
/// proof that an external helper result has a receive-budget charge.
#[derive(Default)]
pub struct DataRetention(Vec<Arc<ReadLease>>);
impl std::fmt::Debug for DataRetention {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DataRetention")
            .field("source_records", &self.0.len())
            .finish()
    }
}
impl DataRetention {
    /// Reserve an independent charge before deep-copying retained payload.
    /// On failure, all reservations made by this attempt are returned.
    pub fn try_clone(&self) -> io::Result<Self> {
        let mut leases = Vec::new();
        for source in &self.0 {
            let lease = source.clone_data_charge()?;
            leases
                .try_reserve(1)
                .map_err(|_| io::Error::other("receive clone bookkeeping allocation failed"))?;
            leases.push(Arc::new(lease));
        }
        Ok(Self(leases))
    }
    /// Split ownership for disjoint moved or explicitly precharged derived data.
    /// This is not an admission policy for arbitrary deep payload clones.
    pub fn split(self) -> (Self, Self) {
        let shared = Self(self.0.clone());
        (shared, self)
    }
    /// Combine the accounting for payload pieces moved into the same owner.
    pub fn merge(mut self, other: Self) -> Self {
        self.0.extend(other.0);
        self
    }
    pub(crate) fn into_record(mut self) -> io::Result<ReadLease> {
        if self.0.len() != 1 {
            return Err(budget_error());
        }
        Arc::try_unwrap(self.0.pop().unwrap()).map_err(|_| budget_error())
    }
}
impl<T: std::fmt::Debug> std::fmt::Debug for Retained<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(formatter)
    }
}
impl<T: Clone> Retained<T> {
    /// Reserve data credit before cloning the payload. This does not clone any
    /// process/HOME or location IO lease.
    pub fn try_clone(&self) -> io::Result<Self> {
        let retention = self.retention.try_clone()?;
        Ok(Self {
            value: self.value.clone(),
            retention,
        })
    }
}
impl<T> std::ops::Deref for Retained<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T> AsRef<T> for Retained<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}
impl<T> Retained<T> {
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Retained<U> {
        let Self { value, retention } = self;
        Retained {
            value: map(value),
            retention,
        }
    }
    pub fn try_map<U, E>(self, map: impl FnOnce(T) -> Result<U, E>) -> Result<Retained<U>, E> {
        let Self { value, retention } = self;
        Ok(Retained {
            value: map(value)?,
            retention,
        })
    }
    /// Move disjoint decoded fields into independent owners while retaining
    /// the original record's charge until both pieces are gone.
    pub fn split_map<U, V>(self, map: impl FnOnce(T) -> (U, V)) -> (Retained<U>, Retained<V>) {
        let (left, right) = map(self.value);
        let shared = DataRetention(self.retention.0.clone());
        (
            Retained {
                value: left,
                retention: shared,
            },
            Retained {
                value: right,
                retention: self.retention,
            },
        )
    }
    pub fn zip<U>(self, other: Retained<U>) -> Retained<(T, U)> {
        Retained {
            value: (self.value, other.value),
            retention: self.retention.merge(other.retention),
        }
    }
    pub(crate) fn from_record(value: T, mut retention: ReadLease) -> Self {
        retention.transfer_to_native();
        Self {
            value,
            retention: DataRetention(vec![Arc::new(retention)]),
        }
    }
    pub fn from_parts(value: T, retention: DataRetention) -> Self {
        Self { value, retention }
    }
    /// Move payload and accounting together to an existing typed holder.
    /// Keep the returned retention in that holder until its payload is dropped.
    pub fn into_parts(self) -> (T, DataRetention) {
        (self.value, self.retention)
    }
}

impl Retained<Value> {
    /// A completed read-only Codex history response no longer needs the
    /// input buffer and creation-stage headroom reserved while framing it.
    /// Keep a bounded charge for the decoded page and its message projection.
    pub(crate) fn settle_history_page(&mut self) {
        let [lease] = self.retention.0.as_mut_slice() else {
            return;
        };
        if let Some(lease) = Arc::get_mut(lease) {
            lease.settle_history_page(
                RECORD_BASE.saturating_add(history_projection_cost(&self.value)),
            );
        }
    }
}

fn history_projection_cost(value: &Value) -> usize {
    // Allow decoded strings, transcript fields and intermediate copies.
    // Object slots cover the map and the per-item projection indexes.
    match value {
        Value::String(text) => 256usize.saturating_add(text.capacity().saturating_mul(8)),
        Value::Array(values) => values.iter().map(history_projection_cost).fold(
            1024usize.saturating_add(values.capacity().saturating_mul(256)),
            usize::saturating_add,
        ),
        Value::Object(values) => values.iter().fold(1024usize, |n, (key, value)| {
            n.saturating_add(1024)
                .saturating_add(key.capacity().saturating_mul(2))
                .saturating_add(history_projection_cost(value))
        }),
        _ => 64,
    }
}

pub(crate) struct ReadRecord {
    pub(crate) value: Value,
    pub(crate) lease: ReadLease,
}
fn budget_error() -> io::Error {
    io::Error::other("runtime receive resource budget exhausted; record not truncated or replayed")
}

// Incremental size proof, not a second JSON parser. Every input byte is charged
// 32x for raw Vec, decoded strings and bounded creation-result/snapshot copies.
// Every container/string/scalar start additionally reserves 16384/8192/2048 bytes
// for DOM nodes, BTree unused slots, Vec spare capacity and those same copies.
// Charges are checked BEFORE buffer growth/serde parsing. Escapes suppress fake
// structural starts; malformed JSON is subsequently rejected by serde normally.
#[derive(Clone, Copy, Default)]
struct Scan {
    quoted: bool,
    escaped: bool,
    atom: bool,
    bytes: usize,
    nodes: usize,
}
impl Scan {
    fn append(mut self, bytes: &[u8]) -> io::Result<Self> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(budget_error)?;
        for &byte in bytes {
            if self.quoted {
                if self.escaped {
                    self.escaped = false;
                } else if byte == b'\\' {
                    self.escaped = true;
                } else if byte == b'"' {
                    self.quoted = false;
                }
                continue;
            }
            let node = match byte {
                b'"' => {
                    self.quoted = true;
                    self.atom = false;
                    8192
                }
                b'{' | b'[' => {
                    self.atom = false;
                    16384
                }
                b'}' | b']' | b',' | b':' | b' ' | b'\n' | b'\r' | b'\t' => {
                    self.atom = false;
                    0
                }
                _ if !self.atom => {
                    self.atom = true;
                    2048
                }
                _ => 0,
            };
            self.nodes = self.nodes.checked_add(node).ok_or_else(budget_error)?;
        }
        Ok(self)
    }
    fn cost(self) -> io::Result<usize> {
        self.bytes
            .checked_mul(32)
            .and_then(|n| n.checked_add(self.nodes))
            .and_then(|n| n.checked_add(RECORD_BASE))
            .ok_or_else(budget_error)
    }
}
/// The sole common reader implementation. No read_until(Vec), oversized first
/// record exemption, eager allocation of the maximum, or truncated JSON success.
pub(crate) fn read_record(
    reader: &mut impl BufRead,
    account: &Arc<ReadAccount>,
) -> io::Result<Option<ReadRecord>> {
    let mut lease = account.lease()?;
    let mut scan = Scan::default();
    let mut bytes = Vec::new();
    loop {
        let input = reader.fill_buf()?;
        if input.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        let take = input
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |n| n + 1);
        let next = scan.append(&input[..take])?;
        lease.grow(next.cost()?, next.bytes, bytes.capacity())?;
        bytes.try_reserve_exact(take).map_err(|_| budget_error())?;
        lease.grow(next.cost()?, next.bytes, bytes.capacity())?;
        bytes.extend_from_slice(&input[..take]);
        let terminal = input[take - 1] == b'\n';
        reader.consume(take);
        scan = next;
        if terminal {
            break;
        }
    }
    let value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    lease.complete_record();
    Ok(Some(ReadRecord { value, lease }))
}

#[cfg(all(test, unix))]
#[path = "creation_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "creation/cold_history_tests.rs"]
mod cold_history_tests;
