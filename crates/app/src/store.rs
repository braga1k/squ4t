//! The ledger on disk: `run()` of store.py with Windows file primitives.
//!
//! - One exclusive `LockFileEx` lock on `ledger.lock` serialises every operation, across
//!   processes (the tray app and the `squ4t-cli` command) as `fcntl.flock` did across monitors.
//! - A changed ledger is written to a new temporary file in the same folder, flushed with
//!   `FlushFileBuffers`, and moved over `ledger.json` with `MoveFileExW(REPLACE_EXISTING |
//!   WRITE_THROUGH)`. A failure removes the temporary file and leaves the ledger untouched.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use squ4t_core::ledger::{self, Refused, Report};
use squ4t_core::pyjson::Num;
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::{
    LockFileEx, MoveFileExW, UnlockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows_sys::Win32::System::IO::OVERLAPPED;

/// Why an operation did not complete. Like store.py, the interface shows one message for all.
#[derive(Debug)]
pub enum Failure {
    Refused(Refused),
    Io(std::io::Error),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Refused(r) => write!(f, "{}", r.0),
            Failure::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Io(e)
    }
}

#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

/// `%LOCALAPPDATA%\squ4t`, or `SQU4T_DATA_DIR` when it is set.
pub fn default_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SQU4T_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    local_app_data().join("squ4t")
}

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// The folder the first builds used, before the app was named squ4t.
pub fn legacy_dir() -> PathBuf {
    local_app_data().join("squat-break")
}

/// Brings the ledger over from the first builds' folder: copied, under that folder's lock, only
/// when squ4t has no ledger yet and no data folder was chosen with SQU4T_DATA_DIR. The original
/// stays where it was, as a backup. True when a ledger was copied.
pub fn migrate_legacy_ledger() -> std::io::Result<bool> {
    if std::env::var_os("SQU4T_DATA_DIR").is_some_and(|d| !d.is_empty()) {
        return Ok(false);
    }
    let (old, new) = (legacy_dir(), default_dir());
    let (from, to) = (old.join("ledger.json"), new.join("ledger.json"));
    if to.exists() || !from.exists() {
        return Ok(false);
    }
    fs::create_dir_all(&new)?;
    let _old_lock = Lock::wait(&old.join("ledger.lock"))?;
    let _new_lock = Lock::wait(&new.join("ledger.lock"))?;
    if to.exists() {
        return Ok(false);
    }
    // Through a temporary name, so a half-copied ledger is never read as the real one.
    let temporary = new.join(".ledger-migrate.tmp");
    fs::copy(&from, &temporary)?;
    OpenOptions::new().write(true).open(&temporary)?.sync_all()?; // FlushFileBuffers needs write access
    fs::rename(&temporary, &to)?;
    Ok(true)
}

pub fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// An exclusive lock on a whole file, released when dropped.
pub struct Lock {
    file: File,
}

impl Lock {
    fn take(path: &Path, wait: bool) -> std::io::Result<Option<Lock>> {
        let file = OpenOptions::new().read(true).append(true).create(true).open(path)?;
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let flags = LOCKFILE_EXCLUSIVE_LOCK | if wait { 0 } else { LOCKFILE_FAIL_IMMEDIATELY };
        // SAFETY: a valid handle and a zeroed OVERLAPPED (offset 0) for a synchronous lock.
        let ok = unsafe { LockFileEx(file.as_raw_handle() as HANDLE, flags, 0, u32::MAX, u32::MAX, &mut overlapped) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            // ERROR_LOCK_VIOLATION
            if !wait && error.raw_os_error() == Some(33) {
                return Ok(None);
            }
            return Err(error);
        }
        Ok(Some(Lock { file }))
    }
    /// Blocks until the lock is ours.
    pub fn wait(path: &Path) -> std::io::Result<Lock> {
        Ok(Lock::take(path, true)?.expect("a blocking lock is granted or fails"))
    }
    /// `None` when another process holds it.
    pub fn try_take(path: &Path) -> std::io::Result<Option<Lock>> {
        Lock::take(path, false)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        // SAFETY: unlocking the range locked in `take`; the handle closes right after.
        unsafe { UnlockFileEx(self.file.as_raw_handle() as HANDLE, 0, u32::MAX, u32::MAX, &mut overlapped) };
    }
}

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Seconds since the Unix epoch, as `time.time()` gives them.
pub fn now() -> Num {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    Num::Float(d.as_secs_f64())
}

/// The local calendar date of a Unix time, as `datetime.fromtimestamp(now).date().isoformat()`.
pub fn local_day(now: Num) -> String {
    let ticks = ((now.as_f64() + 11_644_473_600.0) * 10_000_000.0) as u64;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: plain out-parameters; a null time zone means the current one.
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc);
        SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local);
    }
    format!("{:04}-{:02}-{:02}", local.wYear, local.wMonth, local.wDay)
}

impl Store {
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn ledger_path(&self) -> PathBuf {
        self.dir.join("ledger.json")
    }
    pub fn lock_path(&self) -> PathBuf {
        self.dir.join("ledger.lock")
    }

    /// One operation now, in the local date.
    pub fn run(&self, action: &str, args: &[String]) -> Result<Report, Failure> {
        let now = now();
        self.run_at(action, args, now, &local_day(now))
    }

    pub fn run_at(&self, action: &str, args: &[String], now: Num, today: &str) -> Result<Report, Failure> {
        fs::create_dir_all(&self.dir)?;
        let _lock = Lock::wait(&self.lock_path())?;
        let path = self.ledger_path();
        let existing = match File::open(&path) {
            Ok(mut file) => {
                if file.metadata()?.len() > ledger::MAX_LEDGER_BYTES {
                    return Err(Failure::Refused(Refused("Ledger too large")));
                }
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let (report, write) = ledger::run(existing.as_deref(), action, args, now, today).map_err(Failure::Refused)?;
        if let Some(bytes) = write {
            self.replace(&bytes)?;
        }
        Ok(report)
    }

    fn replace(&self, bytes: &[u8]) -> std::io::Result<()> {
        // A unique new name: a killed writer may have left a file under a reused process id.
        let mut temporary = PathBuf::new();
        let mut created = None;
        for _ in 0..16 {
            let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
            temporary = self.dir.join(format!(".ledger-{}-{n}-{nanos:09}.tmp", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&temporary) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                other => {
                    created = Some(other);
                    break;
                }
            }
        }
        let file = created.unwrap_or_else(|| Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists)))?;
        let result = (|| {
            let mut out = file;
            out.write_all(bytes)?;
            out.sync_all()?; // FlushFileBuffers
            drop(out);
            #[cfg(feature = "test-hooks")]
            test_hooks::pause_before_rename();
            let (from, to) = (wide(&temporary), wide(&self.ledger_path()));
            // SAFETY: NUL-terminated wide paths.
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

impl Store {
    /// Removes temporary files left by a process killed mid-write. Under the lock, so it never
    /// touches a write in progress.
    pub fn remove_stale_temporaries(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let _lock = Lock::wait(&self.lock_path())?;
        remove_temporaries(&self.dir);
        Ok(())
    }
}

fn remove_temporaries(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".ledger-") && name.ends_with(".tmp") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(feature = "test-hooks")]
mod test_hooks {
    pub fn pause_before_rename() {
        if let Some(ms) = std::env::var("SQUAT_TEST_PAUSE_BEFORE_RENAME_MS").ok().and_then(|v| v.parse().ok()) {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }
}
