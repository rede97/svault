# CLAUDE.md

This file provides guidance to Claude Code when working within the `svault-mtp` sub-project. This crate is developed independently from the main svault workspace.

## Project overview

`svault-mtp` is a **standalone daemon process** (binary crate) that:
1. Detects MTP (Media Transfer Protocol) devices via `mtp-rs` (pure-Rust, no libmtp required) by polling `MtpDevice::list_devices()` every 2 seconds
2. Exposes each MTP device as a **single-threaded, read-only WebDAV minimal subset** on a local port, using a custom `MtpFs` (implementing `dav-server`'s `DavFileSystem` trait)
3. Exposes an **IPC socket** for the svault CLI to query connected devices and their WebDAV URLs

This allows svault to import media from phones and cameras via standard HTTP/WebDAV instead of talking MTP directly. No system dependencies — everything is pure Rust.

### WebDAV protocol — minimal read-only subset

The WebDAV server is intentionally minimal. It serves **only the methods required for read-only file browsing and download**:

| Method | Supported | Purpose |
|--------|-----------|---------|
| `GET` | yes | Download file content |
| `HEAD` | yes | Get file metadata without body |
| `PROPFIND` | yes | List directory contents and properties |
| `OPTIONS` | yes | Advertise supported methods |
| `PUT` | **no** | No write access to MTP devices |
| `DELETE` | **no** | No delete access |
| `MOVE` | **no** | No rename/move |
| `MKCOL` | **no** | No directory creation |
| `LOCK` / `UNLOCK` | **no** | No locking (fake lock system only, sufficient for macOS/Windows mount) |

The server runs in a **single hyper HTTP/1.1 connection at a time** per device — it does not spawn a thread pool or use multi-threaded serving. This keeps the daemon lightweight, matching the MTP USB throughput limit (typically ~30 MB/s), which is well below what a single-threaded HTTP server can saturate.

## Building and running

```bash
# Build
cargo build -p svault-mtp
cargo build -p svault-mtp --release

# Run (foreground, debug logging)
RUST_LOG=debug cargo run -p svault-mtp

# Run as background daemon
svault-mtp &
```

## Architecture

### Crate type

Binary crate (`[[bin]]` in Cargo.toml). Not a library — this is a long-running service process.

### Module map

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point: initialize IPC listener + device watcher + WebDAV registry, wire them together, handle signals |
| `mtp_watcher.rs` | Device hotplug detection via polling `MtpDevice::list_devices()` every 2s. Compares device keys against known set. Emits `MtpEvent::Connected` / `Disconnected` via `tokio::sync::broadcast`. |
| `mtp_fs.rs` | Custom `DavFileSystem` implementation (`MtpFs`) that wraps `mtp_rs::MtpDevice`. All write ops return `FsError::Forbidden`. Files are cached in memory on first open (MTP has no seek support). |
| `webdav.rs` | Wraps `dav-server` + `MtpFs` to serve an MTP device via hyper. Each device gets its own server on a unique local port. |
| `ipc.rs` | Unix-domain socket listener (`/tmp/svault-mtp.sock`). JSON-line protocol: request → response, one per connection. Commands: `ListDevices`, `GetDavUrl`, `Shutdown`. |
| `device.rs` | `DeviceDescriptor` — serializable device info wrapper around `mtp_rs::MtpDeviceInfo`. |
| `error.rs` | `MtpError` enum and `Result<T>` alias via `thiserror`. |

### Data flow

```
USB bus → MtpWatcher → broadcast channel → main (device registry)
                                              ├─→ MtpWebDav (serve on :port)
                                              └─→ IpcServer (answer queries)
```

### Runtime

Uses `tokio` (full features). The daemon runs three concurrent tasks:
1. **Device watcher** — monitors USB/udev for MTP device events
2. **WebDAV registry** — starts/stops per-device WebDAV servers as devices come and go. Each server is single-threaded (one hyper HTTP/1.1 connection at a time).
3. **IPC listener** — accepts local socket connections in a `spawn_blocking` thread, handles JSON-line queries synchronously

### IPC protocol

Socket path: `/tmp/svault-mtp.sock`

Request format (one JSON line):
```json
{"ListDevices": null}
{"GetDavUrl": {"device_id": "0x22b8:0x2e82:SN12345"}}
{"Shutdown": null}
```

Response format (one JSON line):
```json
{"DeviceList": [{"device_id": "...", "name": "...", "dav_url": "http://127.0.0.1:8080"}]}
{"DavUrl": {"device_id": "...", "url": "http://127.0.0.1:8080"}}
{"Ok": null}
{"Error": "message"}
```

## Key dependencies

| Crate | Purpose |
|--------|---------|
| `mtp-rs` | Pure-Rust MTP library. Used for device detection (`list_devices()`), file listing (`list_objects()`), and file download (`get_object()`). No system dependencies. |
| `dav-server` | WebDAV protocol handler (fork of `webdav-handler`). Uses custom `MtpFs` backend implementing `DavFileSystem` + `FakeLs` (fake lock system, sufficient for readonly). |
| `hyper` + `hyper-util` | HTTP/1.1 server. Required by dav-server to serve WebDAV over TCP. Single-connection per device. |
| `tokio` | Async runtime. Used for device polling, WebDAV serving, and coordinating concurrent tasks. |
| `futures` | `Stream` trait. Required by dav-server's `FsStream` type. |
| `serde` / `serde_json` | JSON serialization for IPC protocol messages. |
| `svault-core` | Reuses core types where applicable (not yet wired up). |

## Design invariants

- **Read-only WebDAV minimal subset** — only GET, HEAD, PROPFIND, OPTIONS. No write methods (PUT, DELETE, MOVE, MKCOL).
- **Single-threaded serving** — each device's WebDAV server uses one hyper HTTP/1.1 connection. No thread pool. MTP USB throughput (~30 MB/s) is the bottleneck, not HTTP.
- **One server per device** — each connected MTP device gets its own WebDAV server on a unique `127.0.0.1` port.
- **Sequential port allocation** — ports are allocated starting from 8090, incrementing by 1. Not port 0 (dynamic) to ensure predictable addressing for debugging.
- **No persistence** — the daemon holds no durable state. If it restarts, the svault CLI re-discovers devices via IPC.
- **Clean shutdown** — on SIGTERM/SIGINT, unmount all devices and remove the socket file.

## Pending implementation (TODOs)

1. **MTP tree walking** — `mtp_fs.rs` has the `DavFileSystem` trait fully scaffolded but `resolve_path()` and `list_dir_entries()` are stubs. Need to:
   - Implement `resolve_path()` by walking the MTP directory tree from root, resolving each path component via `storage.list_objects(parent)`
   - Implement `list_dir_entries()` by calling `storage.list_objects(parent)` and converting `ObjectInfo` → `MtpDirEntry`
   - Implement file download in `open()`: call `storage.get_object(handle)`, stream content from USB into `self.file_cache`

2. **WebDAV server wiring** — `webdav.rs` `serve()` has the `MtpFs` + `DavHandler` scaffolding but is not yet connected to the hyper server loop. The TODO comment shows the intended implementation.

3. **Graceful shutdown** — the `Shutdown` IPC command needs a `tokio::sync::oneshot` channel to signal the main loop to stop.

4. **Cross-platform IPC** — currently using `std::os::unix::net::UnixListener`. The `interprocess` dependency is commented out in Cargo.toml for future migration to support Windows named pipes.

## Relevant parent project context

- `svault-core` provides the root project's types (hash, db, fs, pipeline, etc.). `svault-mtp` may use core types but is otherwise self-contained.
- The main svault CLI (`svault-cli`) will be the client of the IPC socket — it queries for devices and reads files via the WebDAV URLs.
- Both crates live in the same Cargo workspace.
