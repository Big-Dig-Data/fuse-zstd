use fuser::FUSE_ROOT_ID;
use lru_time_cache::LruCache;
use std::{path::Path, time::Duration};

use crate::inode::Inode;

pub const TTL: Duration = Duration::from_secs(1); // dcache lifetime

pub struct InodeCache {
    new_inode_idx: u64,
    inode_db: LruCache<u128, Vec<u8>>,
}

impl InodeCache {
    pub fn new() -> Result<Self, libc::c_int> {
        Ok(Self {
            new_inode_idx: u64::MAX,
            inode_db: LruCache::with_expiry_duration(TTL + Duration::from_secs(1)),
        })
    }

    fn new_inode_number(&mut self) -> u64 {
        // 2 is the lowest usable FD
        if self.new_inode_idx - 1 <= FUSE_ROOT_ID {
            log::warn!("Fuse inodes counter rotated");
            // Note that this should never happen
            // fuse-zstd should be running for ceturies to achieve this...
            self.new_inode_idx = u64::MAX;
        }

        self.new_inode_idx -= 1;
        self.new_inode_idx
    }

    fn extract_data(data: &[u8]) -> (u64, u64, String) {
        (
            u64::from_be_bytes(data[0..8].to_vec().try_into().unwrap()),
            u64::from_be_bytes(data[8..16].to_vec().try_into().unwrap()),
            String::from_utf8_lossy(&data[16..]).to_string(),
        )
    }

    fn make_data(inode: Inode, value: &[u8]) -> Vec<u8> {
        inode
            .mount_point_inode
            .to_be_bytes()
            .into_iter()
            .chain(inode.data_dir_inode.to_be_bytes().into_iter())
            .chain(value.into_iter().map(|e| *e))
            .collect()
    }

    pub fn get_inode_path(&mut self, ino: Inode) -> Result<String, libc::c_int> {
        // Try to obtain both keys to hit the cache
        let data_mp = self
            .inode_db
            .get(&ino.mount_point_key())
            .map(|e| e.to_owned());
        let data_dd = self.inode_db.get(&ino.data_dir_key()).map(|e| e.to_owned());

        match (data_mp, data_dd) {
            (Some(data_mp), Some(data_dd)) => {
                if data_mp == data_dd {
                    let (_, _, data) = Self::extract_data(&data_mp);
                    Ok(data.to_owned())
                } else {
                    log::warn!("Inconsistent data for {:?}", ino);
                    // inconsistent data this should not happen
                    // clean both records
                    let (mp_ino, dd_ino, _) = Self::extract_data(&data_mp);
                    self.del_inode_path(Inode::new_dd(dd_ino));
                    self.del_inode_path(Inode::new_mp(mp_ino));

                    let (mp_ino, dd_ino, _) = Self::extract_data(&data_dd);
                    self.del_inode_path(Inode::new_dd(dd_ino));
                    self.del_inode_path(Inode::new_mp(mp_ino));

                    self.del_inode_path(ino);
                    Err(libc::ENOENT)
                }
            }
            (None, Some(data_dd)) => {
                let (mp_ino, _, data) = Self::extract_data(&data_dd);
                // hit the other part of the cache
                if self
                    .inode_db
                    .get(&Inode::new_mp(mp_ino).data_dir_key())
                    .is_none()
                {
                    // restore if not present
                    self.inode_db
                        .insert(Inode::new_mp(mp_ino).data_dir_key(), data_dd);
                }
                Ok(data.to_owned())
            }
            (Some(data_mp), None) => {
                let (_, dd_ino, data) = Self::extract_data(&data_mp);
                // hit the other part of the cache
                if self
                    .inode_db
                    .get(&Inode::new_dd(dd_ino).data_dir_key())
                    .is_none()
                {
                    // restore if not present
                    self.inode_db
                        .insert(Inode::new_dd(dd_ino).data_dir_key(), data_mp);
                }
                Ok(data.to_owned())
            }
            (None, None) => Err(libc::ENOENT),
        }
    }

    pub fn del_inode_path(&mut self, ino: Inode) {
        // remove inode - best effort
        if let Some(data) = self.inode_db.remove(&ino.mount_point_key()) {
            let (_, dd, _) = Self::extract_data(&data);
            self.inode_db.remove(&Inode::new_dd(dd).data_dir_key());
        }

        if let Some(data) = self.inode_db.remove(&ino.data_dir_key()) {
            let (mp, _, _) = Self::extract_data(&data);
            self.inode_db.remove(&Inode::new_mp(mp).mount_point_key());
        }
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

    pub fn set_inode_path<P, N>(&mut self, ino: Inode, path: P, name: N) -> Result<u64, libc::c_int>
    where
        P: AsRef<Path>,
        N: ToString,
    {
        if ino.data_dir_inode == 0 {
            // Data dir has to be defined
            return Err(libc::ENOENT);
        }

        let path_data = Self::make_path_str(path, name)?.as_bytes().to_vec();
        if ino.mount_point_inode == 0 {
            // we need to generate new one

            Ok(if let Some(data) = self.inode_db.get(&ino.data_dir_key()) {
                // Updating records
                let (dd_ino, mp_ino, _) = Self::extract_data(data);

                // Delete related records
                self.del_inode_path(Inode::new_mp(mp_ino));
                self.del_inode_path(Inode::new_dd(dd_ino));
                self.del_inode_path(ino);

                // Insert record
                let inode = Inode::new(Some(mp_ino), Some(ino.data_dir_inode));
                let data = Self::make_data(inode, &path_data);
                self.inode_db.insert(inode.data_dir_key(), data.clone());
                self.inode_db.insert(inode.mount_point_key(), data);
                inode.mount_point_inode
            } else {
                // Creating new one
                self.del_inode_path(ino);

                // Insert record
                let mp_ino = self.new_inode_number();
                let inode = Inode::new(Some(mp_ino), Some(ino.data_dir_inode));
                let data = Self::make_data(inode, &path_data);
                self.inode_db.insert(inode.data_dir_key(), data.clone());
                self.inode_db.insert(inode.mount_point_key(), data);
                inode.mount_point_inode
            })
        } else {
            // delete cache records
            self.del_inode_path(ino);
            let data = Self::make_data(ino, &path_data);
            self.inode_db.insert(ino.data_dir_key(), data.clone());
            self.inode_db.insert(ino.mount_point_key(), data);
            Ok(ino.mount_point_inode)
        }
    }

    pub fn get_data_dir_inode(&mut self, mount_point_inode: u64) -> Option<u64> {
        let data = self
            .inode_db
            .get(&Inode::new_mp(mount_point_inode).mount_point_key())?;
        let (_, dd_ino, _) = Self::extract_data(data);
        Some(dd_ino)
    }
}
