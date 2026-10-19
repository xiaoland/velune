//! The remote service's state, shared by every window.
use std::time::Duration;

use gpui_kit::Context;
use velune_bindings::{
    BindingContactConfirmation, BindingDeviceContact, BindingError, BindingRemoteServiceSnapshot,
};

use crate::core::{Core, Device, describe};

pub struct Hub {
    pub core: Core,
    pub state: Option<BindingRemoteServiceSnapshot>,
    pub failure: Option<String>,
}

impl Hub {
    pub fn new(core: Core, cx: &mut Context<Self>) -> Self {
        let opening = core.clone();
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let opened = executor
                .spawn(async move {
                    let endpoint = opening.open_endpoint()?;
                    let first = endpoint.read_state()?;
                    Ok::<_, BindingError>((endpoint, first))
                })
                .await;
            let (endpoint, first) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    let _ = this.update(cx, |hub, cx| {
                        hub.failure = Some(format!("远程控制服务无法启动：{}", describe(&error)));
                        cx.notify();
                    });
                    return;
                }
            };
            if this
                .update(cx, |hub, cx| {
                    hub.state = Some(first);
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            let observer = endpoint.observe_state();
            loop {
                let observer = observer.clone();
                let next = executor.spawn(async move { observer.next(5_000) }).await;
                let snapshot = match next {
                    Ok(Some(snapshot)) => snapshot,
                    Ok(None) => continue,
                    Err(BindingError::Closed) => return,
                    Err(_) => {
                        executor.timer(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                let applied = this.update(cx, |hub, cx| {
                    hub.state = Some(snapshot);
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
        Self {
            core,
            state: None,
            failure: None,
        }
    }

    /// Paired devices this PC can open: the issuing device has confirmed them.
    pub fn devices(&self) -> Vec<BindingDeviceContact> {
        self.state
            .iter()
            .flat_map(|state| state.contacts.iter())
            .filter(|contact| {
                matches!(
                    contact.confirmation,
                    BindingContactConfirmation::Confirmed { .. }
                )
            })
            .cloned()
            .collect()
    }

    pub fn device_name(&self, device: &Device) -> String {
        match device {
            Device::Local => self
                .state
                .as_ref()
                .and_then(|state| state.descriptor.as_ref())
                .map(|descriptor| descriptor.name.clone())
                .unwrap_or_else(crate::core::device_name),
            Device::Remote(peer) => self
                .state
                .iter()
                .flat_map(|state| state.contacts.iter())
                .find(|contact| &contact.peer_id == peer)
                .map(|contact| contact.name.clone())
                .unwrap_or_else(|| "设备".into()),
        }
    }

    /// Tells the core which device the user looks at, so it keeps that
    /// connection ready.
    pub fn set_activity(&self, previous: &Device, current: &Device, foreground: bool) {
        let Ok(endpoint) = self.core.endpoint() else {
            return;
        };
        let previous = previous.clone();
        let current = current.clone();
        std::thread::spawn(move || {
            if let Device::Remote(peer) = &previous
                && previous != current
            {
                let _ = endpoint.set_controller_activity(peer.clone(), false, false);
            }
            if let Device::Remote(peer) = current {
                let _ = endpoint.set_controller_activity(peer, true, foreground);
            }
        });
    }
}
