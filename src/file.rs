use log::debug;
use lru_time_cache::LruCache;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    rc::Rc,
    time::Duration,
};

pub struct OpenedFiles {
    inode_mapping: HashMap<u64, HashSet<u64>>,
    handlers: HashMap<u64, FileHandler>,
}

#[derive(Clone)]
pub struct FileHandler {
    pub flags: i32,
    pub needs_sync: bool,
    pub file: Rc<File>,
    pub ino: Option<u64>,
}

impl OpenedFiles {
    pub fn new() -> Self {
        Self {
            inode_mapping: HashMap::new(),
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

    pub fn update_ino(&mut self, old_ino: u64, new_ino: u64) -> Option<usize> {
        let fhs = self.inode_mapping.remove(&old_ino)?;
        let len = fhs.len();
        for fh in &fhs {
            let handler = self.handlers.get_mut(fh).unwrap();
            handler.ino = Some(new_ino);
        }
        self.inode_mapping
            .entry(new_ino)
            .or_insert_with(HashSet::new)
            .extend(fhs);

        Some(len)
    }

    pub fn insert(&mut self, ino: u64, flags: i32, file: File) -> Option<u64> {
        let new_fh = self.new_fh_number()?;

        let _ = self.handlers.insert(
            new_fh,
            FileHandler {
                file: Rc::new(file),
                flags,
                needs_sync: false,
                ino: Some(ino),
            },
        );
        self.inode_mapping
            .entry(ino)
            .or_insert_with(HashSet::new)
            .insert(new_fh);

        Some(new_fh)
    }

    pub fn duplicate(&mut self, ino: u64, flags: i32) -> Option<u64> {
        let mapping = self.inode_mapping.get(&ino)?;
        let fh = mapping.iter().next().unwrap(); // mapping should not be empty
        let handler = self.handlers.get(fh).unwrap(); // should contain fh
        let new_fh = self.new_fh_number()?;
        let new_handler = FileHandler {
            flags,
            needs_sync: handler.needs_sync,
            file: handler.file.clone(),
            ino: Some(ino),
        };
        let _ = self.handlers.insert(new_fh, new_handler);
        Some(new_fh)
    }

    pub fn close(&mut self, fh: u64) -> Option<FileHandler> {
        if let Some(handler) = self.handlers.remove(&fh) {
            if let Some(ino) = handler.ino {
                if let Some(mut mapping) = self.inode_mapping.remove(&ino) {
                    if mapping.remove(&fh) && !mapping.is_empty() {
                        self.inode_mapping.insert(ino, mapping);
                    }
                }
            }
            Some(handler)
        } else {
            None
        }
    }

    pub fn unlink(&mut self, ino: u64) -> Option<HashSet<u64>> {
        let handlers = self.inode_mapping.remove(&ino)?;
        // Set ino to None in handlers
        handlers
            .iter()
            .for_each(|fh| self.handlers.get_mut(fh).unwrap().ino = None);
        Some(handlers)
    }

    pub fn get(&self, fh: u64) -> Option<&FileHandler> {
        self.handlers.get(&fh)
    }

    pub fn get_mut(&mut self, fh: u64) -> Option<&mut FileHandler> {
        self.handlers.get_mut(&fh)
    }

    pub fn get_fhs(&self, ino: u64) -> Option<&HashSet<u64>> {
        self.inode_mapping.get(&ino)
    }
}

const TTL: u64 = 5000; // default 5 seconds dcache of kernel should be 1 second

struct OverridenInode {
    pub ino_successor: u64,
    pub file: File, // keep opened file to preserve inode so it is not reused in original fs
}

pub struct OverridenInodes {
    mapping: LruCache<u64, OverridenInode>,
}

impl OverridenInodes {
    pub fn new() -> Self {
        Self {
            mapping: LruCache::with_expiry_duration(Duration::from_millis(TTL)),
        }
    }

    pub fn get(&mut self, ino: u64) -> Option<u64> {
        if let Some(oi) = self.mapping.get(&ino) {
            debug!("Overriden inode {} -> {}", ino, oi.ino_successor);
            Some(oi.ino_successor)
        } else {
            None
        }
    }

    pub fn insert(&mut self, old_ino: u64, new_ino: u64, file: File) {
        // Update exsiting mappings so no chain created
        let keys = self
            .mapping
            .peek_iter()
            .map(|(k, _)| *k)
            .filter(|e| *e == old_ino)
            .collect::<Vec<u64>>();
        for key in keys {
            self.mapping.get_mut(&key).unwrap().ino_successor = new_ino;
        }
        self.mapping.insert(
            old_ino,
            OverridenInode {
                file,
                ino_successor: new_ino,
            },
        );
    }

    pub fn remove(&mut self, ino: u64) {
        let keys = self
            .mapping
            .peek_iter()
            .filter_map(|(k, v)| {
                if v.ino_successor == ino {
                    Some(*k)
                } else {
                    None
                }
            })
            .collect::<Vec<u64>>();

        for key in keys {
            self.mapping.remove(&key).unwrap();
        }

        self.mapping.remove(&ino);
    }
}
