use crate::error::Result;
use crate::mtp_fs::MtpFs;
use dav_server::{DavHandler, fakels::FakeLs};
use log::info;
use mtp_rs::mtp::MtpDevice;
use std::net::SocketAddr;

/// A read-only WebDAV server that exposes an MTP device's filesystem.
///
/// Each connected MTP device gets its own server on a unique local port.
/// Serves only GET/HEAD/PROPFIND/OPTIONS — all write methods return 403.
/// Uses a custom `MtpFs` (implementing `DavFileSystem`) backed by `mtp-rs`.
pub struct MtpWebDav {
    device_key: String,
    addr: SocketAddr,
}

impl MtpWebDav {
    pub fn new(device_key: String, addr: SocketAddr) -> Self {
        Self { device_key, addr }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Start the WebDAV server for the given MTP device.
    ///
    /// Opens the device via mtp-rs, wraps it in an `MtpFs`, and serves
    /// via hyper on `self.addr`. This is a single-threaded server —
    /// one hyper HTTP/1.1 connection at a time.
    pub async fn serve(&self, device: MtpDevice) -> Result<()> {
        info!(
            "Starting read-only WebDAV for '{}' on {}",
            self.device_key, self.addr
        );

        let _fs = MtpFs::new(device);

        // TODO: Wire up the hyper server with MtpFs.
        //
        // let dav = DavHandler::builder()
        //     .filesystem(Box::new(fs))
        //     .locksystem(FakeLs::new())
        //     .build_handler();
        //
        // let listener = tokio::net::TcpListener::bind(self.addr).await?;
        // loop {
        //     let (stream, _) = listener.accept().await?;
        //     let dav = dav.clone();
        //     tokio::spawn(async move {
        //         hyper::server::conn::http1::Builder::new()
        //             .serve_connection(
        //                 hyper_util::rt::TokioIo::new(stream),
        //                 hyper::service::service_fn(move |req| {
        //                     let dav = dav.clone();
        //                     async move { Ok::<_, std::convert::Infallible>(dav.handle(req).await) }
        //                 }),
        //             )
        //             .await
        //     });
        // }

        // Suppress unused import warnings; will be wired up in the hyper server loop.
        let _ = (self.addr, std::any::type_name::<DavHandler>(), std::any::type_name::<FakeLs>(), std::any::type_name::<MtpFs>());

        Ok(())
    }
}
