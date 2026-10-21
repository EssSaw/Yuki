use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Holds directory handles against rename and an exclusive target handle.
/// Replacement is deliberately in-place: recovery records cover interrupted writes,
/// but this is not a crash-atomic filesystem transaction.
pub struct LockedTarget {
    file: File,
    _directories: Vec<File>,
}

impl LockedTarget {
    #[cfg(windows)]
    pub fn open(path: &Path) -> Result<Self, String> {
        use std::os::windows::fs::OpenOptionsExt;
        let mut directories = Vec::new();
        let mut ancestors: Vec<_> = path.ancestors().skip(1).collect();
        ancestors.reverse();
        for directory in ancestors {
            let handle = OpenOptions::new()
                .read(true)
                .share_mode(1 | 2)
                .custom_flags(0x02000000 | 0x00200000)
                .open(directory)
                .map_err(|_| "Cannot lock edit directory; target left unchanged")?;
            verify(&handle, directory)?;
            directories.push(handle);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|_| "Target is in use or cannot be opened exclusively; left unchanged")?;
        verify(&file, path)?;
        reject_hard_links(&file)?;
        let metadata = file
            .metadata()
            .map_err(|_| "Cannot inspect opened target")?;
        use std::os::windows::fs::MetadataExt;
        if !metadata.is_file()
            || metadata.file_attributes() & (0x400 | 0x1000 | 0x4000 | 0x800 | 0x1) != 0
        {
            return Err("Unsupported target attributes; file left unchanged".into());
        }
        Ok(Self {
            file,
            _directories: directories,
        })
    }

    #[cfg(not(windows))]
    pub fn open(_path: &Path) -> Result<Self, String> {
        Err("Recoverable file mutation currently supports Windows only".into())
    }

    pub fn read(&mut self) -> Result<Vec<u8>, String> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "Cannot seek locked file")?;
        let mut bytes = Vec::new();
        (&mut self.file)
            .take((super::MAX_EDIT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read locked file")?;
        super::validate_text(&bytes)?;
        Ok(bytes)
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "Cannot seek target; outcome uncertain")?;
        self.file
            .write_all(bytes)
            .map_err(|_| "Write failed; outcome uncertain; recovery retained")?;
        self.file
            .set_len(bytes.len() as u64)
            .map_err(|_| "Truncation failed; outcome uncertain; recovery retained")?;
        self.file
            .sync_all()
            .map_err(|_| "Sync failed; outcome uncertain; recovery retained")?;
        if self.read()? != bytes {
            return Err("Verification failed; outcome uncertain; recovery retained".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
fn verify(file: &File, expected: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFinalPathNameByHandleW(
            handle: *mut std::ffi::c_void,
            buffer: *mut u16,
            length: u32,
            flags: u32,
        ) -> u32;
    }
    if file
        .metadata()
        .map_err(|_| "Cannot inspect opened path")?
        .file_attributes()
        & 0x400
        != 0
    {
        return Err("Reparse points are not supported".into());
    }
    let mut buffer = vec![0u16; 32768];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    } as usize;
    if count == 0 || count >= buffer.len() {
        return Err("Cannot verify opened path".into());
    }
    let actual = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..count]));
    let canonical_actual =
        dunce::canonicalize(actual).map_err(|_| "Cannot verify opened destination")?;
    let canonical_expected =
        dunce::canonicalize(expected).map_err(|_| "Cannot verify intended destination")?;
    if canonical_actual != canonical_expected {
        return Err("Opened path identity changed".into());
    }
    Ok(())
}
#[cfg(windows)]
fn reject_hard_links(file: &File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct FileInfo {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut FileInfo) -> i32;
    }
    let mut info = std::mem::MaybeUninit::<FileInfo>::zeroed();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err("Cannot verify target link count; file left unchanged".into());
    }
    if unsafe { info.assume_init() }.links != 1 {
        return Err("Hard-linked targets are unsupported; file left unchanged".into());
    }
    Ok(())
}
