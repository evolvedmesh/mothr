//! Glue between the UI and the Mothr Remote server (runs on the tokio runtime).

use parking_lot::Mutex;
use mothr_core::{HubApi, RemoteSettings};
use mothr_hub::Hub;
pub use mothr_remote::{RemoteStatus, TunnelAvailability, detect_tunnels, generate_token, qr_matrix};
use std::sync::Arc;
use tokio::runtime::Handle;

#[derive(Clone)]
pub struct RemoteController {
    rt: Handle,
    hub: Arc<Hub>,
    server: Arc<Mutex<Option<mothr_remote::RemoteServer>>>,
    status: Arc<Mutex<RemoteStatus>>,
    pairing: Arc<Mutex<Option<String>>>,
    busy: Arc<Mutex<bool>>,
}

impl RemoteController {
    pub fn new(rt: Handle, hub: Arc<Hub>) -> Self {
        let this = Self {
            rt,
            hub: hub.clone(),
            server: Default::default(),
            status: Default::default(),
            pairing: Default::default(),
            busy: Default::default(),
        };
        let settings = hub.settings().remote;
        if settings.enabled {
            this.start(settings);
        }
        this
    }

    pub fn status(&self) -> RemoteStatus {
        self.status.lock().clone()
    }

    pub fn pairing_url(&self) -> Option<String> {
        self.pairing.lock().clone()
    }

    pub fn is_busy(&self) -> bool {
        *self.busy.lock()
    }

    pub fn start(&self, settings: RemoteSettings) {
        let this = self.clone();
        *self.busy.lock() = true;
        self.rt.spawn(async move {
            let old = this.server.lock().take();
            if let Some(old) = old {
                old.stop().await;
            }
            let hub: Arc<dyn HubApi> = this.hub.clone();
            match mothr_remote::RemoteServer::start(hub, settings).await {
                Ok(server) => {
                    let mut rx = server.subscribe_status();
                    *this.status.lock() = server.status();
                    *this.pairing.lock() = server.pairing_url();
                    *this.server.lock() = Some(server);
                    *this.busy.lock() = false;
                    while rx.changed().await.is_ok() {
                        let st = rx.borrow().clone();
                        *this.status.lock() = st;
                        let url = this.server.lock().as_ref().and_then(|s| s.pairing_url());
                        *this.pairing.lock() = url;
                    }
                }
                Err(e) => {
                    *this.status.lock() = RemoteStatus { error: Some(e.to_string()), ..Default::default() };
                    *this.busy.lock() = false;
                }
            }
        });
    }

    pub fn stop(&self) {
        let this = self.clone();
        *self.busy.lock() = true;
        self.rt.spawn(async move {
            let server = this.server.lock().take();
            if let Some(s) = server {
                s.stop().await;
            }
            *this.status.lock() = RemoteStatus::default();
            *this.pairing.lock() = None;
            *this.busy.lock() = false;
        });
    }
}
