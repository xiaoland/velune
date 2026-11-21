//! One links-file ordered domain. Not installed until every runtime links writer is switched.
//! Owner calls reserve BEFORE native side effects, then enqueue/poll even if its view disappears.
//! Only poll_result installs strict successes; tickets are not acknowledgements of persistence.
use super::{LinksDiskLease, LinksHomeGate, RuntimeError, decode_conversation_links};
use crate::config::{ConversationLink, ConversationSegmentReference};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};
use velune_agent_runtime::creation::{CreatedIdentity, CreationBudget, CreationLimits};

#[derive(Clone, Copy)]
pub(crate) struct LinksLimits {
    pub reservations: usize,
    pub patch_bytes: usize,
    pub value_bytes: usize,
    pub file_bytes: usize,
    pub retained_bytes: usize,
}
/// A nonnegative contribution to the final file, NOT just retained workspace.
/// Reserve the entire possible new link; never borrow the old link's deletion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinksGrowth {
    pub value_bytes: usize,
    pub file_bytes: usize,
}
impl LinksGrowth {
    pub(crate) fn for_new_link(new: Option<&ConversationLink>) -> Self {
        new.map_or(Self::default(), |link| Self {
            value_bytes: std::mem::size_of::<ConversationLink>().saturating_add(link_cost(link)),
            file_bytes: encoded_link_bound(link),
        })
    }
    fn baseline(current: &Vec<ConversationLink>) -> Self {
        Self {
            value_bytes: value_cost(current),
            file_bytes: current
                .iter()
                .map(encoded_link_bound)
                .fold(64, usize::saturating_add),
        }
    }
    fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            value_bytes: self.value_bytes.checked_add(other.value_bytes)?,
            file_bytes: self.file_bytes.checked_add(other.file_bytes)?,
        })
    }
    fn fits(self, limit: Self) -> bool {
        // Saturated estimates are never a capacity proof, even with usize::MAX limits.
        self.value_bytes != usize::MAX
            && self.file_bytes != usize::MAX
            && self.value_bytes <= limit.value_bytes
            && self.file_bytes <= limit.file_bytes
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LinksAdmissionError {
    Busy,
    Closing,
    Capacity,
    InvalidReservation,
    InvalidPatch,
    NotDrained,
    WorkerStopped,
    /// Native Created is retained; this is policy partial success, not a caller
    /// envelope violation or permission to create a replacement automatically.
    CreatedOverBudget,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LinksReservation(u64);
/// Owns the logical-ID reservation and its pre-dispatch SDK policy. No maximum
/// raw buffer or second authoritative link value is allocated here.
pub(crate) struct LinksOpaqueReservation {
    pub reservation: LinksReservation,
    budget: CreationBudget,
}
impl LinksOpaqueReservation {
    pub(crate) fn limits(&self) -> CreationLimits {
        self.budget.limits()
    }
}
struct OpaqueCredit {
    budget: CreationBudget,
    expected_patch_cost: usize,
    native_prefix: String,
    shrunk: bool,
    io_attached: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinksTicket(u64);
pub(crate) struct LinksPatch {
    pub expected_old: Option<ConversationLink>,
    pub new: Option<ConversationLink>,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LinksOutcome {
    Patched,
    BackupReady(Option<PathBuf>),
    KnownNotWritten(String),
    /// No automatic retry/replay. The domain keeps the candidate, budget and HOME lease.
    DurabilityUnknown(String),
}
#[derive(Debug)]
pub(crate) struct LinksReceipt {
    pub ticket: LinksTicket,
    pub outcome: LinksOutcome,
    pub backup: Option<PathBuf>,
}
struct Reservation {
    id: String,
    patch_bytes: usize,
    growth: LinksGrowth,
    charge: usize,
    queued: bool,
    opaque: Option<OpaqueCredit>,
}
enum Operation {
    BackupFence,
    Patch { patch: LinksPatch, backup: bool },
}
struct Queued {
    reservation: u64,
    ticket: LinksTicket,
    operation: Operation,
}
struct DiskJob {
    queued: Queued,
    value: Vec<ConversationLink>,
    _lease: LinksDiskLease,
    backup: Option<PathBuf>,
    verify: bool,
}
struct DiskReply {
    job: DiskJob,
    outcome: LinksOutcome,
}
/// Does not own application state or a second links database. `current` remains the
/// caller's sole in-memory authority and must be passed to every poll.
pub(crate) struct LinksMetadata {
    limits: LinksLimits,
    // Scalar proof only. The authoritative links remain exclusively in the caller.
    committed: LinksGrowth,
    gate: Option<Arc<LinksHomeGate>>,
    reservations: BTreeMap<u64, Reservation>,
    queue: VecDeque<Queued>,
    retained: usize,
    active: bool,
    uncertain: Option<DiskReply>,
    closing: bool,
    jobs: Option<mpsc::SyncSender<DiskJob>>,
    replies: mpsc::Receiver<DiskReply>,
    worker: Option<JoinHandle<()>>,
}
impl LinksMetadata {
    pub(super) fn new(
        home: PathBuf,
        gate: Arc<LinksHomeGate>,
        limits: LinksLimits,
        current: &Vec<ConversationLink>,
    ) -> Result<Self, RuntimeError> {
        Self::start(home, gate, limits, current, Hooks::default())
    }
    fn start(
        home: PathBuf,
        gate: Arc<LinksHomeGate>,
        limits: LinksLimits,
        current: &Vec<ConversationLink>,
        hooks: Hooks,
    ) -> Result<Self, RuntimeError> {
        let committed = LinksGrowth::baseline(current);
        if !committed.fits(LinksGrowth {
            value_bytes: limits.value_bytes,
            file_bytes: limits.file_bytes,
        }) {
            return Err(RuntimeError::invalid(
                "links initial whole-value capacity unavailable",
            ));
        }
        // Fixed shared workspace is reserved before any owner clone or worker allocation.
        // 256x bounds JSON DOM/typed decode transients (including tiny container nodes),
        // input, capped encoding, bounded errors and channel/result bookkeeping. The one
        // active typed value is moved into the reply, never duplicated for delivery.
        let path_budget = home
            .capacity()
            .checked_add(256)
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| RuntimeError::invalid("links target path budget overflow"))?;
        let workspace = limits
            .file_bytes
            .checked_mul(256)
            .and_then(|n| {
                limits
                    .value_bytes
                    .checked_mul(4)
                    .and_then(|v| n.checked_add(v))
            })
            .and_then(|n| n.checked_add(8192))
            .and_then(|n| n.checked_add(path_budget))
            .ok_or_else(|| RuntimeError::invalid("links metadata budget overflow"))?;
        if limits.reservations == 0
            || limits.patch_bytes == 0
            || limits.file_bytes == 0
            || workspace > limits.retained_bytes
        {
            return Err(RuntimeError::invalid("links metadata budget too small"));
        }
        let (jobs, receiver) = mpsc::sync_channel::<DiskJob>(1);
        let (sender, replies) = mpsc::sync_channel(1);
        let worker_gate = gate.clone();
        let worker = thread::Builder::new()
            .name("velune-links-disk".into())
            .spawn(move || {
                let _gate = worker_gate;
                let target = Target::new(home, limits.file_bytes, limits.value_bytes, hooks);
                while let Ok(mut job) = receiver.recv() {
                    let outcome = target.execute(&mut job);
                    let sent = sender.send(DiskReply { job, outcome }).is_ok();
                    target.hooks.after_result_send();
                    if !sent {
                        break;
                    }
                }
            })?;
        Ok(Self {
            limits,
            committed,
            gate: Some(gate),
            reservations: BTreeMap::new(),
            queue: VecDeque::new(),
            retained: workspace,
            active: false,
            uncertain: None,
            closing: false,
            jobs: Some(jobs),
            replies,
            worker: Some(worker),
        })
    }
    /// `patch_bytes` reserves the complete expectedOld/new envelope BEFORE the native
    /// operation. `growth` must bound the ENTIRE possible new link (not a net
    /// delta), including its typed capacities and worst-case JSON bytes. If the
    /// native result cannot be bounded, remain pending/reject BEFORE doing it.
    /// The initial authoritative current is bound at construction; only this
    /// domain may change it. Fence-only deletions can reserve zero growth.
    pub(crate) fn reserve(
        &mut self,
        id: &str,
        patch_bytes: usize,
        growth: LinksGrowth,
    ) -> Result<LinksReservation, LinksAdmissionError> {
        if self.closing {
            return Err(LinksAdmissionError::Closing);
        }
        if self.reservations.values().any(|r| r.id == id) {
            return Err(LinksAdmissionError::Busy);
        }
        let charge = patch_bytes
            .checked_add(id.len())
            .and_then(|n| n.checked_add(2048))
            .ok_or(LinksAdmissionError::Capacity)?;
        if id.is_empty()
            || patch_bytes > self.limits.patch_bytes
            || self.reservations.len() >= self.limits.reservations
            || charge > self.limits.retained_bytes.saturating_sub(self.retained)
        {
            return Err(LinksAdmissionError::Capacity);
        }
        // Includes reserved, queued, active, held replies, successful fences and
        // quarantined writes. Only settle removes a credit; pending deletes add
        // no space. Installation advances committed BEFORE its credit is freed.
        let projected = self
            .reservations
            .values()
            .try_fold(self.committed, |sum, r| sum.checked_add(r.growth))
            .and_then(|sum| sum.checked_add(growth))
            .ok_or(LinksAdmissionError::Capacity)?;
        if !projected.fits(LinksGrowth {
            value_bytes: self.limits.value_bytes,
            file_bytes: self.limits.file_bytes,
        }) {
            return Err(LinksAdmissionError::Capacity);
        }
        let key = self.number()?;
        self.retained += charge;
        self.reservations.insert(
            key,
            Reservation {
                id: id.to_owned(),
                patch_bytes,
                growth,
                charge,
                queued: false,
                opaque: None,
            },
        );
        Ok(LinksReservation(key))
    }
    /// `skeleton` is the complete planned new link, with the final native ID
    /// equal to its KNOWN runtime prefix only. The SDK owns the unknown suffix;
    /// cwd is already known. Reserve before any native new/import side effect.
    pub(crate) fn reserve_opaque(
        &mut self,
        id: &str,
        expected_old: Option<&ConversationLink>,
        skeleton: &ConversationLink,
    ) -> Result<LinksOpaqueReservation, LinksAdmissionError> {
        if self.closing {
            return Err(LinksAdmissionError::Closing);
        }
        if self.reservations.values().any(|r| r.opaque.is_some()) {
            return Err(LinksAdmissionError::Busy);
        }
        let tail = skeleton
            .segments
            .last()
            .ok_or(LinksAdmissionError::InvalidPatch)?;
        if skeleton.id != id
            || expected_old.is_some_and(|old| old.id != id)
            || tail.native_conversation_id.strip_suffix(':')
                != Some(tail.runtime_instance_id.as_str())
        {
            return Err(LinksAdmissionError::InvalidPatch);
        }
        let committed = self
            .reservations
            .values()
            .try_fold(self.committed, |n, r| n.checked_add(r.growth))
            .ok_or(LinksAdmissionError::Capacity)?;
        let known = LinksGrowth::for_new_link(Some(skeleton));
        let expected_cost = std::mem::size_of::<LinksPatch>()
            .checked_add(expected_old.map_or(0, link_cost))
            .ok_or(LinksAdmissionError::Capacity)?;
        let patch_known = expected_cost
            .checked_add(link_cost(skeleton))
            .ok_or(LinksAdmissionError::Capacity)?;
        let base_charge = self
            .limits
            .patch_bytes
            .checked_add(id.len())
            .and_then(|n| n.checked_add(tail.native_conversation_id.len()))
            .and_then(|n| n.checked_add(2048))
            .ok_or(LinksAdmissionError::Capacity)?;
        let available_retained = self
            .limits
            .retained_bytes
            .checked_sub(self.retained)
            .and_then(|n| n.checked_sub(base_charge))
            .ok_or(LinksAdmissionError::Capacity)?;
        // Leave retained headroom for ordinary other-ID intents; the opaque
        // exclusivity gate applies only to a second opaque creation, not them.
        let sdk_retained = available_retained / 2;
        let sdk_known = tail
            .runtime_instance_id
            .capacity()
            .checked_add(tail.native_conversation_id.capacity())
            .and_then(|n| n.checked_add(tail.cwd.as_ref().map_or(0, String::capacity)))
            .and_then(|n| n.checked_add(256))
            .and_then(|n| n.checked_mul(32))
            .ok_or(LinksAdmissionError::Capacity)?;
        let fixed = CreationBudget::identity_retained_bound(0)
            .and_then(|n| n.checked_add(sdk_known))
            .ok_or(LinksAdmissionError::Capacity)?;
        let record_bytes = self.limits.file_bytes;
        let identity_bytes = self
            .limits
            .value_bytes
            .checked_sub(committed.value_bytes)
            .and_then(|n| n.checked_sub(known.value_bytes))
            .ok_or(LinksAdmissionError::Capacity)?
            .min(
                self.limits
                    .file_bytes
                    .checked_sub(committed.file_bytes)
                    .and_then(|n| n.checked_sub(known.file_bytes))
                    .ok_or(LinksAdmissionError::Capacity)?
                    / 6,
            )
            .min(
                self.limits
                    .patch_bytes
                    .checked_sub(patch_known)
                    .ok_or(LinksAdmissionError::Capacity)?,
            )
            .min(record_bytes.saturating_sub(256) / 6)
            .min(sdk_retained.saturating_sub(fixed) / 192);
        if identity_bytes == 0 {
            return Err(LinksAdmissionError::Capacity);
        }
        let growth = known
            .checked_add(LinksGrowth {
                value_bytes: identity_bytes,
                file_bytes: identity_bytes
                    .checked_mul(6)
                    .ok_or(LinksAdmissionError::Capacity)?,
            })
            .ok_or(LinksAdmissionError::Capacity)?;
        let budget = CreationBudget::new(CreationLimits {
            identity_bytes,
            record_bytes,
            retained_bytes: sdk_retained,
            records: 32,
            known_bytes: sdk_known,
        })
        .map_err(|_| LinksAdmissionError::Capacity)?;
        let patch_bytes = patch_known
            .checked_add(identity_bytes)
            .ok_or(LinksAdmissionError::Capacity)?;
        let reservation = self.reserve(id, patch_bytes, growth)?;
        let extra_charge = sdk_retained
            .checked_add(tail.native_conversation_id.len())
            .ok_or(LinksAdmissionError::Capacity)?;
        // Checked above before any clone: full SDK input/result retention stays
        // charged until the real reservation settles, not just ID observation.
        self.retained += extra_charge;
        let r = self.reservations.get_mut(&reservation.0).unwrap();
        r.charge += extra_charge;
        r.opaque = Some(OpaqueCredit {
            budget: budget.clone(),
            expected_patch_cost: expected_cost,
            native_prefix: tail.native_conversation_id.clone(),
            shrunk: false,
            io_attached: false,
        });
        Ok(LinksOpaqueReservation {
            reservation,
            budget,
        })
    }
    /// Attach the actual controlled native-IO HOME lease immediately before SDK
    /// admission. The SDK reader/process/retained-result owners, not a view, keep it.
    pub(crate) fn start_opaque_io(
        &mut self,
        reservation: &LinksReservation,
    ) -> Result<CreationBudget, LinksAdmissionError> {
        let r = self
            .reservations
            .get_mut(&reservation.0)
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        let opaque = r
            .opaque
            .as_mut()
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        if opaque.io_attached || r.queued {
            return Err(LinksAdmissionError::Busy);
        }
        let lease = self
            .gate
            .as_ref()
            .ok_or(LinksAdmissionError::Closing)?
            .disk_lease()
            .map_err(|_| LinksAdmissionError::Closing)?;
        opaque
            .budget
            .retain_until_settled(Arc::new(lease))
            .map_err(|_| LinksAdmissionError::Busy)?;
        opaque.io_attached = true;
        Ok(opaque.budget.clone())
    }
    /// Borrow the real Created identity and actual link capacities before cloning
    /// a queued patch. An oversize complete identity remains Created/recoverable.
    /// No SDK/result credit is returned by this monotonic growth shrink.
    pub(crate) fn shrink_opaque(
        &mut self,
        reservation: &LinksReservation,
        identity: &CreatedIdentity,
        new: &ConversationLink,
    ) -> Result<(), LinksAdmissionError> {
        let r = self
            .reservations
            .get_mut(&reservation.0)
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        let opaque = r
            .opaque
            .as_mut()
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        if r.queued || !opaque.io_attached {
            return Err(LinksAdmissionError::Busy);
        }
        let growth = LinksGrowth::for_new_link(Some(new));
        let patch_bytes = opaque
            .expected_patch_cost
            .checked_add(link_cost(new))
            .ok_or(LinksAdmissionError::CreatedOverBudget)?;
        if !opaque.budget.owns_identity(identity)
            || !identity.within_policy()
            || new.id != r.id
            || new.segments.last().and_then(|tail| {
                tail.native_conversation_id
                    .strip_prefix(&opaque.native_prefix)
            }) != Some(identity.native_id())
            || !growth.fits(r.growth)
            || patch_bytes > r.patch_bytes
        {
            return Err(LinksAdmissionError::CreatedOverBudget);
        }
        let returned = r.patch_bytes - patch_bytes;
        self.retained -= returned;
        r.charge -= returned;
        r.patch_bytes = patch_bytes;
        r.growth = growth;
        opaque.shrunk = true;
        Ok(())
    }
    fn number(&mut self) -> Result<u64, LinksAdmissionError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.try_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |n| n.checked_add(1),
        )
        .map_err(|_| LinksAdmissionError::Capacity)
    }
    pub(crate) fn enqueue_patch(
        &mut self,
        reservation: &LinksReservation,
        patch: LinksPatch,
        backup: bool,
    ) -> Result<LinksTicket, LinksAdmissionError> {
        let r = self
            .reservations
            .get(&reservation.0)
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        if r.opaque
            .as_ref()
            .is_some_and(|opaque| !opaque.shrunk || opaque.budget.is_in_flight())
        {
            return Err(LinksAdmissionError::Busy);
        }
        if [&patch.expected_old, &patch.new]
            .into_iter()
            .flatten()
            .any(|link| link.id != r.id)
            || patch_cost(&patch) > r.patch_bytes
            || !LinksGrowth::for_new_link(patch.new.as_ref()).fits(r.growth)
        {
            return Err(LinksAdmissionError::InvalidPatch);
        }
        self.enqueue(reservation, Operation::Patch { patch, backup })
    }
    /// A fence shares the patch FIFO and keeps the reservation after its successful
    /// receipt. Only then may the caller perform destructive native work.
    pub(crate) fn enqueue_backup_fence(
        &mut self,
        reservation: &LinksReservation,
    ) -> Result<LinksTicket, LinksAdmissionError> {
        let r = self
            .reservations
            .get(&reservation.0)
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        // A fence must precede handing native IO its policy. Its failure may
        // settle an unstarted reservation, never a policy already handed out
        // (including the worker-before-arm window) or an actual native job.
        if r.opaque.as_ref().is_some_and(|opaque| opaque.io_attached) {
            return Err(LinksAdmissionError::Busy);
        }
        self.enqueue(reservation, Operation::BackupFence)
    }
    fn enqueue(
        &mut self,
        reservation: &LinksReservation,
        operation: Operation,
    ) -> Result<LinksTicket, LinksAdmissionError> {
        let r = self
            .reservations
            .get(&reservation.0)
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        if r.queued {
            return Err(LinksAdmissionError::Busy);
        }
        let ticket = LinksTicket(self.number()?);
        self.reservations.get_mut(&reservation.0).unwrap().queued = true;
        self.queue.push_back(Queued {
            reservation: reservation.0,
            ticket,
            operation,
        });
        Ok(ticket)
    }
    /// Only a reservation with no accepted file operation may be abandoned.
    pub(crate) fn release(
        &mut self,
        reservation: &LinksReservation,
    ) -> Result<(), LinksAdmissionError> {
        let r = self
            .reservations
            .get(&reservation.0)
            .ok_or(LinksAdmissionError::InvalidReservation)?;
        if r.queued
            || r.opaque
                .as_ref()
                .is_some_and(|opaque| opaque.budget.is_in_flight())
        {
            return Err(LinksAdmissionError::Busy);
        }
        if let Some(opaque) = &r.opaque {
            // start/cancel use one CAS: if a worker won dispatch after our
            // first observation, do not free its reservation or IO credit.
            if opaque.budget.cancel_before_dispatch().is_err() && !opaque.budget.is_settled() {
                return Err(LinksAdmissionError::Busy);
            }
        }
        self.settle(reservation.0);
        Ok(())
    }
    fn settle(&mut self, key: u64) {
        let r = self.reservations.remove(&key).unwrap();
        self.retained -= r.charge;
    }
    /// Nonblocking. Always poll independently of business/view lifetime. A strict
    /// success is moved into `current` BEFORE returning its receipt or dispatching
    /// another head. Timeout means only "not ready"; it does not cancel anything.
    pub(crate) fn poll_result(
        &mut self,
        current: &mut Vec<ConversationLink>,
    ) -> Option<LinksReceipt> {
        if self.active && self.uncertain.is_none() {
            match self.replies.try_recv() {
                Ok(mut reply) => {
                    let ticket = reply.job.queued.ticket;
                    match &reply.outcome {
                        LinksOutcome::DurabilityUnknown(message) => {
                            let outcome = LinksOutcome::DurabilityUnknown(message.clone());
                            let backup = reply.job.backup.clone();
                            self.uncertain = Some(reply);
                            return Some(LinksReceipt {
                                ticket,
                                outcome,
                                backup,
                            });
                        }
                        LinksOutcome::Patched => {
                            self.committed = LinksGrowth::baseline(&reply.job.value);
                            *current = std::mem::take(&mut reply.job.value);
                        }
                        _ => {}
                    }
                    self.active = false;
                    if matches!(reply.outcome, LinksOutcome::BackupReady(_)) {
                        self.reservations
                            .get_mut(&reply.job.queued.reservation)
                            .unwrap()
                            .queued = false;
                    } else {
                        self.settle(reply.job.queued.reservation);
                    }
                    return Some(LinksReceipt {
                        ticket,
                        outcome: reply.outcome,
                        backup: reply.job.backup.take(),
                    });
                    // reply's disk lease is released only after result installation/settlement.
                }
                Err(mpsc::TryRecvError::Empty) => return None,
                // Panic/channel loss is fail-closed: never infer no write or free active.
                Err(mpsc::TryRecvError::Disconnected) => return None,
            }
        }
        if self.active {
            return None;
        }
        let queued = self.queue.pop_front()?;
        let r = self.reservations.get(&queued.reservation).unwrap();
        let invalid = match &queued.operation {
            Operation::Patch { patch, .. } => {
                current.iter().find(|link| link.id == r.id) != patch.expected_old.as_ref()
            }
            Operation::BackupFence => false,
        };
        let failure = if invalid {
            Some("links expectedOld conflict")
        } else if LinksGrowth::baseline(current) != self.committed {
            Some("links owner changed outside its reserved domain")
        } else {
            None
        };
        if let Some(message) = failure {
            self.settle(queued.reservation);
            return Some(LinksReceipt {
                ticket: queued.ticket,
                outcome: LinksOutcome::KnownNotWritten(message.into()),
                backup: None,
            });
        }
        let lease = match self.gate.as_ref().unwrap().disk_lease() {
            Ok(lease) => lease,
            Err(error) => {
                self.settle(queued.reservation);
                return Some(LinksReceipt {
                    ticket: queued.ticket,
                    outcome: LinksOutcome::KnownNotWritten(detail(error)),
                    backup: None,
                });
            }
        };
        // Both the whole-value workspace and patch envelope were admitted first.
        let job = DiskJob {
            queued,
            value: current.clone(),
            _lease: lease,
            backup: None,
            verify: false,
        };
        match self.jobs.as_ref().unwrap().try_send(job) {
            Ok(()) => {
                self.active = true;
                None
            }
            Err(error) => {
                let job = match error {
                    mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
                };
                self.settle(job.queued.reservation);
                Some(LinksReceipt {
                    ticket: job.queued.ticket,
                    outcome: LinksOutcome::KnownNotWritten(
                        "links worker unavailable before submission".into(),
                    ),
                    backup: None,
                })
            }
        }
    }
    /// Read/compare + sync only, NEVER write/replay. Pending FIFO stays quarantined
    /// until this confirms the exact candidate's schema/value and durability.
    pub(crate) fn verify_uncertain(&mut self) -> Result<(), LinksAdmissionError> {
        let jobs = self
            .jobs
            .as_ref()
            .ok_or(LinksAdmissionError::WorkerStopped)?;
        let mut reply = self.uncertain.take().ok_or(LinksAdmissionError::Busy)?;
        reply.job.verify = true;
        match jobs.try_send(reply.job) {
            Ok(()) => Ok(()),
            Err(error) => {
                let job = match error {
                    mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
                };
                self.uncertain = Some(DiskReply {
                    job,
                    outcome: reply.outcome,
                });
                Err(LinksAdmissionError::WorkerStopped)
            }
        }
    }
    pub(crate) fn begin_close(&mut self) {
        self.closing = true;
    }
    pub(crate) fn poll_drained(&self) -> bool {
        self.closing
            && self.reservations.is_empty()
            && self.queue.is_empty()
            && !self.active
            && self.uncertain.is_none()
    }
    /// No join of live IO. Retry on later owner turns after the idle thread exits.
    /// The caller still owns native cleanup; this proves ONLY the links domain.
    pub(crate) fn finish_close(&mut self) -> Result<(), LinksAdmissionError> {
        if !self.poll_drained() {
            return Err(LinksAdmissionError::NotDrained);
        }
        self.jobs.take();
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return Err(LinksAdmissionError::NotDrained);
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| LinksAdmissionError::WorkerStopped)?;
        }
        self.gate.take();
        Ok(())
    }
}
// Dropping this component detaches its single worker, never cancels an accepted
// job or joins IO. The job/result lease survives until real return and disposal.

