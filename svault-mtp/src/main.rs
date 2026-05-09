use crate::device::DeviceDescriptor;
use crate::error::Result;
use crate::mtp_watcher::{MtpEvent, MtpWatcher};
use crate::webdav::MtpWebDav;
use log::{error, info, warn};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};

mod device;
mod error;
mod ipc;
mod mtp_fs;
mod mtp_watcher;
mod webdav;

/// Tracks the state of a connected device.
struct DeviceState {
    descriptor: DeviceDescriptor,
    webdav: MtpWebDav,
}

/// The daemon's shared application state.
pub(crate) struct AppState {
    devices: Mutex<HashMap<String, DeviceState>>,
    next_port: Mutex<u16>,
}

impl AppState {
    pub(crate) fn new() -> Self {
        Self {
            devices: Mutex::new(HashMap::new()),
            next_port: Mutex::new(8090),
        }
    }

    async fn allocate_port(&self) -> u16 {
        let mut port = self.next_port.lock().await;
        let p = *port;
        *port += 1;
        p
    }

    pub(crate) async fn device_connected(
        &self,
        descriptor: DeviceDescriptor,
    ) -> Result<SocketAddr> {
        let port = self.allocate_port().await;
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port);
        let webdav = MtpWebDav::new(descriptor.device_key.clone(), addr);
        let key = descriptor.device_key.clone();
        self.devices.lock().await.insert(
            key,
            DeviceState { descriptor, webdav },
        );
        Ok(addr)
    }

    pub(crate) async fn device_disconnected(&self, device_key: &str) {
        self.devices.lock().await.remove(device_key);
    }

    pub(crate) async fn get_device_urls(&self) -> Vec<(String, String, SocketAddr)> {
        let devices = self.devices.lock().await;
        devices
            .iter()
            .map(|(key, state)| {
                (
                    key.clone(),
                    state.descriptor.to_string(),
                    state.webdav.addr(),
                )
            })
            .collect()
    }

    pub(crate) async fn get_dav_url(&self, device_key: &str) -> Option<SocketAddr> {
        self.devices
            .lock()
            .await
            .get(device_key)
            .map(|s| s.webdav.addr())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("svault-mtp daemon starting...");

    let state = Arc::new(AppState::new());
    let watcher = Arc::new(MtpWatcher::new(16));

    // Spawn IPC server in a blocking thread
    let ipc_state = Arc::clone(&state);
    let ipc_watcher = Arc::clone(&watcher);
    tokio::task::spawn_blocking(move || {
        if let Err(e) = ipc::serve(ipc_state, ipc_watcher) {
            error!("IPC server stopped: {e}");
        }
    });

    // Subscribe to device events
    let mut events = watcher.subscribe();
    let watch_state = Arc::clone(&state);
    let watch_handle = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(MtpEvent::Connected(descriptor)) => {
                    info!("Device connected: {descriptor}");
                    match watch_state.device_connected(descriptor).await {
                        Ok(addr) => {
                            info!("WebDAV available at http://{addr}");
                        }
                        Err(e) => error!("Failed to register device: {e}"),
                    }
                }
                Ok(MtpEvent::Disconnected(key)) => {
                    info!("Device disconnected: {key}");
                    watch_state.device_disconnected(&key).await;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Event receiver lagged by {n} messages");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Event channel closed, exiting device watcher");
                    break;
                }
            }
        }
    });

    // Start device watching (blocks until error)
    if let Err(e) = watcher.watch().await {
        error!("Device watcher error: {e}");
    }

    watch_handle.await.ok();
    info!("svault-mtp daemon stopped");
    Ok(())
}
