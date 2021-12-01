use std::{
    fs::File as FsFile,
    ops::{Deref, DerefMut},
};

pub struct File {
    file: FsFile,
    ref_count: usize,
}

impl File {
    pub fn inc(&mut self) -> usize {
        self.ref_count += 1;
        self.ref_count
    }
    pub fn dec(&mut self) -> usize {
        self.ref_count -= 1;
        self.ref_count
    }
}

impl From<FsFile> for File {
    fn from(file: FsFile) -> Self {
        Self { file, ref_count: 1 }
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
