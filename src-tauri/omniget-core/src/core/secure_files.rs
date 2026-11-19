#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
}

pub struct Root {
    directory: File,
    identity: Identity,
}
pub struct Snapshot {
    pub file: File,
    pub bytes: u64,
    pub digest: String,
    pub identity: Identity,
}

fn denied() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "FILE_ACCESS_DENIED")
}

#[cfg(unix)]
mod os {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    const MAX_NEW_DIRS: usize = 8;

    pub fn identity(file: &File) -> std::io::Result<Identity> {
        let m = file.metadata()?;
        Ok(Identity {
            device: m.dev(),
            inode: m.ino(),
        })
    }
    pub struct Stamp {
        pub bytes: u64,
        pub written: (i64, i64),
        pub changed: (i64, i64),
        pub links: u64,
    }
    pub fn stamp(file: &File) -> std::io::Result<Stamp> {
        let m = file.metadata()?;
        Ok(Stamp {
            bytes: m.len(),
            written: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            links: m.nlink(),
        })
    }
    pub fn name(value: &std::ffi::OsStr) -> std::io::Result<CString> {
        CString::new(value.as_bytes()).map_err(|_| denied())
    }
    pub fn at(dir: &File, n: &std::ffi::OsStr, flags: i32) -> std::io::Result<File> {
        let n = name(n)?;
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                n.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub fn root(path: &Path) -> std::io::Result<File> {
        if !path.is_absolute() {
            return Err(denied());
        }
        let mut dir = File::open("/")?;
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(n) => dir = at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY)?,
                _ => return Err(denied()),
            }
        }
        Ok(dir)
    }
    pub fn parent(root: &File, path: &Path) -> std::io::Result<(File, std::ffi::OsString)> {
        let raw = path.as_os_str().as_bytes();
        if raw.is_empty()
            || raw
                .split(|b| *b == b'/')
                .any(|s| s.is_empty() || s == b"." || s == b"..")
        {
            return Err(denied());
        }
        let mut parts = path.components().peekable();
        let mut dir = root.try_clone()?;
        while let Some(part) = parts.next() {
            let Component::Normal(n) = part else {
                return Err(denied());
            };
            if parts.peek().is_none() {
                return Ok((dir, n.to_os_string()));
            }
            dir = at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY)?;
        }
        Err(denied())
    }
    pub fn read(root: &File, path: &Path) -> std::io::Result<File> {
        let (dir, n) = parent(root, path)?;
        let file = at(&dir, &n, libc::O_RDONLY | libc::O_NONBLOCK)?;
        let m = file.metadata()?;
        if !m.is_file() || m.nlink() != 1 {
            return Err(denied());
        }
        Ok(file)
    }
    pub fn parent_creating(
        root: &File,
        path: &Path,
    ) -> std::io::Result<(File, std::ffi::OsString)> {
        let raw = path.as_os_str().as_bytes();
        if raw.is_empty()
            || raw
                .split(|b| *b == b'/')
                .any(|s| s.is_empty() || s == b"." || s == b"..")
        {
            return Err(denied());
        }
        let mut parts = path.components().peekable();
        let mut dir = root.try_clone()?;
        let mut created = 0;
        while let Some(part) = parts.next() {
            let Component::Normal(n) = part else {
                return Err(denied());
            };
            if parts.peek().is_none() {
                return Ok((dir, n.to_os_string()));
            }
            dir = match at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && created < MAX_NEW_DIRS => {
                    let c = name(n)?;
                    if unsafe { libc::mkdirat(dir.as_raw_fd(), c.as_ptr(), 0o755) } != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    created += 1;
                    dir.sync_all()?;
                    at(&dir, n, libc::O_RDONLY | libc::O_DIRECTORY)?
                }
                other => other?,
            };
        }
        Err(denied())
    }
    pub fn create(root: &File, staging: &File, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let (dir, n) = parent_creating(root, path)?;
        let tmp = format!(".omniget-{}.pending", uuid::Uuid::new_v4());
        let mut file = at(
            staging,
            std::ffi::OsStr::new(&tmp),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        let tmp = name(std::ffi::OsStr::new(&tmp))?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            let n = name(&n)?;
            if unsafe {
                libc::linkat(
                    staging.as_raw_fd(),
                    tmp.as_ptr(),
                    dir.as_raw_fd(),
                    n.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        })();
        if unsafe { libc::unlinkat(staging.as_raw_fd(), tmp.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        staging.sync_all()?;
        dir.sync_all()?;
        result
    }
}

#[cfg(windows)]
mod os {
    use super::*;
    use std::ffi::{c_void, OsStr, OsString};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Prefix;
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FileLinkInformation, NtCreateFile, NtSetInformationFile, FILE_CREATE, FILE_DIRECTORY_FILE,
        FILE_LINK_INFORMATION, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_FOR_BACKUP_INTENT,
        FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, RtlNtStatusToDosError, HANDLE, UNICODE_STRING,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, FileDispositionInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        DELETE, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
        FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    const MAX_NEW_DIRS: usize = 8;
    const MAX_NAME: usize = 255;
    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    const DIRECTORY_ACCESS: u32 =
        FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
    const DIRECTORY_OPTIONS: u32 = FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT;
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    pub struct Stamp {
        pub bytes: u64,
        pub written: i64,
        pub changed: i64,
        pub links: u32,
    }

    struct Descriptor(*mut c_void);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: the pointer came from ConvertStringSecurityDescriptorToSecurityDescriptorW, which allocates with LocalAlloc, and is freed once.
            unsafe { LocalFree(self.0) };
        }
    }

    fn last() -> std::io::Error {
        std::io::Error::last_os_error()
    }

    fn details(file: &File) -> std::io::Result<BY_HANDLE_FILE_INFORMATION> {
        // SAFETY: BY_HANDLE_FILE_INFORMATION is plain integers, valid when zeroed.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: the handle is open for the lifetime of `file` and `info` is a valid out pointer.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
            return Err(last());
        }
        Ok(info)
    }

    pub fn identity(file: &File) -> std::io::Result<Identity> {
        let info = details(file)?;
        Ok(Identity {
            device: u64::from(info.dwVolumeSerialNumber),
            inode: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }

    pub fn stamp(file: &File) -> std::io::Result<Stamp> {
        let info = details(file)?;
        // SAFETY: FILE_BASIC_INFO is plain integers, valid when zeroed.
        let mut basic: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: the handle is open and the buffer is exactly one FILE_BASIC_INFO.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle() as HANDLE,
                FileBasicInfo,
                (&raw mut basic).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
        {
            return Err(last());
        }
        Ok(Stamp {
            bytes: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
            written: basic.LastWriteTime,
            changed: basic.ChangeTime,
            links: info.nNumberOfLinks,
        })
    }

    pub fn name(value: &OsStr) -> std::io::Result<Vec<u16>> {
        let wide: Vec<u16> = value.encode_wide().collect();
        let forbidden = |c: &u16| *c < 0x20 || b"\"*/:<>?\\|".iter().any(|f| u16::from(*f) == *c);
        let dot = u16::from(b'.');
        let stem: String = wide
            .iter()
            .take_while(|c| **c != dot)
            .map(|c| char::from_u32(u32::from(*c)).unwrap_or('\u{fffd}'))
            .collect();
        if wide.is_empty()
            || wide.len() > MAX_NAME
            || wide.iter().any(forbidden)
            || wide
                .last()
                .is_some_and(|c| *c == dot || *c == u16::from(b' '))
            || RESERVED
                .iter()
                .any(|r| r.eq_ignore_ascii_case(stem.trim_end()))
        {
            return Err(denied());
        }
        Ok(wide)
    }

    fn entry(
        dir: &File,
        name: &[u16],
        access: u32,
        disposition: u32,
        options: u32,
        security: Option<&Descriptor>,
    ) -> std::io::Result<File> {
        let bytes = u16::try_from(name.len() * 2).map_err(|_| denied())?;
        let object = UNICODE_STRING {
            Length: bytes,
            MaximumLength: bytes,
            Buffer: name.as_ptr().cast_mut(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: dir.as_raw_handle() as HANDLE,
            ObjectName: &object,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: security.map_or(std::ptr::null(), |d| d.0.cast_const().cast()),
            SecurityQualityOfService: std::ptr::null(),
        };
        let mut handle: HANDLE = std::ptr::null_mut();
        // SAFETY: IO_STATUS_BLOCK is plain data, valid when zeroed.
        let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
        // SAFETY: every pointer refers to a live local; the name buffer outlives the call and is only read; the root handle is open for the lifetime of `dir`.
        let code = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &attributes,
                &mut status,
                std::ptr::null(),
                FILE_ATTRIBUTE_NORMAL,
                SHARE_ALL,
                disposition,
                options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
                std::ptr::null(),
                0,
            )
        };
        if code < 0 {
            // SAFETY: RtlNtStatusToDosError only maps an integer.
            return Err(std::io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(code) } as i32,
            ));
        }
        // SAFETY: NtCreateFile succeeded, so `handle` is a new handle this function owns.
        Ok(unsafe { File::from_raw_handle(handle.cast()) })
    }

    fn open(
        dir: &File,
        name: &[u16],
        access: u32,
        disposition: u32,
        options: u32,
        security: Option<&Descriptor>,
    ) -> std::io::Result<File> {
        let file = entry(dir, name, access, disposition, options, security)?;
        if details(&file)?.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(denied());
        }
        Ok(file)
    }

    pub fn identity_at(root: &File, path: &Path) -> std::io::Result<Identity> {
        let (dir, n) = walk(root, path, false)?;
        identity(&entry(
            &dir,
            &name(&n)?,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_OPEN,
            FILE_OPEN_FOR_BACKUP_INTENT,
            None,
        )?)
    }

    fn directory(dir: &File, n: &OsStr) -> std::io::Result<File> {
        open(
            dir,
            &name(n)?,
            DIRECTORY_ACCESS,
            FILE_OPEN,
            DIRECTORY_OPTIONS,
            None,
        )
    }

    fn private(inherited: bool) -> std::io::Result<Descriptor> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo handle of the current process is always valid and `token` is a valid out pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(last());
        }
        let mut user = [0u64; 16];
        let mut used = 0u32;
        // SAFETY: the buffer is 128 bytes aligned to 8, enough for TOKEN_USER plus the largest SID, and its size is passed along.
        let read = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                user.as_mut_ptr().cast(),
                std::mem::size_of_val(&user) as u32,
                &mut used,
            )
        };
        let failure = last();
        // SAFETY: `token` was opened above and is closed once.
        unsafe { CloseHandle(token) };
        if read == 0 {
            return Err(failure);
        }
        // SAFETY: GetTokenInformation succeeded, so the buffer starts with a TOKEN_USER whose SID pointer points inside the same buffer.
        let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: `sid` is valid while `user` lives and `text` is a valid out pointer.
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return Err(last());
        }
        let mut rule: Vec<u16> = if inherited {
            "D:P(A;OICI;FA;;;"
        } else {
            "D:P(A;;FA;;;"
        }
        .encode_utf16()
        .collect();
        let mut at = text;
        // SAFETY: ConvertSidToStringSidW returned a NUL-terminated string; the walk stops at the terminator.
        unsafe {
            while *at != 0 {
                rule.push(*at);
                at = at.add(1);
            }
            LocalFree(text.cast());
        }
        rule.extend(")\0".encode_utf16());
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        // SAFETY: `rule` is NUL-terminated and `descriptor` is a valid out pointer.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                rule.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(last());
        }
        Ok(Descriptor(descriptor))
    }

    pub fn root(path: &Path) -> std::io::Result<File> {
        let mut parts = path.components();
        let (Some(Component::Prefix(prefix)), Some(Component::RootDir)) =
            (parts.next(), parts.next())
        else {
            return Err(denied());
        };
        if !matches!(
            prefix.kind(),
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..)
        ) {
            return Err(denied());
        }
        let mut base = prefix.as_os_str().to_os_string();
        base.push("\\");
        let mut dir = std::fs::OpenOptions::new()
            .access_mode(DIRECTORY_ACCESS)
            .share_mode(SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&base)?;
        if details(&dir)?.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(denied());
        }
        for part in parts {
            match part {
                Component::Normal(n) => dir = directory(&dir, n)?,
                _ => return Err(denied()),
            }
        }
        Ok(dir)
    }

    fn walk(root: &File, path: &Path, creating: bool) -> std::io::Result<(File, OsString)> {
        let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
        let dot = u16::from(b'.');
        if raw.is_empty()
            || raw
                .split(|c| *c == u16::from(b'/') || *c == u16::from(b'\\'))
                .any(|s| s.is_empty() || s == [dot] || s == [dot, dot])
        {
            return Err(denied());
        }
        let mut parts = path.components().peekable();
        let mut dir = root.try_clone()?;
        let mut created = 0;
        while let Some(part) = parts.next() {
            let Component::Normal(n) = part else {
                return Err(denied());
            };
            if parts.peek().is_none() {
                return Ok((dir, n.to_os_string()));
            }
            dir = match directory(&dir, n) {
                Err(e)
                    if creating
                        && e.kind() == std::io::ErrorKind::NotFound
                        && created < MAX_NEW_DIRS =>
                {
                    created += 1;
                    open(
                        &dir,
                        &name(n)?,
                        DIRECTORY_ACCESS,
                        FILE_CREATE,
                        DIRECTORY_OPTIONS,
                        None,
                    )?
                }
                other => other?,
            };
        }
        Err(denied())
    }

    pub fn read(root: &File, path: &Path) -> std::io::Result<File> {
        let (dir, n) = walk(root, path, false)?;
        let file = open(
            &dir,
            &name(&n)?,
            FILE_GENERIC_READ,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
            None,
        )?;
        if details(&file)?.nNumberOfLinks != 1 {
            return Err(denied());
        }
        Ok(file)
    }

    pub fn private_directory(dir: &File, n: &OsStr) -> std::io::Result<()> {
        open(
            dir,
            &name(n)?,
            DIRECTORY_ACCESS,
            FILE_CREATE,
            DIRECTORY_OPTIONS,
            Some(&private(true)?),
        )
        .map(drop)
    }

    fn link(file: &File, dir: &File, leaf: &[u16]) -> std::io::Result<()> {
        let bytes = leaf.len() * 2;
        let size = std::mem::offset_of!(FILE_LINK_INFORMATION, FileName) + bytes;
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let info = buffer.as_mut_ptr().cast::<FILE_LINK_INFORMATION>();
        // SAFETY: the buffer is zeroed, aligned to 8 and holds the header plus `bytes` of name; zero leaves ReplaceIfExists off.
        unsafe {
            (*info).RootDirectory = dir.as_raw_handle() as HANDLE;
            (*info).FileNameLength = bytes as u32;
            std::ptr::copy_nonoverlapping(
                leaf.as_ptr(),
                (&raw mut (*info).FileName).cast::<u16>(),
                leaf.len(),
            );
        }
        // SAFETY: IO_STATUS_BLOCK is plain data, valid when zeroed.
        let mut status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
        // SAFETY: both handles are open, and `info` points to `size` initialised bytes that outlive the call.
        let code = unsafe {
            NtSetInformationFile(
                file.as_raw_handle() as HANDLE,
                &mut status,
                info.cast(),
                size as u32,
                FileLinkInformation,
            )
        };
        if code < 0 {
            // SAFETY: RtlNtStatusToDosError only maps an integer.
            return Err(std::io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(code) } as i32,
            ));
        }
        Ok(())
    }

    fn discard(file: &File) -> std::io::Result<()> {
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: the handle is open with DELETE access and the buffer is exactly one FILE_DISPOSITION_INFO.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle() as HANDLE,
                FileDispositionInfo,
                (&raw const info).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(last());
        }
        Ok(())
    }

    pub fn create(root: &File, staging: &File, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let (dir, n) = walk(root, path, true)?;
        let leaf = name(&n)?;
        let tmp = format!(".omniget-{}.pending", uuid::Uuid::new_v4());
        let mut file = open(
            staging,
            &name(OsStr::new(&tmp))?,
            FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES | DELETE,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE,
            Some(&private(false)?),
        )?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            link(&file, &dir, &leaf)
        })();
        discard(&file)?;
        result
    }
}

