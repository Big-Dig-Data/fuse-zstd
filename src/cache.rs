use std::path::Path;

use sled;
use tempfile::TempDir;

use crate::errors::{convert_io_error, convert_sled_error};
use crate::Inode;

pub struct InodeCache {
    inode_dir: TempDir,
    inode_db: sled::Db,
}

impl InodeCache {
    pub fn new<P>(data_dir: P) -> Result<Self, libc::c_int>
    where
        P: AsRef<Path>,
    {
        let inode_dir = TempDir::new_in(data_dir).map_err(convert_io_error)?;
        let inode_db = sled::open(&inode_dir).map_err(convert_sled_error)?;
        Ok(Self {
            inode_dir,
            inode_db,
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
        let data = self
            .inode_db
            .get(&ino.to_be_bytes())
            .map(|e| e.to_owned())
            .map_err(convert_sled_error)?;
        match data {
            Some(data) => {
                let path = Self::extract_data(&data);
                Ok(path)
            }
            None => Err(libc::ENOENT),
        }
    }

    pub fn del_inode_path(&mut self, ino: Inode) -> Result<(), libc::c_int> {
        // remove inode - best effort
        self.inode_db
            .remove(&ino.to_be_bytes())
            .map_err(convert_sled_error)?;
        Ok(())
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
        Ok(self
            .inode_db
            .insert(ino.to_be_bytes(), data)
            .map_err(convert_sled_error)?
            .is_some())
    }

    pub fn cache_data_dir(&self) -> &tempfile::TempDir {
        &self.inode_dir
    }
}
