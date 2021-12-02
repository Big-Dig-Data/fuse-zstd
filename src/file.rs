use std::{
    collections::{HashMap, HashSet},
    fs::File as FsFile,
    ops::{Deref, DerefMut},
};

pub struct File {
    file: FsFile,
    file_handlers: HashSet<u64>,
}

impl File {
    pub fn add_fh(&mut self, fh: u64) -> Option<usize> {
        if self.file_handlers.insert(fh) {
            Some(self.file_handlers.len())
        } else {
            None
        }
    }
    pub fn del_fh(&mut self, fh: u64) -> Option<usize> {
        if self.file_handlers.remove(&fh) {
            Some(self.file_handlers.len())
        } else {
            None
        }
    }
}

impl From<FsFile> for File {
    fn from(file: FsFile) -> Self {
        Self {
            file,
            file_handlers: HashSet::new(),
        }
    }
}

impl Into<FsFile> for File {
    fn into(self) -> FsFile {
        self.file
    }
}

impl Deref for File {
    type Target = FsFile;
    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl DerefMut for File {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.file
    }
}

pub struct FileHandlerData {
    pub flags: i32,
    pub needs_sync: bool,
}

pub struct FileHandlerManager {
    fh_data: HashMap<u64, FileHandlerData>,
}

impl FileHandlerManager {
    pub fn new() -> Self {
        Self {
            fh_data: HashMap::new(),
        }
    }

    pub fn insert(&mut self, item: FileHandlerData) -> Option<u64> {
        for i in 0..=u64::MAX {
            if !self.fh_data.contains_key(&i) {
                let _ = self.fh_data.insert(i, item);
                return Some(i);
            }
        }
        None // all file handlers were used
    }

    pub fn remove(&mut self, fh: u64) -> Option<FileHandlerData> {
        self.fh_data.remove(&fh)
    }

    pub fn get(&self, fh: u64) -> Option<&FileHandlerData> {
        self.fh_data.get(&fh)
    }

    pub fn get_mut(&mut self, fh: u64) -> Option<&mut FileHandlerData> {
        self.fh_data.get_mut(&fh)
    }
}
