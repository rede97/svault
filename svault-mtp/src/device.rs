//! Device type wrappers around mtp-rs.
//!
//! These provide serde serialization and Display impls for IPC.

use serde::Serialize;

/// Descriptor for an available (not yet opened) MTP device.
///
/// Wraps `mtp_rs::MtpDeviceInfo` with serde support for IPC.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceDescriptor {
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub location_id: u64,
    /// Unique key: "vid:pid:location"
    pub device_key: String,
}

impl From<mtp_rs::mtp::MtpDeviceInfo> for DeviceDescriptor {
    fn from(info: mtp_rs::mtp::MtpDeviceInfo) -> Self {
        let device_key = format!(
            "{:04x}:{:04x}:{}",
            info.vendor_id, info.product_id, info.location_id
        );
        Self {
            vendor_id: info.vendor_id,
            product_id: info.product_id,
            manufacturer: info.manufacturer,
            product: info.product,
            serial_number: info.serial_number,
            location_id: info.location_id,
            device_key,
        }
    }
}

impl std::fmt::Display for DeviceDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self
            .product
            .as_deref()
            .unwrap_or(self.manufacturer.as_deref().unwrap_or("Unknown device"));
        write!(f, "{name} [{}]", self.device_key)
    }
}