fn detail(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    text.chars().take(512).collect()
}
fn string_cost(value: &String) -> usize {
    value.capacity()
}
fn strings_cost(values: &Vec<String>) -> usize {
    values
        .capacity()
        .saturating_mul(std::mem::size_of::<String>())
        .saturating_add(
            values
                .iter()
                .map(string_cost)
                .fold(0, usize::saturating_add),
        )
}
fn segment_cost(s: &ConversationSegmentReference) -> usize {
    let mut n = s
        .runtime_instance_id
        .capacity()
        .saturating_add(s.native_conversation_id.capacity());
    n = n.saturating_add(s.cwd.as_ref().map_or(0, string_cost));
    n = n.saturating_add(s.cutoff.as_ref().map_or(0, |c| c.prefix_digest.capacity()));
    n = n.saturating_add(s.handoff.as_ref().map_or(0, |h| {
        h.marker
            .capacity()
            .saturating_add(h.payload_digest.capacity())
    }));
    n = n.saturating_add(s.imported.as_ref().map_or(0, |i| {
        i.prefix
            .prefix_digest
            .capacity()
            .saturating_add(i.summary_work_id.as_ref().map_or(0, string_cost))
    }));
    n = n.saturating_add(strings_cost(&s.excluded_message_ids));
    for (key, values) in &s.message_origins {
        // Conservative per-entry allowance includes BTree node overhead/unused slots.
        n = n
            .saturating_add(1024)
            .saturating_add(key.capacity())
            .saturating_add(strings_cost(values));
    }
    n
}
fn link_cost(link: &ConversationLink) -> usize {
    link.id
        .capacity()
        .saturating_add(
            link.segments
                .capacity()
                .saturating_mul(std::mem::size_of::<ConversationSegmentReference>()),
        )
        .saturating_add(
            link.segments
                .iter()
                .map(segment_cost)
                .fold(0, usize::saturating_add),
        )
}
fn value_cost(value: &Vec<ConversationLink>) -> usize {
    value
        .capacity()
        .saturating_mul(std::mem::size_of::<ConversationLink>())
        .saturating_add(value.iter().map(link_cost).fold(0, usize::saturating_add))
}
fn patch_cost(patch: &LinksPatch) -> usize {
    std::mem::size_of::<LinksPatch>().saturating_add(
        [&patch.expected_old, &patch.new]
            .into_iter()
            .flatten()
            .map(link_cost)
            .fold(0, usize::saturating_add),
    )
}

