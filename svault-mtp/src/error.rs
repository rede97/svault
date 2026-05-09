use thiserror::Error;

#[derive(Error, Debug)]
pub enum MtpError {
    #[error("no MTP device found")]
    NoDevice,

    #[error("failed to connect to device: {0}")]
    ConnectionFailed(String),

    #[error("device communication error: {0}")]
    CommunicationError(String),

    #[error("file not found on device: {0}")]
    FileNotFound(String),

    #[error("device disconnected")]
    DeviceDisconnected,

    #[error("WebDAV server error: {0}")]
    WebDavError(String),

    #[error("IPC error: {0}")]
    IpcError(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Serde(#[from] serde_json::Error),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, MtpError>;
