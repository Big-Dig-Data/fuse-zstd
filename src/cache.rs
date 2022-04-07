#[cfg(not(feature = "with_disk_inode_cache"))]
use lru_time_cache::LruCache;
#[cfg(feature = "with_disk_inode_cache")]
use sled::Db;
use std::{path::Path, time::Duration};

#[cfg(feature = "with_disk_inode_cache")]
use super::convert_io_error;

pub const TTL: Duration = Duration::from_secs(1); // dcache lifetime

#[cfg(feature = "with_disk_inode_cache")]
fn convert_sled_error(err: sled::Error) -> libc::c_int {
    match err {
        sled::Error::Io(ioerror) => convert_io_error(ioerror),
        _ => libc::EIO,
    }
}

pub struct InodeCache {
    #[cfg(feature = "with_disk_inode_cache")]
    inode_db: Db,
    #[cfg(not(feature = "with_disk_inode_cache"))]
    inode_db: LruCache<u64, String>,
    #[cfg(feature = "with_disk_inode_cache")]
    #[allow(dead_code)]
    inode_dir: tempfile::TempDir,
}

impl InodeCache {
    pub fn new() -> Result<Self, libc::c_int> {
        #[cfg(feature = "with_disk_inode_cache")]
        let inode_dir = tempfile::tempdir().map_err(convert_io_error)?;
        Ok(Self {
            #[cfg(not(feature = "with_disk_inode_cache"))]
            inode_db: LruCache::with_expiry_duration(TTL + Duration::from_secs(1)),

            #[cfg(feature = "with_disk_inode_cache")]
            inode_db: sled::open(&inode_dir).map_err(convert_sled_error)?,
            #[cfg(feature = "with_disk_inode_cache")]
            inode_dir: inode_dir,
        })
    }

    #[cfg(feature = "with_disk_inode_cache")]
    pub fn get_inode_path(&self, ino: u64) -> Result<String, libc::c_int> {
        let inodes = self
            .inode_db
            .open_tree("inodes")
            .map_err(convert_sled_error)?;

        if let Some(path) = inodes.get(ino.to_be_bytes()).map_err(convert_sled_error)? {
            Ok(String::from_utf8_lossy(&path).to_string())
        } else {
            Err(libc::ENOENT)
        }
    }

    #[cfg(not(feature = "with_disk_inode_cache"))]
    pub fn get_inode_path(&mut self, ino: u64) -> Result<String, libc::c_int> {
        Ok(self.inode_db.get(&ino).ok_or(libc::ENOENT)?.to_owned())
    }

    #[cfg(feature = "with_disk_inode_cache")]
    pub fn del_inode_path(&mut self, ino: u64) -> Result<String, libc::c_int> {
        let inodes = self
            .inode_db
            .open_tree("inodes")
            .map_err(convert_sled_error)?;

        if let Some(path) = inodes
            .remove(ino.to_be_bytes())
            .map_err(convert_sled_error)?
        {
            Ok(String::from_utf8_lossy(&path).to_string())
        } else {
            Err(libc::ENOENT)
        }
    }

    #[cfg(not(feature = "with_disk_inode_cache"))]
    pub fn del_inode_path(&mut self, ino: u64) -> Result<String, libc::c_int> {
        self.inode_db.remove(&ino).ok_or(libc::ENOENT)
    }

    #[cfg(feature = "with_disk_inode_cache")]
    pub fn set_inode_path<P, N>(&mut self, ino: u64, path: P, name: N) -> Result<(), libc::c_int>
    where
        P: AsRef<Path>,
        N: ToString,
    {
        let inodes = self
            .inode_db
            .open_tree("inodes")
            .map_err(convert_sled_error)?;
        let path: &Path = path.as_ref();
        let path_str = path.to_string_lossy();
        let name = name.to_string();
        let value = match (&path_str, &name) {
            (p, n) if !p.is_empty() && !n.is_empty() => {
                format!("{}/{}", p, n)
            }
            (p, n) if p.is_empty() && !n.is_empty() => n.to_string(),
            (p, n) if !p.is_empty() && n.is_empty() => p.to_string(),
            _ => return Err(libc::EIO),
        };
        inodes
            .insert(ino.to_be_bytes(), value.as_bytes())
            .map_err(convert_sled_error)
            .map(|_| ())
    }

    #[cfg(not(feature = "with_disk_inode_cache"))]
    pub fn set_inode_path<P, N>(&mut self, ino: u64, path: P, name: N) -> Result<(), libc::c_int>
    where
        P: AsRef<Path>,
        N: ToString,
    {
        let path: &Path = path.as_ref();
        let path_str = path.to_string_lossy();
        let name = name.to_string();
        let value = match (&path_str, &name) {
            (p, n) if !p.is_empty() && !n.is_empty() => {
                format!("{}/{}", p, n)
            }
            (p, n) if p.is_empty() && !n.is_empty() => n.to_string(),
            (p, n) if !p.is_empty() && n.is_empty() => p.to_string(),
            _ => return Err(libc::EIO),
        };
        let _ = self.inode_db.insert(ino, value);
        Ok(())
    }
}