// Size proof, not encoding: serde_json's default pretty formatter adds two spaces
// per level. This schema is at most 9 levels deep. 64 bytes per variable entry
// covers indentation, comma/newline, colon and collection delimiters; 128 per
// link / 1024 per segment cover all fixed keys, nulls, booleans, u64s (<=20
// digits), optional envelopes and closing delimiters. Strings cost at most SIX
// bytes per UTF-8 input byte (\u00XX for controls); quotes are counted separately.
// No Serialize/JSON/IO is executed by admission. Exhaustive destructuring keeps
// schema additions from silently escaping this proof.
fn json_string_bound(text: &str) -> usize {
    text.len().saturating_mul(6).saturating_add(2)
}
fn cutoff_string_bound(cutoff: &crate::config::ConversationCutoff) -> usize {
    let crate::config::ConversationCutoff {
        message_count: _,
        prefix_digest,
    } = cutoff;
    json_string_bound(prefix_digest)
}
fn encoded_link_bound(link: &ConversationLink) -> usize {
    let ConversationLink { id, segments } = link;
    segments.iter().map(encoded_segment_bound).fold(
        128usize.saturating_add(json_string_bound(id)),
        usize::saturating_add,
    )
}
fn encoded_segment_bound(segment: &ConversationSegmentReference) -> usize {
    let ConversationSegmentReference {
        runtime_instance_id,
        runtime_type_id,
        native_conversation_id,
        cwd,
        cutoff,
        handoff,
        imported,
        excluded_message_ids,
        message_origins,
        deleted: _,
    } = segment;
    let mut n = 1024usize;
    for text in [
        runtime_instance_id.as_str(),
        runtime_type_id.id(),
        native_conversation_id.as_str(),
    ] {
        n = n.saturating_add(json_string_bound(text));
    }
    if let Some(cwd) = cwd {
        n = n.saturating_add(json_string_bound(cwd));
    }
    if let Some(cutoff) = cutoff {
        n = n.saturating_add(cutoff_string_bound(cutoff));
    }
    if let Some(handoff) = handoff {
        let crate::config::ConversationHandoff {
            marker,
            payload_digest,
            user_message_ordinal: _,
        } = handoff;
        n = n
            .saturating_add(json_string_bound(marker))
            .saturating_add(json_string_bound(payload_digest));
    }
    if let Some(imported) = imported {
        let crate::config::ConversationImport {
            prefix,
            summary_work_id,
        } = imported;
        n = n.saturating_add(cutoff_string_bound(prefix));
        if let Some(id) = summary_work_id {
            n = n.saturating_add(json_string_bound(id));
        }
    }
    for id in excluded_message_ids {
        n = n.saturating_add(64).saturating_add(json_string_bound(id));
    }
    for (id, targets) in message_origins {
        n = n.saturating_add(64).saturating_add(json_string_bound(id));
        for target in targets {
            n = n
                .saturating_add(64)
                .saturating_add(json_string_bound(target));
        }
    }
    n
}