impl Root {
    pub fn open(path: &Path, expected: Option<&Identity>) -> std::io::Result<Self> {
        #[cfg(any(unix, windows))]
        {
            let directory = os::root(path)?;
            let identity = os::identity(&directory)?;
            if expected.is_some_and(|e| e != &identity) {
                return Err(denied());
            }
            Ok(Self {
                directory,
                identity,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (path, expected);
            Err(denied())
        }
    }
    pub fn create_directory(&self, relative: &Path) -> std::io::Result<()> {
        let mut parts = relative.components();
        let (Some(std::path::Component::Normal(name)), None) = (parts.next(), parts.next()) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "single directory name required",
            ));
        };
        #[cfg(unix)]
        {
            use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
            let name = std::ffi::CString::new(name.as_bytes())
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
            if unsafe { libc::mkdirat(self.directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            self.directory.sync_all()?;
            Ok(())
        }
        #[cfg(windows)]
        {
            os::private_directory(&self.directory, name)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = name;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "descriptor directories unavailable",
            ))
        }
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn snapshot(&self, relative: &Path, max_bytes: u64) -> std::io::Result<Snapshot> {
        #[cfg(any(unix, windows))]
        {
            let mut file = os::read(&self.directory, relative)?;
            let before = os::stamp(&file)?;
            if before.bytes > max_bytes {
                return Err(denied());
            }
            let mut hash = Sha256::new();
            let mut frozen = tempfile::tempfile()?;
            let mut total = 0u64;
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total = total.checked_add(n as u64).ok_or_else(denied)?;
                if total > max_bytes {
                    return Err(denied());
                }
                hash.update(&buf[..n]);
                frozen.write_all(&buf[..n])?;
            }
            let after = os::stamp(&file)?;
            if before.bytes != total
                || after.bytes != total
                || before.written != after.written
                || before.changed != after.changed
                || after.links != 1
            {
                return Err(denied());
            }
            frozen.seek(SeekFrom::Start(0))?;
            let identity = os::identity(&file)?;
            Ok(Snapshot {
                file: frozen,
                bytes: total,
                digest: format!("{:x}", hash.finalize()),
                identity,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (relative, max_bytes);
            Err(denied())
        }
    }
    pub fn create_new(&self, staging: &Root, relative: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if self.identity == staging.identity || self.identity.device != staging.identity.device {
            return Err(denied());
        }
        #[cfg(any(unix, windows))]
        {
            os::create(&self.directory, &staging.directory, relative, bytes)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (staging, relative, bytes);
            Err(denied())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn fixture() -> std::path::PathBuf {
        let p = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn object_bound_access_rejects_escape_links_and_overwrite() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        root.create_new(&staging, Path::new("valid"), b"verified bytes")
            .unwrap();
        assert!(root
            .create_new(&staging, Path::new("valid"), b"corrupt")
            .is_err());
        for path in ["../escape", "/etc/passwd", "./valid", "valid/../valid", ""] {
            assert!(root.snapshot(Path::new(path), 100).is_err());
        }
        symlink("valid", p.join("link")).unwrap();
        assert!(root.snapshot(Path::new("link"), 100).is_err());
        std::fs::hard_link(p.join("valid"), p.join("hard")).unwrap();
        assert!(root.snapshot(Path::new("valid"), 100).is_err());
        std::fs::remove_file(p.join("hard")).unwrap();
        let mut snap = root.snapshot(Path::new("valid"), 100).unwrap();
        std::fs::write(p.join("valid"), b"changed source").unwrap();
        std::fs::rename(p.join("valid"), p.join("old")).unwrap();
        symlink("/etc/passwd", p.join("valid")).unwrap();
        let mut data = String::new();
        snap.file.read_to_string(&mut data).unwrap();
        assert_eq!(data, "verified bytes");
        assert!(root.snapshot(Path::new("valid"), 100).is_err());
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(staging_path).unwrap();
    }
    #[test]
    fn create_adds_missing_folders_but_never_adopts_a_planted_link() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        root.create_new(&staging, Path::new("piloto/deep/resultado.md"), b"ok")
            .unwrap();
        assert_eq!(
            std::fs::read(p.join("piloto/deep/resultado.md")).unwrap(),
            b"ok"
        );
        let outside = fixture();
        symlink(&outside, p.join("planted")).unwrap();
        assert!(root
            .create_new(&staging, Path::new("planted/new/file"), b"no")
            .is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        let nine = (0..9)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/")
            + "/f";
        assert!(root.create_new(&staging, Path::new(&nine), b"no").is_err());
        for d in [p, staging_path, outside] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }
    #[test]
    fn changed_root_identity_and_parent_symlink_are_denied() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        std::fs::create_dir(p.join("folder")).unwrap();
        symlink("folder", p.join("alias")).unwrap();
        assert!(root
            .create_new(&staging, Path::new("alias/file"), b"no")
            .is_err());
        let wrong = Identity {
            device: root.identity().device,
            inode: root.identity().inode + 1,
        };
        assert!(Root::open(&p, Some(&wrong)).is_err());
        assert!(root
            .create_new(&staging, Path::new("folder/file"), b"ok")
            .is_ok());
        assert!(root.snapshot(Path::new("folder/file"), 1).is_err());
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(staging_path).unwrap();
    }
}

#[cfg(unix)]
mod manifest_files {
    use super::*;
    use std::ffi::{CStr, CString, OsString};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    fn open_dir_at(parent: &File, name: &std::ffi::OsStr) -> std::io::Result<File> {
        let name = CString::new(name.as_bytes()).map_err(|_| denied())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    fn stamp(m: &std::fs::Metadata) -> (u64, u64, i64, i64, i64, i64) {
        (
            m.dev(),
            m.ino(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    }
    fn walk(
        dir: &File,
        prefix: &Path,
        limit: usize,
        seen: &mut usize,
        out: &mut Vec<(PathBuf, bool)>,
    ) -> std::io::Result<()> {
        if prefix.components().count() > 64 {
            return Err(denied());
        }
        let before = dir.metadata()?;
        let scan = open_dir_at(dir, std::ffi::OsStr::new("."))?;
        use std::os::fd::IntoRawFd;
        let fd = scan.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(std::io::Error::last_os_error());
        }
        let stream = Stream(stream);
        loop {
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error().unwrap_or(0) != 0 {
                    return Err(error);
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            *seen = seen.checked_add(1).ok_or_else(denied)?;
            if *seen > limit {
                return Err(denied());
            }
            let name = OsString::from_vec(name.to_vec());
            let c_name = CString::new(name.as_bytes()).map_err(|_| denied())?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let md = file.metadata()?;
            let relative = prefix.join(&name);
            if md.is_dir() {
                out.push((relative.clone(), true));
                walk(&file, &relative, limit, seen, out)?;
            } else if md.is_file() && md.nlink() == 1 {
                out.push((relative, false));
            } else {
                return Err(denied());
            }
        }
        if stamp(&before) != stamp(&dir.metadata()?) {
            return Err(denied());
        }
        Ok(())
    }
    pub fn entries(
        root: &File,
        relative: &Path,
        limit: usize,
    ) -> std::io::Result<Vec<(PathBuf, bool)>> {
        let directory = if relative == Path::new(".") {
            open_dir_at(root, std::ffi::OsStr::new("."))?
        } else {
            let (parent, name) = os::parent(root, relative)?;
            open_dir_at(&parent, &name)?
        };
        let prefix = if relative == Path::new(".") {
            Path::new("")
        } else {
            relative
        };
        let mut out = Vec::new();
        walk(&directory, prefix, limit, &mut 0, &mut out)?;
        out.sort();
        Ok(out)
    }

    fn tree_error(code: &str, detail: String) -> std::io::Error {
        std::io::Error::other(format!("{code}: {detail}"))
    }
    #[allow(clippy::too_many_arguments)]
    fn scan_walk(
        dir: &File,
        prefix: &Path,
        limit: usize,
        filter: &dyn Fn(&Path) -> super::ScanFilter,
        seen: &mut usize,
        out: &mut super::TreeScan,
    ) -> std::io::Result<()> {
        if prefix.components().count() > 64 {
            out.skipped.push((prefix.to_path_buf(), "too_deep"));
            return Ok(());
        }
        let before = dir.metadata()?;
        let scan = open_dir_at(dir, std::ffi::OsStr::new("."))?;
        use std::os::fd::IntoRawFd;
        let fd = scan.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(std::io::Error::last_os_error());
        }
        let stream = Stream(stream);
        loop {
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error().unwrap_or(0) != 0 {
                    return Err(tree_error(
                        "FILE_TREE_READ_FAILED",
                        format!("{}: {error}", prefix.display()),
                    ));
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let name = OsString::from_vec(name.to_vec());
            let relative = prefix.join(&name);
            let decision = filter(&relative);
            if decision == super::ScanFilter::Hide {
                continue;
            }
            *seen = seen.saturating_add(1);
            if *seen > limit {
                return Err(tree_error(
                    "FILE_TREE_LIMIT",
                    format!(
                        "more than {limit} entries; stopped at `{}`. Pass a narrower `path`",
                        relative.display()
                    ),
                ));
            }
            let c_name = CString::new(name.as_bytes()).map_err(|_| denied())?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let e = std::io::Error::last_os_error();
                let why = match e.raw_os_error() {
                    Some(libc::ELOOP) | Some(libc::EMLINK) => "symlink",
                    Some(libc::ENOENT) => "vanished",
                    Some(libc::ENXIO) | Some(libc::EOPNOTSUPP) => "special",
                    _ => "unreadable",
                };
                out.skipped.push((relative, why));
                continue;
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let Ok(md) = file.metadata() else {
                out.skipped.push((relative, "unreadable"));
                continue;
            };
            if md.is_dir() {
                out.entries.push((relative.clone(), true));
                if decision == super::ScanFilter::ListOnly {
                    out.skipped.push((relative, "ignored_dir"));
                    continue;
                }
                scan_walk(&file, &relative, limit, filter, seen, out)?;
            } else if md.is_file() && md.nlink() == 1 {
                out.entries.push((relative, false));
            } else if md.is_file() {
                out.skipped.push((relative, "hardlink"));
            } else {
                out.skipped.push((relative, "special"));
            }
        }
        if stamp(&before) != stamp(&dir.metadata()?) {
            out.skipped.push((
                if prefix.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    prefix.to_path_buf()
                },
                "changed_while_listing",
            ));
        }
        Ok(())
    }
    pub fn scan(
        root: &File,
        relative: &Path,
        limit: usize,
        filter: &dyn Fn(&Path) -> super::ScanFilter,
    ) -> std::io::Result<super::TreeScan> {
        let directory = if relative == Path::new(".") {
            open_dir_at(root, std::ffi::OsStr::new("."))?
        } else {
            let (parent, name) = os::parent(root, relative)?;
            open_dir_at(&parent, &name)?
        };
        let prefix = if relative == Path::new(".") {
            Path::new("")
        } else {
            relative
        };
        let mut out = super::TreeScan::default();
        scan_walk(&directory, prefix, limit, filter, &mut 0, &mut out)?;
        out.entries.sort();
        out.skipped.sort();
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanFilter {
    Walk,
    Hide,
    ListOnly,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeScan {
    pub entries: Vec<(std::path::PathBuf, bool)>,
    pub skipped: Vec<(std::path::PathBuf, &'static str)>,
}

impl Root {
    pub fn scan(
        &self,
        relative: &Path,
        max_entries: usize,
        filter: &dyn Fn(&Path) -> ScanFilter,
    ) -> std::io::Result<TreeScan> {
        #[cfg(unix)]
        {
            manifest_files::scan(&self.directory, relative, max_entries, filter)
        }
        #[cfg(not(unix))]
        {
            let _ = (relative, max_entries, filter);
            Err(denied())
        }
    }
    pub fn identity_of(&self, relative: &Path) -> std::io::Result<Identity> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let (dir, leaf) = os::parent(&self.directory, relative)?;
            let leaf = os::name(&leaf)?;
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    dir.as_raw_fd(),
                    leaf.as_ptr(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            #[allow(clippy::unnecessary_cast)]
            Ok(Identity {
                device: st.st_dev as u64,
                inode: st.st_ino as u64,
            })
        }
        #[cfg(windows)]
        {
            os::identity_at(&self.directory, relative)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = relative;
            Err(denied())
        }
    }
}
impl Root {
    pub fn entries(
        &self,
        relative: &Path,
        max_entries: usize,
    ) -> std::io::Result<Vec<(std::path::PathBuf, bool)>> {
        #[cfg(unix)]
        {
            manifest_files::entries(&self.directory, relative, max_entries)
        }
        #[cfg(not(unix))]
        {
            let _ = (relative, max_entries);
            Err(denied())
        }
    }
}

pub mod replace_codes {
    pub const PATH_INVALID: &str = "FILE_PATH_INVALID";
    pub const NOT_FOUND: &str = "FILE_NOT_FOUND";
    pub const ACCESS_DENIED: &str = "FILE_ACCESS_DENIED";
    pub const SYMLINK: &str = "FILE_SYMLINK_REFUSED";
    pub const PARENT_NOT_DIRECTORY: &str = "FILE_PARENT_NOT_DIRECTORY";
    pub const NOT_REGULAR: &str = "FILE_NOT_REGULAR";
    pub const HARDLINKED: &str = "FILE_HARDLINKED";
    pub const OWNER: &str = "FILE_OWNER_MISMATCH";
    pub const SIZE: &str = "FILE_SIZE_LIMIT";
    pub const DIGEST_FORMAT: &str = "EXPECTED_SHA256_INVALID";
    pub const CONFLICT: &str = "FILE_REVISION_CONFLICT";
    pub const CHANGED: &str = "FILE_CHANGED_DURING_COMMIT";
    pub const LOCKED: &str = "FILE_LOCKED";
    pub const ROLLBACK_FAILED: &str = "FILE_COMMIT_ROLLBACK_FAILED";
    pub const UNSUPPORTED: &str = "ATOMIC_REPLACE_UNSUPPORTED";
    pub const STAGING: &str = "STAGING_UNAVAILABLE";
    pub const ROOT: &str = "WORKSPACE_ROOT_CHANGED";
    pub const IO: &str = "FILE_IO_ERROR";
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct Replaced {
    pub previous_digest: String,
    pub digest: String,
    pub bytes: u64,
    pub identity: Identity,
}

#[cfg(test)]
pub mod replace_test_hook {
    use std::cell::RefCell;
    type Hook = Box<dyn FnMut(&str)>;
    thread_local! { static HOOK: RefCell<Option<Hook>> = RefCell::new(None); }
    pub fn set(f: impl FnMut(&str) + 'static) {
        HOOK.with(|h| *h.borrow_mut() = Some(Box::new(f)));
    }
    pub fn clear() {
        HOOK.with(|h| *h.borrow_mut() = None);
    }
    pub(crate) fn fire(stage: &str) {
        let taken = HOOK.with(|h| h.borrow_mut().take());
        if let Some(mut f) = taken {
            f(stage);
            HOOK.with(|h| {
                let mut b = h.borrow_mut();
                if b.is_none() {
                    *b = Some(f);
                }
            });
        }
    }
}

fn valid_sha256(value: &str) -> Option<String> {
    let v = value.to_ascii_lowercase();
    (v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit())).then_some(v)
}

#[cfg(unix)]
mod replace_os {
    use super::replace_codes as c;
    use super::*;
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    fn fire(_stage: &str) {
        #[cfg(test)]
        super::replace_test_hook::fire(_stage);
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Stamp {
        dev: u64,
        ino: u64,
        nlink: u64,
        size: i64,
        mtime: i64,
        mtime_nsec: i64,
        ctime: i64,
        ctime_nsec: i64,
        mode: u32,
        uid: u32,
    }
    #[allow(clippy::unnecessary_cast)]
    fn stamp(st: &libc::stat) -> Stamp {
        Stamp {
            dev: st.st_dev as u64,
            ino: st.st_ino as u64,
            nlink: st.st_nlink as u64,
            size: st.st_size as i64,
            mtime: st.st_mtime as i64,
            mtime_nsec: st.st_mtime_nsec as i64,
            ctime: st.st_ctime as i64,
            ctime_nsec: st.st_ctime_nsec as i64,
            mode: st.st_mode as u32,
            uid: st.st_uid as u32,
        }
    }
    fn fstat(file: &File) -> Result<Stamp, String> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(file.as_raw_fd(), &mut st) } != 0 {
            return Err(c::IO.into());
        }
        Ok(stamp(&st))
    }
    fn stat_at(dir: &File, name: &CStr) -> std::io::Result<Stamp> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                name.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(stamp(&st))
    }
    #[allow(clippy::unnecessary_cast)]
    fn is_regular(s: &Stamp) -> bool {
        s.mode & (libc::S_IFMT as u32) == libc::S_IFREG as u32
    }
    fn open_code(e: &std::io::Error) -> &'static str {
        match e.raw_os_error() {
            Some(libc::ENOENT) => c::NOT_FOUND,
            Some(libc::ELOOP) | Some(libc::EMLINK) => c::SYMLINK,
            Some(libc::ENOTDIR) => c::PARENT_NOT_DIRECTORY,
            Some(libc::EACCES) | Some(libc::EPERM) => c::ACCESS_DENIED,
            Some(libc::ENXIO) | Some(libc::EOPNOTSUPP) => c::NOT_REGULAR,
            _ if e.kind() == std::io::ErrorKind::PermissionDenied => c::ACCESS_DENIED,
            _ => c::IO,
        }
    }
    fn check_target(s: &Stamp, max: u64) -> Result<(), String> {
        if !is_regular(s) {
            return Err(c::NOT_REGULAR.into());
        }
        if s.nlink != 1 {
            return Err(c::HARDLINKED.into());
        }
        if s.uid != unsafe { libc::geteuid() } as u32 {
            return Err(c::OWNER.into());
        }
        if s.size < 0 || s.size as u64 > max {
            return Err(c::SIZE.into());
        }
        Ok(())
    }
    fn read_all(file: &mut File, max: u64) -> Result<(Vec<u8>, String), String> {
        file.seek(SeekFrom::Start(0)).map_err(|_| c::IO)?;
        let mut out = Vec::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = match file.read(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    return Err(c::NOT_REGULAR.into())
                }
                Err(_) => return Err(c::IO.into()),
            };
            if n == 0 {
                break;
            }
            if out.len() as u64 + n as u64 > max {
                return Err(c::SIZE.into());
            }
            out.extend_from_slice(&buf[..n]);
        }
        let digest = format!("{:x}", Sha256::digest(&out));
        Ok((out, digest))
    }
    fn swap(a_dir: &File, a: &CStr, b_dir: &File, b: &CStr) -> std::io::Result<()> {
        #[cfg(target_os = "macos")]
        let r = unsafe {
            libc::renameatx_np(
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let r = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        } as i32;
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
        let r = {
            let _ = (a_dir, a, b_dir, b);
            return Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
        };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    fn move_noreplace(a_dir: &File, a: &CStr, b_dir: &File, b: &CStr) -> std::io::Result<()> {
        #[cfg(target_os = "macos")]
        let r = unsafe {
            libc::renameatx_np(
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let r = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                a_dir.as_raw_fd(),
                a.as_ptr(),
                b_dir.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } as i32;
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
        let r = {
            let _ = (a_dir, a, b_dir, b);
            return Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
        };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    fn same_parent(root: &File, relative: &Path, expected: &Identity) -> bool {
        os::parent(root, relative)
            .ok()
            .and_then(|(d, _)| os::identity(&d).ok())
            .is_some_and(|i| &i == expected)
    }

    fn recovery_name(leaf: &std::ffi::OsStr) -> std::ffi::OsString {
        let lossy = leaf.to_string_lossy();
        let mut head = String::new();
        for ch in lossy.chars() {
            if head.len() + ch.len_utf8() > 160 {
                break;
            }
            head.push(ch);
        }
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let id = uuid::Uuid::new_v4().simple().to_string();
        std::ffi::OsString::from(format!("{head}.omniget-recovered-{ms}-{}", &id[..8]))
    }
    fn recovered_path(relative: &Path, name: &std::ffi::OsStr) -> String {
        relative
            .parent()
            .unwrap_or(Path::new(""))
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
    fn preserve_staged(
        staging: &File,
        tmp: &CStr,
        dir: &File,
        leaf: &std::ffi::OsStr,
        relative: &Path,
    ) -> String {
        let name = recovery_name(leaf);
        let Ok(name_c) = os::name(&name) else {
            return "unrecoverable: invalid name".into();
        };
        if move_noreplace(staging, tmp, dir, &name_c).is_ok() {
            let _ = dir.sync_all();
            let _ = staging.sync_all();
            return recovered_path(relative, &name);
        }
        let tmp_os = std::ffi::OsStr::from_bytes(tmp.to_bytes());
        match os::at(staging, tmp_os, libc::O_RDONLY | libc::O_NONBLOCK) {
            Ok(mut f) => {
                let kept = preserve_bytes_named(&mut f, dir, &name, relative, u64::MAX);
                if !kept.starts_with("unrecoverable") {
                    unsafe {
                        libc::unlinkat(staging.as_raw_fd(), tmp.as_ptr(), 0);
                    }
                }
                kept
            }
            Err(_) => "unrecoverable: staging object vanished".into(),
        }
    }
    fn preserve_bytes(
        f: &mut File,
        dir: &File,
        leaf: &std::ffi::OsStr,
        relative: &Path,
        max: u64,
    ) -> String {
        preserve_bytes_named(f, dir, &recovery_name(leaf), relative, max)
    }
    fn preserve_bytes_named(
        f: &mut File,
        dir: &File,
        name: &std::ffi::OsStr,
        relative: &Path,
        max: u64,
    ) -> String {
        let bytes = match read_all(f, max) {
            Ok((b, _)) => b,
            Err(e) => return format!("unrecoverable: {e}"),
        };
        let written =
            os::at(dir, name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL).and_then(|mut out| {
                out.write_all(&bytes)?;
                out.sync_all()
            });
        match written {
            Ok(()) => {
                let _ = dir.sync_all();
                recovered_path(relative, name)
            }
            Err(e) => format!("unrecoverable: {e}"),
        }
    }
    fn with_recovery(code: &str, kept: &[String]) -> String {
        if kept.is_empty() {
            code.to_string()
        } else {
            format!("{code}; recovered: {}", kept.join(", "))
        }
    }

    pub fn replace<F>(
        root: &File,
        staging: &File,
        relative: &Path,
        expected: &str,
        max: u64,
        transform: F,
    ) -> Result<Replaced, String>
    where
        F: FnOnce(&[u8]) -> Result<Vec<u8>, String>,
    {
        let (dir, leaf) = os::parent(root, relative).map_err(|e| open_code(&e))?;
        let dir_id = os::identity(&dir).map_err(|_| c::IO)?;
        let leaf_c = os::name(&leaf).map_err(|_| c::PATH_INVALID)?;
        let mut target =
            os::at(&dir, &leaf, libc::O_RDONLY | libc::O_NONBLOCK).map_err(|e| open_code(&e))?;
        let before = fstat(&target)?;
        check_target(&before, max)?;
        if unsafe { libc::flock(target.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                c::LOCKED
            } else {
                c::IO
            }
            .into());
        }
        let (old, previous) = read_all(&mut target, max)?;
        if previous != expected {
            return Err(c::CONFLICT.into());
        }
        if fstat(&target)? != before {
            return Err(c::CHANGED.into());
        }
        fire("after_read");
        let new = transform(&old)?;
        if new.len() as u64 > max {
            return Err(c::SIZE.into());
        }
        let digest = format!("{:x}", Sha256::digest(&new));

        let tmp = format!(".omniget-{}.replace", uuid::Uuid::new_v4());
        let tmp_os = std::ffi::OsStr::new(&tmp);
        let tmp_c = os::name(tmp_os).map_err(|_| c::STAGING)?;
        let mut staged = os::at(staging, tmp_os, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)
            .map_err(|_| c::STAGING)?;
        let unlink_tmp = || unsafe { libc::unlinkat(staging.as_raw_fd(), tmp_c.as_ptr(), 0) };
        let staged_st = (|| -> Result<Stamp, String> {
            staged.write_all(&new).map_err(|_| c::IO)?;
            if unsafe { libc::fchmod(staged.as_raw_fd(), (before.mode & 0o777) as libc::mode_t) }
                != 0
            {
                return Err(c::IO.into());
            }
            staged.sync_all().map_err(|_| c::IO)?;
            fstat(&staged)
        })();
        let staged_st = match staged_st {
            Ok(s) => s,
            Err(e) => {
                unlink_tmp();
                return Err(e);
            }
        };

        fire("before_precheck");
        let precheck = stat_at(&dir, &leaf_c).is_ok_and(|s| s == before)
            && fstat(&target).is_ok_and(|s| s == before)
            && same_parent(root, relative, &dir_id)
            && stat_at(staging, &tmp_c)
                .is_ok_and(|s| s.dev == staged_st.dev && s.ino == staged_st.ino);
        if !precheck {
            unlink_tmp();
            return Err(c::CHANGED.into());
        }

        fire("before_swap");
        if let Err(e) = swap(staging, &tmp_c, &dir, &leaf_c) {
            unlink_tmp();
            return Err(match e.raw_os_error() {
                Some(libc::EINVAL) | Some(libc::ENOTSUP) | Some(libc::ENOSYS)
                | Some(libc::EXDEV) => c::UNSUPPORTED,
                Some(libc::ENOENT) => c::CHANGED,
                _ => c::IO,
            }
            .into());
        }
        fire("after_swap");

        let name_is_new =
            stat_at(&dir, &leaf_c).is_ok_and(|s| s.dev == staged_st.dev && s.ino == staged_st.ino);
        let verified = name_is_new
            && same_parent(root, relative, &dir_id)
            && os::at(staging, tmp_os, libc::O_RDONLY | libc::O_NONBLOCK)
                .ok()
                .is_some_and(|mut f| {
                    fstat(&f).is_ok_and(|s| {
                        s.dev == before.dev && s.ino == before.ino && is_regular(&s) && s.nlink == 1
                    }) && read_all(&mut f, max).is_ok_and(|(_, d)| d == expected)
                });
        if !verified {
            let untouched_new = |f: &mut File| {
                fstat(f).is_ok_and(|s| {
                    s.dev == staged_st.dev
                        && s.ino == staged_st.ino
                        && s.size == staged_st.size
                        && s.mtime == staged_st.mtime
                        && s.mtime_nsec == staged_st.mtime_nsec
                }) && read_all(f, max).is_ok_and(|(_, d)| d == digest)
            };
            if !name_is_new {
                let mut recovered = Vec::new();
                if stat_at(&dir, &leaf_c).is_err_and(|e| e.raw_os_error() == Some(libc::ENOENT))
                    && move_noreplace(staging, &tmp_c, &dir, &leaf_c).is_ok()
                {
                } else if stat_at(staging, &tmp_c).is_ok() {
                    recovered.push(preserve_staged(staging, &tmp_c, &dir, &leaf, relative));
                }
                if !untouched_new(&mut staged) {
                    recovered.push(preserve_bytes(&mut staged, &dir, &leaf, relative, max));
                }
                let _ = dir.sync_all();
                return Err(with_recovery(c::CHANGED, &recovered));
            }
            if swap(staging, &tmp_c, &dir, &leaf_c).is_err() {
                let kept = preserve_staged(staging, &tmp_c, &dir, &leaf, relative);
                let _ = dir.sync_all();
                return Err(with_recovery(c::ROLLBACK_FAILED, &[kept]));
            }
            let mut recovered = Vec::new();
            let ours_untouched = os::at(staging, tmp_os, libc::O_RDONLY | libc::O_NONBLOCK)
                .ok()
                .is_some_and(|mut f| untouched_new(&mut f));
            if ours_untouched {
                unlink_tmp();
            } else if stat_at(staging, &tmp_c).is_ok() {
                recovered.push(preserve_staged(staging, &tmp_c, &dir, &leaf, relative));
            }
            let _ = dir.sync_all();
            return Err(with_recovery(c::CHANGED, &recovered));
        }
        unlink_tmp();
        dir.sync_all().map_err(|_| c::IO)?;
        let _ = staging.sync_all();
        drop(target);
        Ok(Replaced {
            previous_digest: previous,
            digest,
            bytes: new.len() as u64,
            identity: Identity {
                device: staged_st.dev,
                inode: staged_st.ino,
            },
        })
    }
}

impl Root {
    pub fn replace<F>(
        &self,
        staging: &Root,
        relative: &Path,
        expected_sha256: &str,
        max_bytes: u64,
        transform: F,
    ) -> Result<Replaced, String>
    where
        F: FnOnce(&[u8]) -> Result<Vec<u8>, String>,
    {
        use replace_codes as c;
        if self.identity == staging.identity || self.identity.device != staging.identity.device {
            return Err(c::STAGING.into());
        }
        let expected = valid_sha256(expected_sha256).ok_or(c::DIGEST_FORMAT)?;
        let raw = relative.as_os_str().as_encoded_bytes();
        if raw.is_empty()
            || raw.starts_with(b"/")
            || raw
                .split(|b| *b == b'/')
                .any(|s| s.is_empty() || s == b"." || s == b"..")
        {
            return Err(c::PATH_INVALID.into());
        }
        #[cfg(unix)]
        {
            if os::identity(&self.directory).map_err(|_| c::IO)? != self.identity {
                return Err(c::ROOT.into());
            }
            replace_os::replace(
                &self.directory,
                &staging.directory,
                relative,
                &expected,
                max_bytes,
                transform,
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (expected, max_bytes, transform);
            Err(c::UNSUPPORTED.into())
        }
    }
}

#[cfg(all(test, unix))]
mod replace_tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Root, Root, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir(base.join("ws")).unwrap();
        std::fs::create_dir(base.join("stage")).unwrap();
        let root = Root::open(&base.join("ws"), None).unwrap();
        let stage = Root::open(&base.join("stage"), None).unwrap();
        (dir, root, stage, base.join("ws"))
    }
    fn sha(b: &[u8]) -> String {
        format!("{:x}", Sha256::digest(b))
    }
    #[test]
    fn replace_commits_verified_revision_and_new_inode() {
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let r = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |old| {
                assert_eq!(old, b"old");
                Ok(b"new".to_vec())
            })
            .unwrap();
        assert_eq!(
            (r.previous_digest, r.digest, r.bytes),
            (sha(b"old"), sha(b"new"), 3)
        );
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"new");
        assert!(
            root.replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| Ok(vec![]))
                .unwrap_err()
                == replace_codes::CONFLICT
        );
        assert_eq!(
            root.replace(&root, Path::new("f"), &sha(b"new"), 64, |_| Ok(vec![]))
                .unwrap_err(),
            replace_codes::STAGING
        );
        assert_eq!(
            root.replace(&stage, Path::new("f"), &sha(b"new"), 2, |_| Ok(vec![]))
                .unwrap_err(),
            replace_codes::SIZE
        );
        assert_eq!(
            root.replace(&stage, Path::new("f"), &sha(b"new"), 64, |_| Ok(vec![
                0;
                65
            ]))
            .unwrap_err(),
            replace_codes::SIZE
        );
        assert_eq!(
            root.replace(&stage, Path::new("../f"), &sha(b"new"), 64, |_| Ok(vec![]))
                .unwrap_err(),
            replace_codes::PATH_INVALID
        );
        assert_eq!(std::fs::read_dir(stage_path(&ws)).unwrap().count(), 0);
    }
    fn stage_path(ws: &Path) -> std::path::PathBuf {
        ws.parent().unwrap().join("stage")
    }
    #[test]
    fn replace_respects_cooperative_lock_and_swap_back_restores_foreign_object() {
        use std::os::fd::AsRawFd;
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let holder = File::open(ws.join("f")).unwrap();
        assert_eq!(unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX) }, 0);
        assert_eq!(
            root.replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| Ok(
                b"new".to_vec()
            ))
            .unwrap_err(),
            replace_codes::LOCKED
        );
        drop(holder);
        let w = ws.clone();
        replace_test_hook::set(move |s| {
            if s == "before_swap" {
                std::fs::rename(w.join("f"), w.join("f.orig")).unwrap();
                std::fs::write(w.join("f"), b"old").unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"new".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        assert_eq!(err, replace_codes::CHANGED);
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"old");
        assert_eq!(std::fs::read(ws.join("f.orig")).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(stage_path(&ws)).unwrap().count(), 0);
    }

    fn recovered(ws: &Path, err: &str) -> Vec<u8> {
        let rel = err
            .split("; recovered: ")
            .nth(1)
            .unwrap_or_else(|| panic!("no recovery path in {err}"));
        assert!(
            rel.contains(".omniget-recovered-") && !rel.starts_with(".omniget-"),
            "{rel}"
        );
        std::fs::read(ws.join(rel)).unwrap()
    }
    #[test]
    fn f_r1_concurrent_atomic_save_after_swap_is_kept_and_old_revision_recovered() {
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let w = ws.clone();
        replace_test_hook::set(move |s| {
            if s == "after_swap" {
                std::fs::write(w.join(".f.editor-tmp"), b"EDITOR SAVE").unwrap();
                std::fs::rename(w.join(".f.editor-tmp"), w.join("f")).unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"ours".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        assert!(err.starts_with(replace_codes::CHANGED), "{err}");
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"EDITOR SAVE");
        assert_eq!(recovered(&ws, &err), b"old");
        assert_eq!(
            std::fs::read_dir(stage_path(&ws)).unwrap().count(),
            0,
            "nothing left in the private staging"
        );
    }
    #[test]
    fn f_r2_bytes_appended_to_new_file_during_window_are_recovered() {
        use std::io::Write;
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let mut old_fd = std::fs::OpenOptions::new()
            .write(true)
            .open(ws.join("f"))
            .unwrap();
        let w = ws.clone();
        replace_test_hook::set(move |s| {
            if s == "after_swap" {
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(w.join("f"))
                    .unwrap();
                f.write_all(b" +THEIR APPEND").unwrap();
                old_fd.write_all(b"X").unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"ours".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        assert!(err.starts_with(replace_codes::CHANGED), "{err}");
        assert_eq!(
            std::fs::read(ws.join("f")).unwrap(),
            b"Xld",
            "the stale writer's revision is back under the name"
        );
        assert_eq!(recovered(&ws, &err), b"ours +THEIR APPEND");
        assert_eq!(std::fs::read_dir(stage_path(&ws)).unwrap().count(), 0);
    }
    #[test]
    fn f_r3_rollback_failed_keeps_the_swapped_out_revision_outside_staging() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let (_d, root, stage, ws) = fixture();
        std::fs::write(ws.join("f"), b"old").unwrap();
        let mut old_fd = std::fs::OpenOptions::new()
            .write(true)
            .open(ws.join("f"))
            .unwrap();
        let st = stage_path(&ws);
        let st2 = st.clone();
        replace_test_hook::set(move |s| {
            if s == "after_swap" {
                old_fd.write_all(b"X").unwrap();
                std::fs::set_permissions(&st2, std::fs::Permissions::from_mode(0o500)).unwrap();
            }
        });
        let err = root
            .replace(&stage, Path::new("f"), &sha(b"old"), 64, |_| {
                Ok(b"ours".to_vec())
            })
            .unwrap_err();
        replace_test_hook::clear();
        std::fs::set_permissions(&st, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(err.starts_with(replace_codes::ROLLBACK_FAILED), "{err}");
        assert_eq!(std::fs::read(ws.join("f")).unwrap(), b"ours");
        let kept = recovered(&ws, &err);
        assert_eq!(kept, b"Xld");
        std::fs::remove_dir_all(&st).unwrap();
        assert_eq!(recovered(&ws, &err), b"Xld");
    }
    #[test]
    fn scan_skips_links_and_special_files_and_names_the_limit() {
        let (_d, root, _stage, ws) = fixture();
        std::fs::write(ws.join("a.txt"), b"a").unwrap();
        std::fs::create_dir_all(ws.join("node_modules/.bin")).unwrap();
        std::os::unix::fs::symlink("../x/cli.js", ws.join("node_modules/.bin/x")).unwrap();
        std::os::unix::fs::symlink("a.txt", ws.join("link")).unwrap();
        std::fs::hard_link(ws.join("a.txt"), ws.join("hard")).unwrap();
        let fifo = std::ffi::CString::new(ws.join("pipe").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        std::fs::create_dir(ws.join(".git")).unwrap();
        for i in 0..20 {
            std::fs::write(ws.join(format!(".git/{i}")), b"").unwrap();
        }
        let filter = |p: &Path| match p.file_name().and_then(|n| n.to_str()) {
            Some(".git") => ScanFilter::Hide,
            Some("node_modules") => ScanFilter::ListOnly,
            _ => ScanFilter::Walk,
        };
        let scan = root.scan(Path::new("."), 10, &filter).unwrap();
        assert_eq!(
            scan.entries,
            vec![(std::path::PathBuf::from("node_modules"), true)]
        );
        assert!(
            !scan.entries.iter().any(|(p, _)| p.starts_with(".git")),
            "hidden names never listed"
        );
        let reasons: std::collections::BTreeMap<_, _> = scan.skipped.iter().cloned().collect();
        assert_eq!(reasons.get(Path::new("link")), Some(&"symlink"));
        assert_eq!(reasons.get(Path::new("hard")), Some(&"hardlink"));
        assert_eq!(reasons.get(Path::new("a.txt")), Some(&"hardlink"));
        assert_eq!(reasons.get(Path::new("pipe")), Some(&"special"));
        assert_eq!(reasons.get(Path::new("node_modules")), Some(&"ignored_dir"));
        let err = root
            .scan(Path::new("."), 2, &filter)
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("FILE_TREE_LIMIT") && err.contains("more than 2 entries"),
            "{err}"
        );
        assert!(root.entries(Path::new("."), 1000).is_err());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn fixture() -> PathBuf {
        let p = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    fn plain(path: &Path) -> String {
        path.to_str()
            .unwrap()
            .trim_start_matches(r"\\?\")
            .to_owned()
    }
    fn junction(link: &Path, target: &Path) {
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J", &plain(link), &plain(target)])
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
    }
    fn grants(path: &Path) -> usize {
        let listed = Command::new("icacls").arg(plain(path)).output().unwrap();
        assert!(listed.status.success(), "{listed:?}");
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .filter(|l| l.contains(":("))
            .count()
    }

    #[test]
    fn files_stay_inside_the_root_and_are_never_overwritten() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        root.create_new(&staging, Path::new("valid"), b"verified bytes")
            .unwrap();
        assert!(root
            .create_new(&staging, Path::new("valid"), b"corrupt")
            .is_err());
        assert!(std::fs::read_dir(&staging_path).unwrap().next().is_none());
        for path in [
            r"..\escape",
            "../escape",
            r"C:\Windows\win.ini",
            r"\Windows\win.ini",
            r".\valid",
            r"valid\..\valid",
            "",
            "valid:stream",
            "valid.",
            "NUL",
        ] {
            assert!(root.snapshot(Path::new(path), 100).is_err(), "{path}");
            assert!(
                root.create_new(&staging, Path::new(path), b"no").is_err(),
                "{path}"
            );
        }
        std::fs::hard_link(p.join("valid"), p.join("hard")).unwrap();
        assert!(root.snapshot(Path::new("valid"), 100).is_err());
        std::fs::remove_file(p.join("hard")).unwrap();
        assert!(root.snapshot(Path::new("valid"), 5).is_err());
        let mut snap = root.snapshot(Path::new("valid"), 100).unwrap();
        std::fs::write(p.join("valid"), b"changed source").unwrap();
        let mut data = String::new();
        snap.file.read_to_string(&mut data).unwrap();
        assert_eq!(data, "verified bytes");
        drop(snap);
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(staging_path).unwrap();
    }

    #[test]
    fn a_junction_that_points_outside_the_root_is_never_followed() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        let outside = fixture();
        std::fs::write(outside.join("secret"), b"outside").unwrap();
        junction(&p.join("planted"), &outside);
        assert_eq!(
            std::fs::read(p.join("planted").join("secret")).unwrap(),
            b"outside"
        );
        assert!(root.snapshot(Path::new(r"planted\secret"), 100).is_err());
        assert!(root
            .create_new(&staging, Path::new(r"planted\file"), b"no")
            .is_err());
        assert!(root
            .create_new(&staging, Path::new(r"planted\new\file"), b"no")
            .is_err());
        assert!(Root::open(&p.join("planted"), None).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        root.create_new(&staging, Path::new(r"piloto\deep\resultado.md"), b"ok")
            .unwrap();
        assert_eq!(
            std::fs::read(p.join("piloto").join("deep").join("resultado.md")).unwrap(),
            b"ok"
        );
        let nine = (0..9)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("\\")
            + "\\f";
        assert!(root.create_new(&staging, Path::new(&nine), b"no").is_err());
        std::fs::remove_dir(p.join("planted")).unwrap();
        for d in [p, staging_path, outside] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn a_changed_root_is_denied_and_new_entries_belong_to_the_user_alone() {
        let p = fixture();
        let root = Root::open(&p, None).unwrap();
        let staging_path = fixture();
        let staging = Root::open(&staging_path, None).unwrap();
        let wrong = Identity {
            device: root.identity().device,
            inode: root.identity().inode + 1,
        };
        assert!(Root::open(&p, Some(&wrong)).is_err());
        assert!(Root::open(&p, Some(root.identity())).is_ok());
        root.create_directory(Path::new("job")).unwrap();
        assert_eq!(
            root.create_directory(Path::new("job")).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert!(root.create_directory(Path::new(r"a\b")).is_err());
        let job = Root::open(&p.join("job"), None).unwrap();
        assert_ne!(job.identity(), root.identity());
        root.create_new(&staging, Path::new("note.txt"), b"mine")
            .unwrap();
        assert!(grants(&p) > 1);
        assert_eq!(grants(&p.join("job")), 1);
        assert_eq!(grants(&p.join("note.txt")), 1);
        let note = root.identity_of(Path::new("note.txt")).unwrap();
        assert_eq!(root.identity_of(Path::new("NOTE.TXT")).unwrap(), note);
        assert_ne!(root.identity_of(Path::new("job")).unwrap(), note);
        std::fs::hard_link(p.join("note.txt"), p.join("alias")).unwrap();
        assert_eq!(root.identity_of(Path::new("alias")).unwrap(), note);
        std::fs::remove_file(p.join("alias")).unwrap();
        assert!(root.identity_of(Path::new("missing")).is_err());
        assert!(root.identity_of(Path::new(r"..\note.txt")).is_err());
        assert!(root.snapshot(Path::new("note.txt"), 3).is_err());
        assert_eq!(root.snapshot(Path::new("note.txt"), 4).unwrap().bytes, 4);
        drop(job);
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(staging_path).unwrap();
    }
}
