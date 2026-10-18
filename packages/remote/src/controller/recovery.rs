use super::*;
use tokio::{sync::watch, task::JoinHandle};

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ControllerPhase {
    Disconnected,
    Suspended,
    Discovering,
    Connecting,
    Connected,
    Recovering,
    Blocked,
}

#[derive(Clone, Serialize)]
pub struct ControllerStatus {
    pub peer_id: String,
    pub phase: ControllerPhase,
    pub selected: bool,
    pub foreground: bool,
    pub paused: bool,
    pub intent_revision: u64,
    pub connection_epoch: u64,
    pub error_code: Option<String>,
}
struct Intent {
    selected: bool,
    foreground: bool,
    paused: bool,
    blocked: bool,
    revision: u64,
    phase: ControllerPhase,
    error: Option<RemoteError>,
}
pub(super) struct Recovery {
    intent: Mutex<Intent>,
    wake: watch::Sender<u64>,
    task: Mutex<Option<JoinHandle<()>>>,
}
impl Recovery {
    pub(super) fn new(paused: bool) -> Self {
        Self {
            intent: Mutex::new(Intent {
                selected: false,
                foreground: true,
                paused,
                blocked: false,
                revision: 0,
                phase: ControllerPhase::Disconnected,
                error: None,
            }),
            wake: watch::channel(0).0,
            task: Mutex::new(None),
        }
    }
}
impl Drop for Recovery {
    fn drop(&mut self) {
        if let Ok(task) = self.task.get_mut()
            && let Some(task) = task.take()
        {
            task.abort();
        }
    }
}
impl Controller {
    pub fn status(&self) -> Result<ControllerStatus> {
        let intent = self
            .recovery
            .intent
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        Ok(ControllerStatus {
            peer_id: self.peer.clone(),
            phase: intent.phase,
            selected: intent.selected,
            foreground: intent.foreground,
            paused: intent.paused,
            intent_revision: intent.revision,
            connection_epoch: self
                .connection
                .lock()
                .map_err(|_| RemoteError::Unavailable)?
                .epoch,
            error_code: intent.error.map(|error| error.code().into()),
        })
    }
    /// Explicit resume clears a user pause. Foreground/activity reports never clear it.
    pub async fn connect(self: &Arc<Self>) -> Result<()> {
        let owner = self.owner()?;
        let revision = {
            let access = owner.state.access()?;
            if access.closed {
                return Err(RemoteError::Closed);
            }
            owner.state.store.pause(&self.peer, false)?;
            let mut intent = self
                .recovery
                .intent
                .lock()
                .map_err(|_| RemoteError::Unavailable)?;
            intent.paused = false;
            intent.blocked = false;
            intent.selected = true;
            intent.error = None;
            intent.phase = ControllerPhase::Connecting;
            intent.revision = intent.revision.wrapping_add(1);
            self.recovery.wake.send_replace(intent.revision);
            intent.revision
        };
        self.start_recovery()?;
        self.connect_revision(revision).await
    }
    pub fn disconnect(&self) -> Result<()> {
        let owner = self.owner()?;
        let _access = owner.state.access()?;
        owner.state.store.pause(&self.peer, true)?;
        let mut intent = self
            .recovery
            .intent
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        intent.paused = true;
        intent.phase = ControllerPhase::Disconnected;
        intent.error = None;
        intent.revision = intent.revision.wrapping_add(1);
        if let Some(client) = self
            .connection
            .lock()
            .map_err(|_| RemoteError::Unavailable)?
            .client
            .take()
        {
            client.close();
        }
        self.recovery.wake.send_replace(intent.revision);
        Ok(())
    }
    /// Report one device's connection intent. Repeated facts do not create another recovery loop.
    pub fn set_activity(self: &Arc<Self>, selected: bool, foreground: bool) -> Result<()> {
        let owner = self.owner()?;
        let access = owner.state.access()?;
        if access.closed {
            return Err(RemoteError::Closed);
        }
        let mut intent = self
            .recovery
            .intent
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if intent.selected != selected || intent.foreground != foreground {
            intent.selected = selected;
            intent.foreground = foreground;
            intent.revision = intent.revision.wrapping_add(1);
            if intent.paused || !selected {
                intent.phase = ControllerPhase::Disconnected;
            } else if intent.blocked {
                intent.phase = ControllerPhase::Blocked;
            } else if !foreground {
                intent.phase = ControllerPhase::Suspended;
            } else if self.client().is_ok() {
                intent.phase = ControllerPhase::Connected;
            } else {
                intent.phase = ControllerPhase::Recovering;
            }
            self.recovery.wake.send_replace(intent.revision);
            owner.state.store.touch();
        }
        drop(intent);
        drop(access);
        self.start_recovery()
    }
    pub(crate) fn network_changed(&self) {
        if let Ok(mut intent) = self.recovery.intent.lock() {
            intent.revision = intent.revision.wrapping_add(1);
            self.recovery.wake.send_replace(intent.revision);
            if let Ok(owner) = self.owner() {
                owner.state.store.touch();
            }
        }
    }
    fn start_recovery(self: &Arc<Self>) -> Result<()> {
        let mut task = self
            .recovery
            .task
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if task.as_ref().is_none_or(JoinHandle::is_finished) {
            *task = Some(tokio::spawn(recover(
                Arc::downgrade(self),
                self.recovery.wake.subscribe(),
            )));
        }
        Ok(())
    }
    fn phase(&self, revision: u64, phase: ControllerPhase, error: Option<RemoteError>) {
        if let Ok(mut intent) = self.recovery.intent.lock()
            && intent.revision == revision
            && (intent.phase != phase || intent.error != error)
        {
            intent.phase = phase;
            if phase == ControllerPhase::Blocked {
                intent.blocked = true;
            }
            intent.error = error;
            if let Ok(owner) = self.owner() {
                owner.state.store.touch();
            }
        }
    }
    async fn connect_revision(&self, revision: u64) -> Result<()> {
        let _dial = self.dial.lock().await;
        {
            let intent = self
                .recovery
                .intent
                .lock()
                .map_err(|_| RemoteError::Unavailable)?;
            if intent.revision != revision || intent.paused {
                return Err(RemoteError::Closed);
            }
        }
        if self.client().is_ok() {
            self.phase(revision, ControllerPhase::Connected, None);
            return Ok(());
        }
        let owner = self.owner()?;
        let client = Arc::new(
            owner
                .connect_client(&self.peer, self.session.clone())
                .await?,
        );
        // The authenticated result and refreshed locator belong only to this still-current intent.
        let access = owner.state.access()?;
        let intent = self
            .recovery
            .intent
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if access.closed || intent.revision != revision || intent.paused {
            client.close();
            return Err(RemoteError::Closed);
        }
        let descriptor = client.descriptor();
        owner.state.store.update_locator(
            &self.peer,
            descriptor.name.clone(),
            DeviceLocator {
                relay_urls: descriptor.relay_urls.clone(),
                direct_addresses: descriptor.direct_addresses.clone(),
                lookup_urls: descriptor.lookup_urls.clone(),
            },
        )?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| RemoteError::Unavailable)?;
        if let Some(old) = connection.client.replace(client) {
            old.close();
        }
        connection.epoch = connection.epoch.wrapping_add(1);
        drop(connection);
        drop(intent);
        drop(access);
        self.phase(revision, ControllerPhase::Connected, None);
        Ok(())
    }
    async fn reconcile_pending(&self) -> Result<()> {
        let owner = self.owner()?;
        let pending = owner.state.store.snapshot()?.pending;
        drop(owner);
        for operation in pending.into_iter().filter(|operation| {
            operation.peer_id == self.peer
                && matches!(
                    operation.status,
                    PendingStatus::Recorded
                        | PendingStatus::Running
                        | PendingStatus::TransportFailed
                )
        }) {
            match self.query(operation.request_id).await {
                Ok(_) | Err(RemoteError::ServerRestarted) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}
async fn recover(controller: Weak<Controller>, mut wake: watch::Receiver<u64>) {
    let mut failures = 0u32;
    loop {
        let Some(current) = controller.upgrade() else {
            return;
        };
        let Ok(owner) = current.owner() else {
            return;
        };
        if !owner.state.access().is_ok_and(|access| !access.closed) {
            return;
        }
        drop(owner);
        let Ok(status) = current.status() else {
            return;
        };
        let active = status.selected
            && status.foreground
            && !status.paused
            && status.phase != ControllerPhase::Blocked;
        let revision = status.intent_revision;
        if active {
            let initial = current.client().is_err();
            current.phase(
                revision,
                if failures > 0 {
                    ControllerPhase::Recovering
                } else if initial {
                    ControllerPhase::Discovering
                } else {
                    ControllerPhase::Connected
                },
                None,
            );
            let result = tokio::select! {
                biased;
                changed=wake.changed()=>{ if changed.is_err(){return;} continue; },
                result=async { current.connect_revision(revision).await?; current.reconcile_pending().await }=>result,
            };
            match result {
                Ok(()) => failures = 0,
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let phase = if matches!(
                        error,
                        RemoteError::Unauthorized
                            | RemoteError::Protocol
                            | RemoteError::IncompatibleVersion
                    ) {
                        ControllerPhase::Blocked
                    } else {
                        ControllerPhase::Recovering
                    };
                    current.phase(revision, phase, Some(error));
                }
            }
        }
        // No strong Controller/service reference survives the wait; service teardown can end the loop.
        let jitter_seed = current.peer.bytes().fold(0u64, |seed, byte| {
            seed.wrapping_mul(31).wrapping_add(u64::from(byte))
        });
        drop(current);
        let delay = if active && failures > 0 {
            (1u64 << failures.saturating_sub(1).min(4)).min(15)
        } else {
            1
        };
        let jitter = if failures > 0 {
            jitter_seed.wrapping_add(u64::from(failures.wrapping_mul(73))) % 200
        } else {
            0
        };
        tokio::select! {
            _=tokio::time::sleep(Duration::from_millis((delay*1000+jitter).min(15_000)))=>{},
            changed=wake.changed()=>{ if changed.is_err(){return;} failures=0; },
        }
    }
}