struct Target {
    path: PathBuf,
    file_limit: usize,
    value_limit: usize,
    instance: u64,
    hooks: Hooks,
}
impl Target {
    fn new(home: PathBuf, file_limit: usize, value_limit: usize, hooks: Hooks) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            path: home.join("conversation-links.json"),
            file_limit,
            value_limit,
            instance: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            hooks,
        }
    }
    fn read(&self) -> Result<Option<Vec<u8>>, RuntimeError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(self.file_limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > self.file_limit {
            return Err(RuntimeError::invalid("links disk file budget exceeded"));
        }
        Ok(Some(bytes))
    }
    fn decode(&self, bytes: Option<&[u8]>) -> Result<Vec<ConversationLink>, RuntimeError> {
        let Some(bytes) = bytes else {
            return Ok(Vec::new());
        };
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        if value["schemaVersion"].as_u64() != Some(3) {
            return Err(RuntimeError::invalid(
                "links require current schema 3; original preserved",
            ));
        }
        decode_conversation_links(value)
    }
    fn execute(&self, job: &mut DiskJob) -> LinksOutcome {
        if job.verify {
            return match self.verify(job) {
                Ok(()) => LinksOutcome::Patched,
                Err(error) => LinksOutcome::DurabilityUnknown(detail(error)),
            };
        }
        match self.prepare(job) {
            Ok(outcome) => outcome,
            Err(error) => LinksOutcome::KnownNotWritten(detail(error)),
        }
    }
    fn prepare(&self, job: &mut DiskJob) -> Result<LinksOutcome, RuntimeError> {
        let bytes = self.read()?;
        if self.decode(bytes.as_deref())? != job.value {
            return Err(RuntimeError::invalid(
                "links disk differs from owner; original preserved",
            ));
        }
        let backup = matches!(
            job.queued.operation,
            Operation::BackupFence | Operation::Patch { backup: true, .. }
        );
        let backup_path = if backup {
            match &bytes {
                Some(bytes) => {
                    let path = self.path.with_file_name(format!(
                        "conversation-links.metadata-backup-{}-{}-{}.json",
                        std::process::id(),
                        self.instance,
                        job.queued.ticket.0
                    ));
                    let mut file = private_new(&path)?;
                    file.write_all(bytes)?;
                    file.sync_all()?;
                    sync_parent(&path)?;
                    Some(path)
                }
                None => None,
            }
        } else {
            None
        };
        job.backup = backup_path.clone();
        let Operation::Patch { patch, .. } = &mut job.queued.operation else {
            return Ok(LinksOutcome::BackupReady(backup_path));
        };
        let id = patch
            .new
            .as_ref()
            .or(patch.expected_old.as_ref())
            .map(|link| link.id.as_str());
        let index = id.and_then(|id| job.value.iter().position(|link| link.id == id));
        match (index, patch.new.take()) {
            (Some(index), Some(new)) => job.value[index] = new,
            (Some(index), None) => {
                job.value.remove(index);
            }
            (None, Some(new)) => job.value.push(new),
            (None, None) => {}
        }
        // expectedOld is no longer needed; do not retain it alongside the result.
        patch.expected_old.take();
        // Box<[T]>::into_vec has capacity == len. Discard outer Vec spare slots
        // before enforcing the final typed limit: appends must not turn allocator
        // growth into an unreserved final-file charge. Transient buffers remain
        // covered by the separate workspace; nested new Vec/String capacities
        // were checked at enqueue and existing ones can only shrink on cloning.
        job.value = std::mem::take(&mut job.value).into_boxed_slice().into_vec();
        if value_cost(&job.value) > self.value_limit {
            return Err(RuntimeError::invalid("links patched value budget exceeded"));
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Envelope<'a> {
            schema_version: u32,
            links: &'a [ConversationLink],
        }
        let mut encoded = CappedBytes {
            bytes: Vec::new(),
            limit: self.file_limit,
        };
        serde_json::to_writer_pretty(
            &mut encoded,
            &Envelope {
                schema_version: 3,
                links: &job.value,
            },
        )?;
        // Validation and encoding stay off owner; invalid new identities never reach rename.
        self.decode(Some(&encoded.bytes))?;
        let temporary = self.path.with_file_name(format!(
            ".conversation-links-{}-{}-{}.tmp",
            std::process::id(),
            self.instance,
            job.queued.ticket.0
        ));
        let result = (|| -> Result<LinksOutcome, RuntimeError> {
            let mut file = private_new(&temporary)?;
            file.write_all(&encoded.bytes)?;
            self.hooks.before_file_sync();
            file.sync_all()?;
            drop(file);
            // Preflight unsupported directory APIs BEFORE changing the target.
            sync_parent(&self.path)?;
            fs::rename(&temporary, &self.path)?;
            if let Err(error) = self
                .hooks
                .after_rename()
                .and_then(|()| sync_parent(&self.path))
            {
                return Ok(LinksOutcome::DurabilityUnknown(detail(error)));
            }
            Ok(LinksOutcome::Patched)
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    fn verify(&self, job: &DiskJob) -> Result<(), RuntimeError> {
        if self.decode(self.read()?.as_deref())? != job.value {
            return Err(RuntimeError::invalid(
                "uncertain links candidate not found; quarantine retained",
            ));
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?
            .sync_all()?;
        sync_parent(&self.path)?;
        Ok(())
    }
}
fn private_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("links parent missing"))?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x02000000); // FILE_FLAG_BACKUP_SEMANTICS: open directory handle.
    }
    // API failures are surfaced, never silently downgraded to successful durability.
    options.open(parent)?.sync_all()
}
struct CappedBytes {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for CappedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("links encoded file budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Default)]
struct Hooks {
    #[cfg(test)]
    hold_sync: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    #[cfg(test)]
    fail_after_rename: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    hold_taken: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    replied: Option<mpsc::Sender<()>>,
}
impl Hooks {
    fn before_file_sync(&self) {
        #[cfg(test)]
        if let Some((entered, release)) = &self.hold_sync
            && !self
                .hold_taken
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
    }
    fn after_result_send(&self) {
        #[cfg(test)]
        if let Some(replied) = &self.replied {
            let _ = replied.send(());
        }
    }
    fn after_rename(&self) -> io::Result<()> {
        #[cfg(test)]
        if self
            .fail_after_rename
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(io::Error::other(
                "synthetic directory sync failure after actual rename",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::Repository;
    use std::time::{Duration, Instant};

    struct Home(PathBuf);
    impl Home {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("velune-links-metadata-{}-{n}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn limits() -> LinksLimits {
        LinksLimits {
            reservations: 4,
            patch_bytes: 16_384,
            value_bytes: 65_536,
            file_bytes: 65_536,
            retained_bytes: 20_000_000,
        }
    }
    fn link(id: &str, version: u32) -> ConversationLink {
        serde_json::from_value(serde_json::json!({
            "id": id, "segments": [{ "runtimeInstanceId": "synthetic",
                "runtimeTypeId": "pi-1.0", "nativeConversationId": format!("synthetic:{id}-{version}"),
                "cwd": null, "cutoff": null, "handoff": null, "deleted": false }]
        })).unwrap()
    }
    fn start(repo: &Repository, hooks: Hooks) -> LinksMetadata {
        start_current(repo, hooks, &Vec::new())
    }
    fn start_current(
        repo: &Repository,
        hooks: Hooks,
        current: &Vec<ConversationLink>,
    ) -> LinksMetadata {
        LinksMetadata::start(
            repo.home.clone(),
            repo.home_lock.claim_links().unwrap(),
            limits(),
            current,
            hooks,
        )
        .unwrap()
    }
    fn hold() -> (
        Hooks,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
        mpsc::Receiver<()>,
    ) {
        let (entered, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let (replied, replied_rx) = mpsc::channel();
        (
            Hooks {
                hold_sync: Some((entered, release_rx)),
                replied: Some(replied),
                ..Hooks::default()
            },
            entered_rx,
            release,
            replied_rx,
        )
    }
    fn receipt(metadata: &mut LinksMetadata, current: &mut Vec<ConversationLink>) -> LinksReceipt {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(receipt) = metadata.poll_result(current) {
                return receipt;
            }
            assert!(Instant::now() < deadline, "real disk worker did not return");
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn close(metadata: &mut LinksMetadata) {
        metadata.begin_close();
        assert!(metadata.poll_drained());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match metadata.finish_close() {
                Ok(()) => break,
                Err(LinksAdmissionError::NotDrained) => {
                    assert!(Instant::now() < deadline);
                    thread::yield_now();
                }
                other => panic!("unexpected finish_close: {other:?}"),
            }
        }
    }
    fn reserve(
        metadata: &mut LinksMetadata,
        id: &str,
        patch_bytes: usize,
    ) -> Result<LinksReservation, LinksAdmissionError> {
        metadata.reserve(
            id,
            patch_bytes,
            LinksGrowth {
                value_bytes: 8192,
                file_bytes: 8192,
            },
        )
    }
    fn patch(
        metadata: &mut LinksMetadata,
        r: &LinksReservation,
        old: Option<ConversationLink>,
        new: Option<ConversationLink>,
    ) -> LinksTicket {
        metadata
            .enqueue_patch(
                r,
                LinksPatch {
                    expected_old: old,
                    new,
                },
                false,
            )
            .unwrap()
    }

    #[test]
    fn held_real_write_preserves_other_ids_and_rejects_same_id_before_native_effects() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let (hooks, entered, release, _) = hold();
        let mut metadata = start(&repo, hooks);
        let mut current = Vec::new();
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        let b = reserve(&mut metadata, "b", 4096).unwrap();
        let ta = patch(&mut metadata, &a, None, Some(link("a", 1)));
        let tb = patch(&mut metadata, &b, None, Some(link("b", 1)));
        assert!(metadata.poll_result(&mut current).is_none());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        // A real private temporary file has been written, but not synced/renamed.
        let native_effect = home.0.join("synthetic-native-mutation");
        match reserve(&mut metadata, "a", 4096) {
            Ok(_) => fs::write(&native_effect, b"must not run").unwrap(),
            Err(error) => assert_eq!(error, LinksAdmissionError::Busy),
        }
        assert!(!native_effect.exists());
        for _ in 0..8 {
            assert!(metadata.poll_result(&mut current).is_none());
        }
        assert!(
            current.is_empty(),
            "enqueue/timeout must not install strict metadata"
        );
        assert!(!home.0.join("conversation-links.json").exists());
        metadata.begin_close();
        assert_eq!(
            reserve(&mut metadata, "c", 4096),
            Err(LinksAdmissionError::Closing)
        );
        assert!(!metadata.poll_drained());
        assert_eq!(
            metadata.finish_close(),
            Err(LinksAdmissionError::NotDrained)
        );
        assert!(repo.unlock().is_err());
        assert!(Repository::lock_home(&home.0).is_err());
        release.send(()).unwrap();
        let first = receipt(&mut metadata, &mut current);
        assert_eq!(first.ticket, ta);
        assert_eq!(first.outcome, LinksOutcome::Patched);
        assert_eq!(current, vec![link("a", 1)]);
        let second = receipt(&mut metadata, &mut current);
        assert_eq!(second.ticket, tb);
        assert_eq!(second.outcome, LinksOutcome::Patched);
        assert_eq!(current, vec![link("a", 1), link("b", 1)]);
        assert_eq!(repo.load_conversation_links().unwrap(), current);
        close(&mut metadata);
        repo.unlock().unwrap();
        assert!(Repository::lock_home(&home.0).is_ok());
    }

    #[test]
    fn backup_fence_copies_exact_validated_bytes_and_retains_reservation_through_patch() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let original = format!(
            " {{\n\"schemaVersion\":3, \"links\":{} }}\n",
            serde_json::to_string(&vec![link("a", 1)]).unwrap()
        )
        .into_bytes();
        let path = home.0.join("conversation-links.json");
        fs::write(&path, &original).unwrap();
        let mut current = vec![link("a", 1)];
        let mut metadata = start_current(&repo, Hooks::default(), &current);
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        let ticket = metadata.enqueue_backup_fence(&a).unwrap();
        let fence = receipt(&mut metadata, &mut current);
        assert_eq!(fence.ticket, ticket);
        let LinksOutcome::BackupReady(Some(backup)) = fence.outcome else {
            panic!("backup not confirmed");
        };
        assert_eq!(fs::read(backup).unwrap(), original);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            reserve(&mut metadata, "a", 4096),
            Err(LinksAdmissionError::Busy)
        );
        metadata.begin_close();
        assert!(
            !metadata.poll_drained(),
            "fence receipt does not release business reservation"
        );
        // Already-reserved follow-up remains admissible during close.
        patch(&mut metadata, &a, Some(link("a", 1)), Some(link("a", 2)));
        assert_eq!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::Patched
        );
        assert_eq!(current, vec![link("a", 2)]);
        close(&mut metadata);
    }

    #[test]
    fn invalid_or_legacy_schema_never_overwrites_or_backs_up_as_current() {
        for bytes in [
            br#"{"schemaVersion":1,"links":[]}"#.as_slice(),
            br#"{"schemaVersion":99,"links":[]}"#,
            br#"{"schemaVersion":3,"links":[],"future":"preserve"}"#,
            br#"{"schemaVersion":3,"links":[{"id":"a","segments":[],"future":true}]}"#,
            br#"{"schemaVersion":3"#,
        ] {
            let home = Home::new();
            let (repo, _) = Repository::open(&home.0).unwrap();
            let path = home.0.join("conversation-links.json");
            fs::write(&path, bytes).unwrap();
            let mut metadata = start(&repo, Hooks::default());
            let a = reserve(&mut metadata, "a", 4096).unwrap();
            metadata
                .enqueue_patch(
                    &a,
                    LinksPatch {
                        expected_old: None,
                        new: Some(link("a", 1)),
                    },
                    true,
                )
                .unwrap();
            let mut current = Vec::new();
            assert!(matches!(
                receipt(&mut metadata, &mut current).outcome,
                LinksOutcome::KnownNotWritten(_)
            ));
            assert!(current.is_empty());
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert!(!fs::read_dir(&home.0).unwrap().any(|e| {
                e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("metadata-backup")
            }));
            close(&mut metadata);
        }
    }

    #[test]
    fn rename_then_sync_failure_quarantines_without_replay_and_installs_only_after_verification() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let hooks = Hooks {
            fail_after_rename: std::sync::atomic::AtomicBool::new(true),
            ..Hooks::default()
        };
        let mut metadata = start(&repo, hooks);
        let mut current = Vec::new();
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        let b = reserve(&mut metadata, "b", 4096).unwrap();
        let ta = patch(&mut metadata, &a, None, Some(link("a", 1)));
        patch(&mut metadata, &b, None, Some(link("b", 1)));
        let unknown = receipt(&mut metadata, &mut current);
        assert_eq!(unknown.ticket, ta);
        assert!(matches!(
            unknown.outcome,
            LinksOutcome::DurabilityUnknown(_)
        ));
        assert!(current.is_empty());
        assert_eq!(repo.load_conversation_links().unwrap(), vec![link("a", 1)]);
        let committed_bytes = fs::read(home.0.join("conversation-links.json")).unwrap();
        for _ in 0..8 {
            assert!(metadata.poll_result(&mut current).is_none());
        }
        assert_eq!(
            fs::read(home.0.join("conversation-links.json")).unwrap(),
            committed_bytes
        );
        assert_eq!(
            reserve(&mut metadata, "a", 4096),
            Err(LinksAdmissionError::Busy)
        );
        metadata.begin_close();
        assert!(!metadata.poll_drained());
        assert!(repo.unlock().is_err());
        metadata.verify_uncertain().unwrap();
        let verified = receipt(&mut metadata, &mut current);
        assert_eq!(verified.ticket, ta);
        assert_eq!(verified.outcome, LinksOutcome::Patched);
        assert_eq!(current, vec![link("a", 1)]);
        assert_eq!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::Patched
        );
        assert_eq!(current, vec![link("a", 1), link("b", 1)]);
        close(&mut metadata);
    }

    #[test]
    fn uncertain_verification_does_not_rewrite_external_corruption() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let mut metadata = start(
            &repo,
            Hooks {
                fail_after_rename: std::sync::atomic::AtomicBool::new(true),
                ..Hooks::default()
            },
        );
        let mut current = Vec::new();
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        patch(&mut metadata, &a, None, Some(link("a", 1)));
        assert!(matches!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::DurabilityUnknown(_)
        ));
        let path = home.0.join("conversation-links.json");
        fs::write(&path, b"synthetic externally corrupted bytes").unwrap();
        metadata.verify_uncertain().unwrap();
        assert!(matches!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::DurabilityUnknown(_)
        ));
        assert!(current.is_empty());
        assert_eq!(
            fs::read(path).unwrap(),
            b"synthetic externally corrupted bytes"
        );
        metadata.begin_close();
        assert!(!metadata.poll_drained());
        assert!(repo.unlock().is_err());
        // Explicit teardown disposes the quarantined result; it does NOT fabricate a drained proof.
        drop(metadata);
    }

    #[test]
    fn repository_drop_cannot_unlock_held_io_or_unsettled_real_result() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let (hooks, entered, release, replied) = hold();
        let mut metadata = start(&repo, hooks);
        let mut current = Vec::new();
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        patch(&mut metadata, &a, None, Some(link("a", 1)));
        assert!(metadata.poll_result(&mut current).is_none());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        metadata.begin_close();
        assert!(!metadata.poll_drained());
        assert!(repo.unlock().is_err());
        drop(repo);
        assert!(Repository::lock_home(&home.0).is_err());
        release.send(()).unwrap();
        replied.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            Repository::lock_home(&home.0).is_err(),
            "returned IO still owns its unconsumed result lease"
        );
        assert!(!metadata.poll_drained());
        assert_eq!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::Patched
        );
        assert_eq!(
            current,
            vec![link("a", 1)],
            "install is independent of the discarded business continuation"
        );
        close(&mut metadata);
        assert!(Repository::lock_home(&home.0).is_ok());
    }

    #[test]
    fn dropping_component_never_joins_held_io_or_releases_its_home_lease() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let (hooks, entered, release, replied) = hold();
        let mut metadata = start(&repo, hooks);
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        patch(&mut metadata, &a, None, Some(link("a", 1)));
        assert!(metadata.poll_result(&mut Vec::new()).is_none());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(metadata); // Must return without waiting for the controlled filesystem gate.
        drop(repo);
        assert!(Repository::lock_home(&home.0).is_err());
        release.send(()).unwrap();
        replied.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(Repository::lock_home(&home.0).is_ok());
        let bytes = fs::read(home.0.join("conversation-links.json")).unwrap();
        assert_eq!(
            decode_conversation_links(serde_json::from_slice(&bytes).unwrap()).unwrap(),
            vec![link("a", 1)]
        );
    }

    #[test]
    fn admission_budget_is_reserved_before_native_work_and_cas_failure_does_not_write() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let mut metadata = start(&repo, Hooks::default());
        assert_eq!(
            reserve(&mut metadata, "a", limits().patch_bytes + 1),
            Err(LinksAdmissionError::Capacity)
        );
        let a = reserve(&mut metadata, "a", 4096).unwrap();
        // Simulates a caller whose captured expectedOld was invalidated before it reached head.
        patch(&mut metadata, &a, Some(link("a", 1)), Some(link("a", 2)));
        let mut current = Vec::new();
        assert!(matches!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::KnownNotWritten(_)
        ));
        assert!(!home.0.join("conversation-links.json").exists());
        let r = reserve(&mut metadata, "a", 4096).unwrap();
        metadata.release(&r).unwrap();
        close(&mut metadata);
    }

    #[test]
    fn cumulative_growth_is_rejected_before_native_and_held_or_unknown_credit_is_not_reused() {
        for typed_limit in [true, false] {
            for unknown in [false, true] {
                let home = Home::new();
                let (repo, _) = Repository::open(&home.0).unwrap();
                let mut base = link("base", 1);
                // Controls exercise six-byte JSON escaping; nested map/list entries
                // exercise variable indentation. No synthetic string is used as an IO path.
                base.segments[0].cwd = Some(
                    format!("/synthetic/{}", "\0".repeat(4096))
                        .into_boxed_str()
                        .into_string(),
                );
                base.segments[0].message_origins.insert(
                    "key\"\\\n".into(),
                    vec!["native\0\n\"\\".into(), "目标".into()],
                );
                let mut prefix = base.segments[0].clone();
                prefix.native_conversation_id = "synthetic:base-prefix".into();
                prefix.cutoff = Some(crate::config::ConversationCutoff {
                    message_count: u64::MAX,
                    prefix_digest: "a".repeat(64),
                });
                prefix.excluded_message_ids = vec!["removed\n\"\\".into()];
                prefix.handoff = Some(crate::config::ConversationHandoff {
                    marker: "b".repeat(64),
                    payload_digest: "c".repeat(64),
                    user_message_ordinal: u64::MAX,
                });
                prefix.imported = Some(crate::config::ConversationImport {
                    prefix: crate::config::ConversationCutoff {
                        message_count: u64::MAX,
                        prefix_digest: "d".repeat(64),
                    },
                    summary_work_id: Some("synthetic-summary".into()),
                });
                // Only the prefix has a cutoff/tombstones; the final segment remains open.
                base.segments.insert(0, prefix);
                let mut current = vec![base];
                let baseline = LinksGrowth::baseline(&current);
                let mut a_value = link("a", 1);
                a_value.segments[0].cwd = Some("\0\"\\".repeat(32));
                // Credit counts actual spare Vec capacity, not just element lengths.
                a_value.segments.reserve_exact(2);
                let mut b_value = link("b", 1);
                b_value.segments[0].cwd = Some("\0\"\\".repeat(32));
                b_value.segments.reserve_exact(2);
                for new in [&a_value, &b_value] {
                    assert!(
                        std::mem::size_of::<LinksPatch>() + link_cost(new) <= 4096,
                        "both complete new-link envelopes are individually legal before native work"
                    );
                }
                let a_growth = LinksGrowth::for_new_link(Some(&a_value));
                let b_growth = LinksGrowth::for_new_link(Some(&b_value));
                let both = baseline
                    .checked_add(a_growth)
                    .unwrap()
                    .checked_add(b_growth)
                    .unwrap();
                let mut capacity = limits();
                capacity.value_bytes = baseline.value_bytes
                    + if typed_limit {
                        a_growth.value_bytes.max(b_growth.value_bytes)
                    } else {
                        a_growth.value_bytes + b_growth.value_bytes
                    };
                capacity.file_bytes = baseline.file_bytes
                    + if typed_limit {
                        a_growth.file_bytes + b_growth.file_bytes
                    } else {
                        a_growth.file_bytes.max(b_growth.file_bytes)
                    };
                assert_eq!(both.value_bytes > capacity.value_bytes, typed_limit);
                assert_eq!(both.file_bytes > capacity.file_bytes, !typed_limit);
                let path = home.0.join("conversation-links.json");
                let original = serde_json::to_vec_pretty(
                    &serde_json::json!({"schemaVersion": 3, "links": &current}),
                )
                .unwrap();
                assert!(original.len() <= baseline.file_bytes);
                fs::write(&path, &original).unwrap();
                let (hooks, entered, release, replied) = hold();
                hooks
                    .fail_after_rename
                    .store(unknown, std::sync::atomic::Ordering::Relaxed);
                let mut metadata = LinksMetadata::start(
                    repo.home.clone(),
                    repo.home_lock.claim_links().unwrap(),
                    capacity,
                    &current,
                    hooks,
                )
                .unwrap();
                let a = metadata.reserve("a", 4096, a_growth).unwrap();
                let deleting = metadata
                    .reserve("base", capacity.patch_bytes, LinksGrowth::default())
                    .unwrap();
                let native_a = home.0.join("native-a");
                let native_b = home.0.join("native-b");
                match metadata.reserve("b", 4096, b_growth) {
                    Ok(_) => fs::write(&native_b, b"must not execute").unwrap(),
                    Err(error) => assert_eq!(error, LinksAdmissionError::Capacity),
                }
                assert!(
                    !native_b.exists(),
                    "combined growth must reject before native work"
                );
                metadata.enqueue_backup_fence(&a).unwrap();
                assert!(matches!(
                    receipt(&mut metadata, &mut current).outcome,
                    LinksOutcome::BackupReady(Some(_))
                ));
                replied.recv_timeout(Duration::from_secs(5)).unwrap();
                assert_eq!(
                    metadata.reserve("b", 4096, b_growth),
                    Err(LinksAdmissionError::Capacity),
                    "a successful backup fence does not return its future patch credit"
                );
                // A legal retained patch envelope can still violate its promised growth.
                let mut oversized_growth = a_value.clone();
                oversized_growth.segments[0].cwd = Some("\0".repeat(1024));
                let invalid = LinksPatch {
                    expected_old: None,
                    new: Some(oversized_growth),
                };
                assert!(patch_cost(&invalid) <= 4096);
                assert_eq!(
                    metadata.enqueue_patch(&a, invalid, false),
                    Err(LinksAdmissionError::InvalidPatch)
                );
                let mut oversized_envelope = a_value.clone();
                oversized_envelope.segments.reserve_exact(4096);
                assert_eq!(
                    metadata.enqueue_patch(
                        &a,
                        LinksPatch {
                            expected_old: None,
                            new: Some(oversized_envelope)
                        },
                        false
                    ),
                    Err(LinksAdmissionError::InvalidPatch)
                );
                fs::write(&native_a, b"admitted native effect").unwrap();
                let a_ticket = patch(&mut metadata, &a, None, Some(a_value));
                let base_old = current[0].clone();
                patch(&mut metadata, &deleting, Some(base_old), None);
                assert!(metadata.poll_result(&mut current).is_none());
                entered.recv_timeout(Duration::from_secs(5)).unwrap();
                assert_eq!(
                    metadata.reserve("b", 4096, b_growth),
                    Err(LinksAdmissionError::Capacity),
                    "held IO and an uncommitted deletion cannot donate capacity"
                );
                release.send(()).unwrap();
                replied.recv_timeout(Duration::from_secs(5)).unwrap();
                assert_eq!(
                    metadata.reserve("b", 4096, b_growth),
                    Err(LinksAdmissionError::Capacity),
                    "an unconsumed real result retains its growth credit"
                );
                let result = receipt(&mut metadata, &mut current);
                assert_eq!(result.ticket, a_ticket);
                if unknown {
                    assert!(matches!(result.outcome, LinksOutcome::DurabilityUnknown(_)));
                    assert_eq!(current.len(), 1);
                    assert_eq!(
                        metadata.reserve("b", 4096, b_growth),
                        Err(LinksAdmissionError::Capacity),
                        "quarantine is nonterminal and cannot donate capacity"
                    );
                    metadata.verify_uncertain().unwrap();
                    assert_eq!(
                        receipt(&mut metadata, &mut current).outcome,
                        LinksOutcome::Patched
                    );
                } else {
                    assert_eq!(result.outcome, LinksOutcome::Patched);
                }
                assert_eq!(current.len(), 2);
                assert!(native_a.exists());
                assert!(!native_b.exists());
                assert_eq!(repo.load_conversation_links().unwrap(), current);
                assert!(fs::read(&path).unwrap().len() <= capacity.file_bytes);
                assert!(
                    LinksGrowth::baseline(&current).fits(baseline.checked_add(a_growth).unwrap()),
                    "normalizing outer capacity must keep the admitted typed/file proof"
                );
                assert_eq!(
                    metadata.reserve("b", 4096, b_growth),
                    Err(LinksAdmissionError::Capacity),
                    "queued deletion has not committed its reclaimed space"
                );
                assert_eq!(
                    receipt(&mut metadata, &mut current).outcome,
                    LinksOutcome::Patched
                );
                assert_eq!(current.len(), 1);
                assert_eq!(current[0].id, "a");
                let b = metadata.reserve("b", 4096, b_growth).unwrap();
                metadata.release(&b).unwrap();
                close(&mut metadata);
            }
        }
    }

    #[cfg(unix)]
    fn opaque_limits() -> LinksLimits {
        LinksLimits {
            reservations: 4,
            patch_bytes: 128 * 1024,
            value_bytes: 256 * 1024,
            file_bytes: 128 * 1024,
            retained_bytes: 64 * 1024 * 1024,
        }
    }
    #[cfg(unix)]
    fn opaque_skeleton(id: &str, home: &Home) -> ConversationLink {
        let mut value = link(id, 1);
        value.segments[0].runtime_type_id = crate::config::RuntimeKind::Acp;
        value.segments[0].native_conversation_id = "synthetic:".into();
        value.segments[0].cwd = Some(home.0.to_string_lossy().into_owned());
        value
    }
    #[cfg(unix)]
    fn opaque_client(home: &Home, mode: &str, identity: &str) -> velune_agent_runtime::acp::Client {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&home.0, fs::Permissions::from_mode(0o700)).unwrap();
        let script = home.0.join("opaque.py");
        fs::write(
            home.0.join("opaque-id.json"),
            serde_json::to_vec(identity).unwrap(),
        )
        .unwrap();
        let source = r#"import json,sys,time,pathlib
