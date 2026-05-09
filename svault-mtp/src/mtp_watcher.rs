use crate::device::DeviceDescriptor;
use crate::Result;
use log::{debug, error, info, warn};
use mtp_rs::mtp::MtpDevice;
use std::collections::HashSet;
use tokio::sync::broadcast;

/// Event emitted when MTP device state changes.
#[derive(Debug, Clone)]
pub enum MtpEvent {
    /// A new MTP device was connected.
    Connected(DeviceDescriptor),
    /// An MTP device was disconnected.
    Disconnected(String),
}

/// Watches for MTP device hotplug events via polling `MtpDevice::list_devices()`.
pub struct MtpWatcher {
    event_tx: broadcast::Sender<MtpEvent>,
}

impl MtpWatcher {
    /// Create a new watcher with the given event channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (event_tx, _) = broadcast::channel(capacity);
        Self { event_tx }
    }

    /// Subscribe to MTP device events.
    pub fn subscribe(&self) -> broadcast::Receiver<MtpEvent> {
        self.event_tx.subscribe()
    }

    /// Start watching for device events.
    ///
    /// Polls `MtpDevice::list_devices()` every 2 seconds, comparing against
    /// the known set of device keys. Emits `Connected` for new devices and
    /// `Disconnected` for removed ones.
    pub async fn watch(&self) -> Result<()> {
        info!("Starting MTP device watcher (polling every 2s)...");
        let mut known: HashSet<String> = HashSet::new();

        loop {
            if let Err(e) = self.poll_once(&mut known).await {
                error!("Device poll error: {e}");
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
    }

    async fn poll_once(&self, known: &mut HashSet<String>) -> Result<()> {
        let devices = match MtpDevice::list_devices() {
            Ok(d) => d,
            Err(e) => {
                debug!("list_devices: {e}");
                return Ok(());
            }
        };

        let current: HashSet<String> = devices
            .iter()
            .map(|d| format!("{:04x}:{:04x}:{}", d.vendor_id, d.product_id, d.location_id))
            .collect();

        // Devices that disappeared
        for key in known.difference(&current) {
            warn!("Device disconnected: {key}");
            let _ = self.event_tx.send(MtpEvent::Disconnected(key.clone()));
        }

        // Devices that appeared
        for info in devices {
            let desc: DeviceDescriptor = info.into();
            if known.insert(desc.device_key.clone()) {
                info!("Device connected: {desc}");
                let _ = self.event_tx.send(MtpEvent::Connected(desc));
            }
        }

        // Remove disconnected devices from known set
        known.retain(|k| current.contains(k));

        Ok(())
    }
}
