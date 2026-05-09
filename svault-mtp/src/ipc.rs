use crate::error::Result;
use crate::mtp_watcher::MtpWatcher;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::Arc;

/// Socket path for svault CLI ↔ svault-mtp daemon communication.
pub(crate) const SOCKET_PATH: &str = "/tmp/svault-mtp.sock";

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum IpcRequest {
    ListDevices,
    GetDavUrl { device_id: String },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum IpcResponse {
    DeviceList(Vec<DeviceEntry>),
    DavUrl { device_id: String, url: String },
    Ok,
    Error(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DeviceEntry {
    pub device_id: String,
    pub name: String,
    pub dav_url: String,
}

use crate::AppState;

/// Start the IPC listener loop. Blocks the calling thread.
pub(crate) fn serve(state: Arc<AppState>, _watcher: Arc<MtpWatcher>) -> Result<()> {
    let _ = std::fs::remove_file(SOCKET_PATH);

    let listener = UnixListener::bind(SOCKET_PATH)
        .map_err(|e| crate::error::MtpError::IpcError(format!("bind failed: {e}")))?;

    info!("IPC server listening on {SOCKET_PATH}");

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(e) = handle_connection(&mut stream, &state) {
                    error!("IPC connection error: {e}");
                }
            }
            Err(e) => {
                error!("Failed to accept IPC connection: {e}");
            }
        }
    }

    Ok(())
}

fn handle_connection(
    stream: &mut std::os::unix::net::UnixStream,
    state: &AppState,
) -> Result<()> {
    let mut reader = BufReader::new(&mut *stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let request: IpcRequest = serde_json::from_str(line.trim())
        .map_err(|e| crate::error::MtpError::IpcError(format!("bad request: {e}")))?;

    let response = match request {
        IpcRequest::ListDevices => {
            let devices = tokio::runtime::Handle::current().block_on(state.get_device_urls());
            let entries: Vec<DeviceEntry> = devices
                .into_iter()
                .map(|(id, name, addr)| DeviceEntry {
                    device_id: id,
                    name,
                    dav_url: format!("http://{addr}"),
                })
                .collect();
            IpcResponse::DeviceList(entries)
        }
        IpcRequest::GetDavUrl { device_id } => {
            match tokio::runtime::Handle::current().block_on(state.get_dav_url(&device_id)) {
                Some(addr) => IpcResponse::DavUrl {
                    device_id,
                    url: format!("http://{addr}"),
                },
                None => IpcResponse::Error(format!("device not found: {device_id}")),
            }
        }
        IpcRequest::Shutdown => {
            info!("Received shutdown request via IPC");
            IpcResponse::Ok
        }
    };

    let json = serde_json::to_string(&response)?;
    writeln!(reader.into_inner(), "{json}")?;

    Ok(())
}