root=pathlib.Path(ROOT)
mode=MODE
for line in sys.stdin:
 r=json.loads(line)
 with (root/'opaque-wire.jsonl').open('a') as f: f.write(json.dumps(r)+'\n')
 def emit(value): print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':value}),flush=True)
 if r.get('method')=='initialize': emit({'protocolVersion':1,'agentCapabilities':{}})
 elif r.get('method')=='session/new':
  (root/'opaque-native-created').touch()
  if mode=='hold':
   while not (root/'opaque-release').exists(): time.sleep(.001)
  if mode=='incomplete':
   sys.stdout.write('{"jsonrpc":"2.0","id":'+json.dumps(r['id'])+',"result":{"sessionId":"'+'x'*262144)
   sys.stdout.flush()
  else: emit({'sessionId':json.loads((root/'opaque-id.json').read_text())})
 else: emit({})
"#.replace("ROOT", &format!("{:?}", home.0.to_string_lossy())).replace("MODE", &format!("{mode:?}"));
        fs::write(&script, source).unwrap();
        let mut client =
            velune_agent_runtime::acp::Client::connect(velune_agent_runtime::acp::Config {
                binary: "/usr/bin/python3".into(),
                args: vec![script.to_string_lossy().into_owned()],
                environment: BTreeMap::new(),
                execution: velune_agent_runtime::execution::ExecutionTarget::Local,
                os_home: Some(home.0.clone()),
                mcp_servers: Vec::new(),
            })
            .unwrap();
        client.enable_command_port().unwrap();
        client
    }
    #[cfg(unix)]
    fn opaque_until(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate() {
            assert!(
                Instant::now() < deadline,
                "actual opaque helper/cleanup did not return"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[cfg(unix)]
    fn opaque_outcome(
        client: &mut velune_agent_runtime::acp::Client,
        ticket: &velune_agent_runtime::acp::AcpTicket,
    ) -> velune_agent_runtime::creation::CreationOutcome {
        let mut outcome = None;
        opaque_until(|| {
            let _ = client.poll();
            outcome = client.take_creation_result(ticket);
            outcome.is_some()
        });
        outcome.unwrap()
    }
    #[cfg(unix)]
    fn opaque_wire_counts(home: &Home) -> (usize, usize) {
        let values: Vec<serde_json::Value> = fs::read_to_string(home.0.join("opaque-wire.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        (
            values
                .iter()
                .filter(|v| v["method"] == "session/new")
                .count(),
            values
                .iter()
                .filter(|v| v["method"] == "session/prompt")
                .count(),
        )
    }
    #[cfg(unix)]
    fn opaque_prompt_refused(client: &mut velune_agent_runtime::acp::Client) -> bool {
        client
            .enqueue(
                velune_agent_runtime::acp::AcpCommand::Prompt {
                    text: "must wait for strict association".into(),
                    images: Vec::new(),
                    run_deadline: None,
                },
                Instant::now() + Duration::from_secs(5),
            )
            .is_err()
    }

    #[cfg(unix)]
    #[test]
    fn opaque_real_created_shrinks_credit_preserves_other_id_and_commits_complex_identity() {
        use velune_agent_runtime::creation::CreationOutcome;
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let mut current = vec![link("kept", 1)];
        repo.store_conversation_links(&current).unwrap(); // synthetic setup BEFORE domain claim
        let mut metadata = LinksMetadata::start(
            repo.home.clone(),
            repo.home_lock.claim_links().unwrap(),
            opaque_limits(),
            &current,
            Hooks::default(),
        )
        .unwrap();
        let other_new = link("kept", 2);
        let other = metadata
            .reserve("kept", 4096, LinksGrowth::for_new_link(Some(&other_new)))
            .unwrap();
        let mut new = opaque_skeleton("alias", &home);
        let opaque = metadata.reserve_opaque("alias", None, &new).unwrap();
        assert!(opaque.budget.limits().identity_bytes > 4096);
        assert_eq!(opaque.budget.peak_record_capacity(), 0);
        assert_eq!(
            metadata
                .reserve_opaque("second", None, &opaque_skeleton("second", &home))
                .err(),
            Some(LinksAdmissionError::Busy)
        );
        let identity = "名\"\\\n\0".repeat(768);
        assert!(identity.len() > 4096 && identity.len() <= opaque.budget.limits().identity_bytes);
        let mut client = opaque_client(&home, "hold", &identity);
        let budget = metadata.start_opaque_io(&opaque.reservation).unwrap();
        let native = client
            .enqueue_creation(
                home.0.clone(),
                "synthetic".into(),
                budget.clone(),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        opaque_until(|| home.0.join("opaque-native-created").exists());
        assert_eq!(
            budget.peak_record_capacity(),
            0,
            "max credit is not max-buffer allocation"
        );
        patch(
            &mut metadata,
            &other,
            Some(link("kept", 1)),
            Some(other_new),
        );
        assert_eq!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::Patched
        );
        assert_eq!(
            current,
            vec![link("kept", 2)],
            "other ID makes progress while native creation is held"
        );
        assert!(opaque_prompt_refused(&mut client));
        fs::write(home.0.join("opaque-release"), []).unwrap();
        let CreationOutcome::Created(created) = opaque_outcome(&mut client, &native) else {
            panic!("no actual identity");
        };
        assert_eq!(created.native_id(), identity);
        assert!(created.within_policy());
        new.segments[0].native_conversation_id = created.prefixed("synthetic:").unwrap();
        metadata
            .shrink_opaque(&opaque.reservation, &created, &new)
            .unwrap();
        assert!(opaque_prompt_refused(&mut client));
        opaque_until(|| {
            let _ = client.poll();
            client.settle_creation_result(&native).is_ok()
        });
        assert!(!budget.is_in_flight());
        assert!(
            opaque_prompt_refused(&mut client),
            "receive settlement is not file acknowledgement"
        );
        patch(&mut metadata, &opaque.reservation, None, Some(new.clone()));
        assert!(metadata.poll_result(&mut current).is_none());
        assert_eq!(
            current,
            vec![link("kept", 2)],
            "strict result is installed only at actual disk completion"
        );
        assert_eq!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::Patched
        );
        assert_eq!(current, vec![link("kept", 2), new]);
        assert_eq!(repo.load_conversation_links().unwrap(), current);
        client.complete_creation(&native).unwrap();
        assert_eq!(opaque_wire_counts(&home), (1, 0));
        let reused = metadata
            .reserve_opaque("second", None, &opaque_skeleton("second", &home))
            .unwrap();
        assert_eq!(reused.budget.peak_record_capacity(), 0);
        metadata.release(&reused.reservation).unwrap();
        close(&mut metadata);
        assert!(
            repo.unlock().is_err(),
            "live controlled native owner still retains HOME"
        );
        drop(client);
        opaque_until(|| repo.unlock().is_ok());
        assert!(Repository::lock_home(&home.0).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn opaque_actual_native_io_and_dropped_view_keep_home_and_credit_until_cleanup() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let mut metadata = LinksMetadata::start(
            repo.home.clone(),
            repo.home_lock.claim_links().unwrap(),
            opaque_limits(),
            &Vec::new(),
            Hooks::default(),
        )
        .unwrap();
        let opaque = metadata
            .reserve_opaque("alias", None, &opaque_skeleton("alias", &home))
            .unwrap();
        let mut client = opaque_client(&home, "hold", "eventual-identity");
        let budget = metadata.start_opaque_io(&opaque.reservation).unwrap();
        let ticket = client
            .enqueue_creation(
                home.0.clone(),
                "synthetic".into(),
                budget.clone(),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        opaque_until(|| home.0.join("opaque-native-created").exists());
        // Discard observation, NOT the accepted process/IO owner.
        drop(client.take_receipts());
        drop(opaque.budget);
        metadata.begin_close();
        assert_eq!(
            metadata.release(&opaque.reservation),
            Err(LinksAdmissionError::Busy)
        );
        assert!(!metadata.poll_drained());
        assert!(repo.unlock().is_err());
        drop(repo);
        assert!(Repository::lock_home(&home.0).is_err());
        assert!(client.take_creation_result(&ticket).is_none());
        assert!(budget.is_in_flight());
        assert_eq!(opaque_wire_counts(&home), (1, 0));
        drop(client); // actual child/read/result cleanup, no automatic retry
        opaque_until(|| !budget.is_in_flight());
        metadata.release(&opaque.reservation).unwrap();
        close(&mut metadata);
        assert!(!home.0.join("conversation-links.json").exists());
        assert!(Repository::lock_home(&home.0).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn opaque_policy_failure_retains_created_or_unknown_without_touching_original_association() {
        use velune_agent_runtime::creation::CreationOutcome;
        for incomplete in [false, true] {
            let home = Home::new();
            let (repo, _) = Repository::open(&home.0).unwrap();
            let mut current = vec![link("kept", 1)];
            repo.store_conversation_links(&current).unwrap();
            let original = fs::read(home.0.join("conversation-links.json")).unwrap();
            let mut metadata = LinksMetadata::start(
                repo.home.clone(),
                repo.home_lock.claim_links().unwrap(),
                opaque_limits(),
                &current,
                Hooks::default(),
            )
            .unwrap();
            let mut new = opaque_skeleton("alias", &home);
            let opaque = metadata.reserve_opaque("alias", None, &new).unwrap();
            let identity = "x".repeat(opaque.budget.limits().identity_bytes + 1);
            assert!(identity.len() + 256 < opaque.budget.limits().record_bytes);
            let mut client = opaque_client(
                &home,
                if incomplete { "incomplete" } else { "full" },
                &identity,
            );
            let budget = metadata.start_opaque_io(&opaque.reservation).unwrap();
            let native = client
                .enqueue_creation(
                    home.0.clone(),
                    "synthetic".into(),
                    budget.clone(),
                    Instant::now() + Duration::from_secs(5),
                )
                .unwrap();
            let outcome = opaque_outcome(&mut client, &native);
            if incomplete {
                assert!(matches!(outcome, CreationOutcome::Unknown { .. }));
            } else {
                let CreationOutcome::Created(created) = outcome else {
                    panic!("full identity must remain recoverable Created");
                };
                assert_eq!(created.native_id(), identity);
                assert!(!created.within_policy());
                new.segments[0].native_conversation_id = created.prefixed("synthetic:").unwrap();
                assert_eq!(
                    metadata.shrink_opaque(&opaque.reservation, &created, &new),
                    Err(LinksAdmissionError::CreatedOverBudget)
                );
            }
            assert!(metadata.poll_result(&mut current).is_none());
            assert_eq!(current, vec![link("kept", 1)]);
            assert_eq!(
                fs::read(home.0.join("conversation-links.json")).unwrap(),
                original
            );
            assert!(opaque_prompt_refused(&mut client));
            assert_eq!(opaque_wire_counts(&home), (1, 0));
            assert_eq!(
                metadata.release(&opaque.reservation),
                Err(LinksAdmissionError::Busy)
            );
            assert!(Repository::lock_home(&home.0).is_err());
            drop(client);
            opaque_until(|| !budget.is_in_flight());
            metadata.release(&opaque.reservation).unwrap();
            close(&mut metadata);
        }
    }

    #[cfg(unix)]
    #[test]
    fn opaque_fence_precedes_handing_out_policy_and_cannot_settle_an_undispatched_policy() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let mut current = Vec::new();
        let mut metadata = LinksMetadata::start(
            repo.home.clone(),
            repo.home_lock.claim_links().unwrap(),
            opaque_limits(),
            &current,
            Hooks::default(),
        )
        .unwrap();
        let opaque = metadata
            .reserve_opaque("alias", None, &opaque_skeleton("alias", &home))
            .unwrap();
        // The legitimate order still uses the actual worker and succeeds.
        metadata.enqueue_backup_fence(&opaque.reservation).unwrap();
        assert!(matches!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::BackupReady(None)
        ));
        let mut client = opaque_client(&home, "full", "must-not-be-created");
        let policy = metadata.start_opaque_io(&opaque.reservation).unwrap();
        assert!(
            !policy.is_in_flight(),
            "worker has policy but has not dispatched yet"
        );
        let bytes = br#"{"schemaVersion":99,"links":[]}"#;
        fs::write(home.0.join("conversation-links.json"), bytes).unwrap();
        let retained = metadata.retained;
        assert_eq!(
            metadata.enqueue_backup_fence(&opaque.reservation),
            Err(LinksAdmissionError::Busy)
        );
        assert!(metadata.poll_result(&mut current).is_none());
        assert_eq!(
            metadata.retained, retained,
            "rejected fence returns no SDK/growth credit"
        );
        assert_eq!(
            metadata
                .reserve_opaque("other", None, &opaque_skeleton("other", &home))
                .err(),
            Some(LinksAdmissionError::Busy)
        );
        assert_eq!(
            metadata.reserve("alias", 4096, LinksGrowth::default()),
            Err(LinksAdmissionError::Busy)
        );
        assert_eq!(
            fs::read(home.0.join("conversation-links.json")).unwrap(),
            bytes
        );
        // Only explicit cancellation can settle an unstarted policy; its CAS
        // also prevents the handed-out policy from starting after settlement.
        metadata.release(&opaque.reservation).unwrap();
        assert!(policy.is_settled());
        assert!(
            client
                .enqueue_creation(
                    home.0.clone(),
                    "synthetic".into(),
                    policy,
                    Instant::now() + Duration::from_secs(5)
                )
                .is_err()
        );
        assert_eq!(opaque_wire_counts(&home), (0, 0));
        drop(client);
        close(&mut metadata);
    }

    #[cfg(unix)]
    #[test]
    fn opaque_fence_cannot_return_credit_after_real_written_creation_with_held_response() {
        let home = Home::new();
        let (repo, _) = Repository::open(&home.0).unwrap();
        let mut current = Vec::new();
        let mut metadata = LinksMetadata::start(
            repo.home.clone(),
            repo.home_lock.claim_links().unwrap(),
            opaque_limits(),
            &current,
            Hooks::default(),
        )
        .unwrap();
        let opaque = metadata
            .reserve_opaque("alias", None, &opaque_skeleton("alias", &home))
            .unwrap();
        metadata.enqueue_backup_fence(&opaque.reservation).unwrap();
        assert!(matches!(
            receipt(&mut metadata, &mut current).outcome,
            LinksOutcome::BackupReady(None)
        ));
        let mut client = opaque_client(&home, "hold", "created-but-held");
        let policy = metadata.start_opaque_io(&opaque.reservation).unwrap();
        let ticket = client
            .enqueue_creation(
                home.0.clone(),
                "synthetic".into(),
                policy.clone(),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        opaque_until(|| home.0.join("opaque-native-created").exists());
        let mut written = false;
        opaque_until(|| {
            let _ = client.poll();
            written |= client.take_receipts().iter().any(|receipt| {
                matches!(
                    receipt.stage,
                    velune_agent_runtime::acp::AcpReceiptStage::Written
                )
            });
            written
        });
        let bytes = br#"{"schemaVersion":99,"links":[]}"#;
        fs::write(home.0.join("conversation-links.json"), bytes).unwrap();
        let retained = metadata.retained;
        assert_eq!(
            metadata.enqueue_backup_fence(&opaque.reservation),
            Err(LinksAdmissionError::Busy)
        );
        assert!(metadata.poll_result(&mut current).is_none());
        assert_eq!(metadata.retained, retained);
        assert_eq!(
            metadata
                .reserve_opaque("other", None, &opaque_skeleton("other", &home))
                .err(),
            Some(LinksAdmissionError::Busy)
        );
        assert_eq!(
            metadata.release(&opaque.reservation),
            Err(LinksAdmissionError::Busy)
        );
        assert!(policy.is_in_flight());
        assert!(client.take_creation_result(&ticket).is_none());
        assert_eq!(opaque_wire_counts(&home), (1, 0));
        metadata.begin_close();
        assert!(!metadata.poll_drained());
        assert!(repo.unlock().is_err());
        drop(client);
        opaque_until(|| !policy.is_in_flight());
        metadata.release(&opaque.reservation).unwrap();
        close(&mut metadata);
        assert_eq!(
            fs::read(home.0.join("conversation-links.json")).unwrap(),
            bytes
        );
    }
}
