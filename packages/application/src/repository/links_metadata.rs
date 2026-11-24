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
mod tests;
