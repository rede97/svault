use dav_server::davpath::DavPath;
use dav_server::fs::{
    DavDirEntry, DavFile, DavFileSystem, DavMetaData, FsError, FsFuture, FsResult, FsStream,
    OpenOptions, ReadDirMeta,
};
use mtp_rs::mtp::MtpDevice;
use std::collections::HashMap;
use std::io::SeekFrom;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// A read-only filesystem backed by an MTP device.
///
/// Wraps `MtpDevice` (cheaply cloneable via internal `Arc`).
/// All read operations go through the MTP session; write operations
/// always return `FsError::Forbidden`.
pub struct MtpFs {
    device: MtpDevice,
    /// Cache of file content by path. MTP does not support random access,
    /// so files are downloaded once on first open and cached in memory.
    file_cache: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MtpFs {
    pub fn new(device: MtpDevice) -> Self {
        Self {
            device,
            file_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolve a path to an MTP ObjectHandle by walking the directory tree.
    async fn resolve_path(&self, path: &DavPath) -> FsResult<mtp_rs::ObjectHandle> {
        // TODO: Walk the MTP directory tree from root, resolving each
        // path component via storage.list_objects(parent).
        let _ = (path, &self.device);
        Err(FsError::NotFound)
    }

    /// List directory entries under the given parent handle.
    async fn list_dir_entries(
        &self,
        _parent: Option<mtp_rs::ObjectHandle>,
    ) -> FsResult<Vec<MtpDirEntry>> {
        // TODO: Call storage.list_objects(parent), convert ObjectInfo → MtpDirEntry.
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// DavFileSystem impl
// ---------------------------------------------------------------------------

impl DavFileSystem for MtpFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        Box::pin(async move {
            if options.write || options.append || options.create || options.create_new {
                return Err(FsError::Forbidden);
            }
            let _handle = self.resolve_path(path).await?;
            // TODO: download file from MTP into self.file_cache,
            // return an MtpFile backed by the cached bytes.
            Err(FsError::NotFound)
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        meta: ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        Box::pin(async move {
            let _ = (path, meta);
            let entries: Vec<FsResult<Box<dyn DavDirEntry>>> = self
                .list_dir_entries(None)
                .await?
                .into_iter()
                .map(|e| Ok(Box::new(e) as Box<dyn DavDirEntry>))
                .collect();
            Ok(Box::pin(futures::stream::iter(entries)) as FsStream<Box<dyn DavDirEntry>>)
        })
    }

    fn metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        Box::pin(async move {
            let _handle = self.resolve_path(path).await?;
            // TODO: get ObjectInfo, return MtpMetaData.
            Err(FsError::NotFound)
        })
    }
}

// ---------------------------------------------------------------------------
// MtpFile — read-only file backed by in-memory cache
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct MtpFile {
    data: Vec<u8>,
    cursor: u64,
    size: u64,
    mtime: SystemTime,
}

impl MtpFile {
    fn new(data: Vec<u8>, mtime: SystemTime) -> Self {
        let size = data.len() as u64;
        Self {
            data,
            cursor: 0,
            size,
            mtime,
        }
    }
}

impl DavFile for MtpFile {
    fn metadata(&'_ mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        Box::pin(std::future::ready(Ok(Box::new(MtpMetaData {
            size: self.size,
            modified: self.mtime,
            is_dir: false,
        }) as Box<dyn DavMetaData>)))
    }

    fn write_buf(&'_ mut self, _buf: Box<dyn bytes::Buf + Send>) -> FsFuture<'_, ()> {
        Box::pin(std::future::ready(Err(FsError::Forbidden)))
    }

    fn write_bytes(&'_ mut self, _buf: bytes::Bytes) -> FsFuture<'_, ()> {
        Box::pin(std::future::ready(Err(FsError::Forbidden)))
    }

    fn read_bytes(&'_ mut self, count: usize) -> FsFuture<'_, bytes::Bytes> {
        Box::pin(std::future::ready({
            let start = self.cursor as usize;
            let end = (start.saturating_add(count)).min(self.data.len());
            self.cursor = end as u64;
            Ok(bytes::Bytes::copy_from_slice(&self.data[start..end]))
        }))
    }

    fn seek(&'_ mut self, pos: SeekFrom) -> FsFuture<'_, u64> {
        Box::pin(std::future::ready({
            let new_pos = match pos {
                SeekFrom::Start(offset) => offset,
                SeekFrom::End(offset) => (self.size as i64).saturating_add(offset).max(0) as u64,
                SeekFrom::Current(offset) => {
                    (self.cursor as i64).saturating_add(offset).max(0) as u64
                }
            };
            self.cursor = new_pos.min(self.size);
            Ok(self.cursor)
        }))
    }

    fn flush(&'_ mut self) -> FsFuture<'_, ()> {
        Box::pin(std::future::ready(Ok(())))
    }
}

// ---------------------------------------------------------------------------
// MtpDirEntry
// ---------------------------------------------------------------------------

struct MtpDirEntry {
    name: Vec<u8>,
    size: u64,
    is_dir: bool,
    modified: SystemTime,
}

impl DavDirEntry for MtpDirEntry {
    fn name(&self) -> Vec<u8> {
        self.name.clone()
    }

    fn metadata(&'_ self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        Box::pin(std::future::ready(Ok(Box::new(MtpMetaData {
            size: self.size,
            modified: self.modified,
            is_dir: self.is_dir,
        }) as Box<dyn DavMetaData>)))
    }

    fn is_dir(&'_ self) -> FsFuture<'_, bool> {
        Box::pin(std::future::ready(Ok(self.is_dir)))
    }

    fn is_file(&'_ self) -> FsFuture<'_, bool> {
        Box::pin(std::future::ready(Ok(!self.is_dir)))
    }
}

// ---------------------------------------------------------------------------
// MtpMetaData
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MtpMetaData {
    size: u64,
    modified: SystemTime,
    is_dir: bool,
}

impl std::fmt::Debug for MtpMetaData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MtpMetaData")
            .field("size", &self.size)
            .field("modified", &self.modified)
            .field("is_dir", &self.is_dir)
            .finish()
    }
}

impl DavMetaData for MtpMetaData {
    fn len(&self) -> u64 {
        self.size
    }

    fn modified(&self) -> FsResult<SystemTime> {
        Ok(self.modified)
    }

    fn is_dir(&self) -> bool {
        self.is_dir
    }
}
