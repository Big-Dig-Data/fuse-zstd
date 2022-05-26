use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io,
    path::PathBuf,
};

use crate::Inode;

#[derive(Debug)]
pub struct OpenedFiles {
    mount_point_inode_mapping: HashMap<u64, HashSet<u64>>,
    handlers: HashMap<u64, FileHandler>,
}

#[derive(Debug, Clone)]
pub struct References {
    pub inode: Inode,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct FileHandler {
    pub flags: i32,
    pub needs_sync: bool,
    pub file: File,
    pub refs: Option<References>,
}

impl OpenedFiles {
    pub fn new() -> Self {
        Self {
            mount_point_inode_mapping: HashMap::new(),
            handlers: HashMap::new(),
        }
    }

    fn new_fh_number(&self) -> Option<u64> {
        for i in 0..=u64::MAX {
            if !self.handlers.contains_key(&i) {
                return Some(i);
            }
        }
        None
    }

    pub fn insert(&mut self, inode: Inode, flags: i32, file: File, path: PathBuf) -> Option<u64> {
        let new_fh = self.new_fh_number()?;

        let _ = self.handlers.insert(
            new_fh,
            FileHandler {
                file,
                flags,
                needs_sync: false,
                refs: Some(References { inode, path }),
            },
        );
        self.mount_point_inode_mapping
            .entry(inode)
            .or_insert_with(HashSet::new)
            .insert(new_fh);

        Some(new_fh)
    }

    pub fn duplicate(&mut self, inode: Inode, flags: i32) -> io::Result<Option<u64>> {
        let mapping = if let Some(mapping) = self.mount_point_inode_mapping.get(&inode) {
            mapping
        } else {
            return Ok(None);
        };

        let fh = mapping.iter().next().unwrap(); // mapping should not be empty
        let handler = self.handlers.get(fh).unwrap(); // should contain fh
        let new_fh = if let Some(new_fh) = self.new_fh_number() {
            new_fh
        } else {
            return Ok(None);
        };

        // Duplicate file
        let new_file = handler.file.try_clone()?;
        let new_handler = FileHandler {
            flags,
            needs_sync: false,
            file: new_file,
            refs: Some(References {
                inode,
                path: handler.refs.as_ref().unwrap().path.clone(),
            }),
        };

        // Update mappings and files
        let _ = self.handlers.insert(new_fh, new_handler);
        self.mount_point_inode_mapping
            .entry(inode)
            .or_insert_with(HashSet::new)
            .insert(new_fh);

        Ok(Some(new_fh))
    }

    pub fn close(&mut self, fh: u64) -> Option<FileHandler> {
        if let Some(handler) = self.handlers.remove(&fh) {
            if let Some(refs) = handler.refs.as_ref() {
                if let Some(mut mapping) = self.mount_point_inode_mapping.remove(&refs.inode) {
                    if mapping.remove(&fh) && !mapping.is_empty() {
                        self.mount_point_inode_mapping.insert(refs.inode, mapping);
                    }
                }
            }
            Some(handler)
        } else {
            None
        }
    }

    pub fn unlink(&mut self, ino: u64) -> Option<HashSet<u64>> {
        let handlers = self.mount_point_inode_mapping.remove(&ino)?;
        // Clear refs
        handlers.iter().for_each(|fh| {
            let handler = self.handlers.get_mut(fh).unwrap();
            handler.refs = None;
        });
        Some(handlers)
    }

    pub fn get(&self, fh: u64) -> Option<&FileHandler> {
        self.handlers.get(&fh)
    }

    pub fn get_mut(&mut self, fh: u64) -> Option<&mut FileHandler> {
        self.handlers.get_mut(&fh)
    }

    pub fn get_fhs_from_mount_point_inode(&self, ino: u64) -> Option<&HashSet<u64>> {
        self.mount_point_inode_mapping.get(&ino)
    }
}
