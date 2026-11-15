//! Synchronizes temporary clipboard copies with the Windows history plugin.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
};

/// Acquire before issuing a temporary copy or deciding whether to store a captured update.
pub struct Suppression(File);

impl Suppression {
    pub fn acquire() -> io::Result<Self> {
        let directory = crate::dirs::root();
        std::fs::create_dir_all(&directory)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("clipboard-capture.lock"))?;
        file.lock()?;
        Ok(Self(file))
    }

    pub fn exclude(&mut self, sequence: u32) -> io::Result<()> {
        self.0.seek(SeekFrom::Start(0))?;
        self.0.write_all(&sequence.to_le_bytes())
    }

    pub fn excludes(&mut self, sequence: u32) -> io::Result<bool> {
        self.0.seek(SeekFrom::Start(0))?;
        let mut bytes = [0; 4];
        match self.0.read_exact(&mut bytes) {
            Ok(()) => Ok(u32::from_le_bytes(bytes) == sequence),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watcher_waits_and_skips_only_the_excluded_update() {
        let path = std::env::temp_dir().join(format!(
            "mytools-clipboard-suppression-{}.lock",
            std::process::id()
        ));
        let writer = File::create(&path).unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        writer.lock().unwrap();
        let mut writer = Suppression(writer);
        assert!(matches!(
            reader.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        writer.exclude(42).unwrap();
        drop(writer);

        reader.lock().unwrap();
        let mut reader = Suppression(reader);
        assert!(reader.excludes(42).unwrap());
        assert!(!reader.excludes(43).unwrap());
        drop(reader);
        std::fs::remove_file(path).unwrap();
    }
}
