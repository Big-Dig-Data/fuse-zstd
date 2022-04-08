#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Inode {
    /// inode number of the mount point
    pub mount_point_inode: u64,
    /// inode number on the host system
    pub data_dir_inode: u64,
}

impl Inode {
    pub fn new(mount_point_inode: Option<u64>, data_dir_inode: Option<u64>) -> Self {
        Self {
            mount_point_inode: mount_point_inode.unwrap_or_default(),
            data_dir_inode: data_dir_inode.unwrap_or_default(),
        }
    }

    pub fn new_mp(mount_point_inode: u64) -> Self {
        Self::new(Some(mount_point_inode), None)
    }

    pub fn new_dd(data_dir_inode: u64) -> Self {
        Self::new(None, Some(data_dir_inode))
    }

    pub fn mount_point_key(&self) -> u128 {
        u128::from(self.mount_point_inode) << 64
    }

    pub fn data_dir_key(&self) -> u128 {
        u128::from(self.data_dir_inode)
    }
}

impl From<u128> for Inode {
    fn from(key: u128) -> Self {
        Self::new(Some((key >> 64) as u64), Some(key as u64))
    }
}

impl Into<u128> for Inode {
    fn into(self) -> u128 {
        ((self.mount_point_inode as u128) << 64) | self.data_dir_inode as u128
    }
}
