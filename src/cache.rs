use lru_time_cache::LruCache;
use std::{path::Path, time::Duration};

use crate::Inode;

pub const TTL: Duration = Duration::from_secs(1); // dcache lifetime

/// Max dcache capacity how many directories can be opened
/// without loosing inode info
pub const CAPACITY: usize = 10_000; // MAX inode cache capacity

pub struct InodeCache {
    inode_db: LruCache<Inode, Vec<u8>>,
}

impl InodeCache {
    pub fn new() -> Result<Self, libc::c_int> {
        Ok(Self {
            inode_db: LruCache::with_capacity(CAPACITY),
        })
    }

    fn extract_data(data: &[u8]) -> String {
        String::from_utf8_lossy(&data[8..]).to_string()
    }

    fn make_data(inode: Inode, value: &[u8]) -> Vec<u8> {
        inode
            .to_be_bytes()
            .into_iter()
            .chain(value.into_iter().map(|e| *e))
            .collect()
    }

    pub fn get_inode_path(&mut self, ino: Inode) -> Result<String, libc::c_int> {
        let data = self.inode_db.get(&ino).map(|e| e.to_owned());
        match data {
            Some(data) => {
                let path = Self::extract_data(&data);
                Ok(path)
            }
            None => Err(libc::ENOENT),
        }
    }

    pub fn del_inode_path(&mut self, ino: Inode) {
        // remove inode - best effort
        self.inode_db.remove(&ino);
    }

    fn make_path_str<P, N>(path: P, name: N) -> Result<String, libc::c_int>
    where
        P: AsRef<Path>,
        N: ToString,
    {
        let path: &Path = path.as_ref();
        let path_str = path.to_string_lossy();
        let name = name.to_string();
        Ok(match (&path_str, &name) {
            (p, n) if !p.is_empty() && !n.is_empty() => {
                format!("{}/{}", p, n)
            }
            (p, n) if p.is_empty() && !n.is_empty() => n.to_string(),
            (p, n) if !p.is_empty() && n.is_empty() => p.to_string(),
            _ => return Err(libc::EIO),
        })
    }

    pub fn set_inode_path<P, N>(
        &mut self,
        ino: Inode,
        path: P,
        name: N,
    ) -> Result<bool, libc::c_int>
    where
        P: AsRef<Path>,
        N: ToString,
    {
        let path_data = Self::make_path_str(path, name)?.as_bytes().to_vec();
        let data = Self::make_data(ino, &path_data);
        Ok(self.inode_db.insert(ino, data).is_some())
    }
}
