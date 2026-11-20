//! Updates, from the project's own GitHub releases and nowhere else.
//!
//! A check asks GitHub for the latest release. An install downloads that release's zip, requires
//! its SHA-256 to match the release's `SHA256SUMS.txt`, and swaps the two programs in place: the
//! running files are renamed aside (Windows allows that), the new ones take their names, and the
//! next start removes the old ones. Nothing is sent but the request itself.
//!
//! The checksum comes from the same release as the zip, so it catches a broken download, not a
//! release that was tampered with at its source. The programs are not code-signed.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use squ4t_core::pyjson::{self, Value};

pub const LATEST: &str = "https://api.github.com/repos/braga1k/squ4t/releases/latest";
/// Every file an update downloads must live under this address.
pub const DOWNLOADS: &str = "https://github.com/braga1k/squ4t/releases/download/";
pub const PROGRAMS: [&str; 2] = ["squ4t.exe", "squ4t-cli.exe"];
const SUMS: &str = "SHA256SUMS.txt";
const MAX_JSON: usize = 2 << 20;
const MAX_ZIP: usize = 64 << 20;

pub type Version = (u32, u32, u32);

/// "1.2.3" or "v1.2.3".
pub fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let version = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(version)
}

pub fn current() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

pub fn version_text(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub zip_name: String,
    pub zip_url: String,
    pub sums_url: String,
}

fn text(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) => String::from_utf8(s.clone()).ok(),
        _ => None,
    }
}

/// GitHub's description of a release. None unless it is a full release that carries this
/// platform's zip and the checksums, both under [`DOWNLOADS`].
pub fn parse_release(json: &[u8]) -> Option<Release> {
    let release = pyjson::loads(json).ok()?;
    if matches!(release.get("draft"), Some(Value::Bool(true))) || matches!(release.get("prerelease"), Some(Value::Bool(true))) {
        return None;
    }
    let version = parse_version(&text(release.get("tag_name"))?)?;
    let zip_name = format!("squ4t-{}-windows-x64.zip", version_text(version));
    let Some(Value::Array(assets)) = release.get("assets") else { return None };
    let url_of = |name: &str| {
        assets
            .iter()
            .find(|a| text(a.get("name")).as_deref() == Some(name))
            .and_then(|a| text(a.get("browser_download_url")))
            .filter(|url| url.starts_with(DOWNLOADS) && !url.contains(".."))
    };
    Some(Release { version, zip_url: url_of(&zip_name)?, sums_url: url_of(SUMS)?, zip_name })
}

/// The hash a `sha256sum` listing gives for `name`, in lower case.
pub fn expected_sum(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let ok = file.trim().trim_start_matches('*') == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| hash.to_ascii_lowercase())
    })
}

// ---------------------------------------------------------------- SHA-256 (FIPS 180-4)

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
    0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// The SHA-256 of `data`, as 64 lower-case hex digits.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

// ---------------------------------------------------------------- the network (WinHTTP)

/// One HTTPS GET, following redirects, refusing more than `limit` bytes.
pub fn https_get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Networking::WinHttp::*;

    use crate::wstr;

    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: a handle WinHTTP gave us, closed once.
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }
    let fail = |what: &str| format!("{what}: {}", std::io::Error::last_os_error());

    let rest = url.strip_prefix("https://").ok_or("not an https address")?;
    let (host, path) = rest.split_once('/').ok_or("no path")?;
    let (agent, host, path, verb) = (wstr("squ4t-updater"), wstr(host), wstr(&format!("/{path}")), wstr("GET"));
    let headers = wstr("Accept: application/vnd.github+json, application/octet-stream\r\n");
    // SAFETY: NUL-terminated strings; every handle is checked and closed by `Handle`.
    unsafe {
        let session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0));
        if session.0.is_null() {
            return Err(fail("WinHttpOpen"));
        }
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 30_000);
        let connection = Handle(WinHttpConnect(session.0, host.as_ptr(), 443, 0));
        if connection.0.is_null() {
            return Err(fail("WinHttpConnect"));
        }
        let request = Handle(WinHttpOpenRequest(connection.0, verb.as_ptr(), path.as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if request.0.is_null() {
            return Err(fail("WinHttpOpenRequest"));
        }
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, std::ptr::null(), 0, 0, 0) == 0 {
            return Err(fail("no connection"));
        }
        if WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0 {
            return Err(fail("no answer"));
        }
        let (mut status, mut size) = (0u32, 4u32);
        WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, std::ptr::null(), (&mut status as *mut u32).cast(), &mut size, std::ptr::null_mut());
        if status != 200 {
            return Err(format!("the server answered {status}"));
        }
        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(fail("the download stopped"));
            }
            if available == 0 {
                return Ok(body);
            }
            if body.len() + available as usize > limit {
                return Err("the file is larger than expected".into());
            }
            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(request.0, body[start..].as_mut_ptr().cast(), available, &mut read) == 0 {
                return Err(fail("the download stopped"));
            }
            body.truncate(start + read as usize);
        }
    }
}

/// The latest release, when it is newer than this program.
pub fn check() -> Result<Option<Release>, String> {
    let release = parse_release(&https_get(LATEST, MAX_JSON)?).ok_or("the latest release has no update for this program")?;
    Ok((release.version > current()).then_some(release))
}

// ---------------------------------------------------------------- the swap

fn aside(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    path.with_file_name(name)
}

/// Removes what an earlier update set aside. The files are free once that program has exited.
pub fn remove_old(dir: &Path) {
    for name in PROGRAMS {
        let _ = std::fs::remove_file(aside(&dir.join(name)));
    }
}

/// Puts the programs in `from` in the place of those in `dir`. On a failure, every program is
/// the one it was before.
pub fn swap(from: &Path, dir: &Path) -> std::io::Result<()> {
    // Copy first, next to the targets: after this, only renames on one volume are left.
    let staged: Vec<(PathBuf, PathBuf)> = PROGRAMS.iter().map(|name| (dir.join(format!("{name}.new")), dir.join(name))).collect();
    let cleanup = |staged: &[(PathBuf, PathBuf)]| staged.iter().for_each(|(new, _)| drop(std::fs::remove_file(new)));
    for (name, (new, _)) in PROGRAMS.iter().zip(&staged) {
        if let Err(e) = std::fs::copy(from.join(name), new) {
            cleanup(&staged);
            return Err(e);
        }
    }
    let mut done: Vec<&(PathBuf, PathBuf)> = Vec::new();
    for pair in &staged {
        let (new, target) = pair;
        let old = aside(target);
        let _ = std::fs::remove_file(&old);
        let step = (|| {
            if target.exists() {
                std::fs::rename(target, &old)?;
            }
            std::fs::rename(new, target).inspect_err(|_| drop(std::fs::rename(&old, target)))
        })();
        if let Err(e) = step {
            for (_, target) in done {
                let _ = std::fs::remove_file(target);
                let _ = std::fs::rename(aside(target), target);
            }
            cleanup(&staged);
            return Err(e);
        }
        done.push(pair);
    }
    Ok(())
}

/// Downloads `release`, checks it, and swaps it into `dir`. The caller restarts the program.
pub fn install(release: &Release, dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let listing = String::from_utf8(https_get(&release.sums_url, MAX_JSON)?).map_err(|_| "the checksums are not text")?;
    let expected = expected_sum(&listing, &release.zip_name).ok_or("the release lists no checksum for its zip")?;
    let zip = https_get(&release.zip_url, MAX_ZIP)?;
    if sha256_hex(&zip) != expected {
        return Err("the download does not match its checksum".into());
    }
    let work = std::env::temp_dir().join(format!("squ4t-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let result = (|| {
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        let file = work.join(&release.zip_name);
        std::fs::write(&file, &zip).map_err(|e| e.to_string())?;
        // Windows' own tar reads zip files (Windows 10 1803 and later). CREATE_NO_WINDOW.
        let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32").join("tar.exe");
        let status = std::process::Command::new(tar).arg("-xf").arg(&file).arg("-C").arg(&work).args(PROGRAMS).creation_flags(0x0800_0000).status();
        if !status.map_err(|e| format!("could not unpack the update: {e}"))?.success() {
            return Err("could not unpack the update".to_string());
        }
        swap(&work, dir).map_err(|e| format!("could not replace the programs: {e}"))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

// ---------------------------------------------------------------- the user's choice

/// Automatic updates are off until the user turns them on: the app then checks once a day.
pub fn auto_enabled(data: &Path) -> bool {
    data.join("auto-update").exists()
}

pub fn set_auto(data: &Path, on: bool) -> std::io::Result<()> {
    let marker = data.join("auto-update");
    if on {
        std::fs::write(marker, b"")
    } else {
        match std::fs::remove_file(marker) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// True when the last automatic check is a day old, or there was none.
pub fn check_due(data: &Path) -> bool {
    let checked = std::fs::metadata(data.join("update-checked")).and_then(|m| m.modified());
    checked.map_or(true, |at| SystemTime::now().duration_since(at).map_or(true, |age| age >= Duration::from_secs(24 * 3600)))
}

pub fn mark_checked(data: &Path) {
    let _ = std::fs::write(data.join("update-checked"), b"");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.0.1"), Some((1, 0, 1)));
        assert_eq!(parse_version("1.10.0"), Some((1, 10, 0)));
        assert!(parse_version("1.10.0").unwrap() > parse_version("1.9.9").unwrap());
        for bad in ["", "1.0", "1.0.0.1", "1.0.x", "latest"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn sha256_known_answers() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        assert_eq!(sha256_hex(&vec![b'a'; 1_000_000]), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    const GITHUB: &str = r#"{"tag_name": "v1.0.1", "draft": false, "prerelease": false, "assets": [
        {"name": "SHA256SUMS.txt", "browser_download_url": "https://github.com/braga1k/squ4t/releases/download/v1.0.1/SHA256SUMS.txt"},
        {"name": "squ4t-1.0.1-windows-x64.zip", "browser_download_url": "https://github.com/braga1k/squ4t/releases/download/v1.0.1/squ4t-1.0.1-windows-x64.zip"}]}"#;

    #[test]
    fn a_release_as_github_describes_it() {
        let r = parse_release(GITHUB.as_bytes()).unwrap();
        assert_eq!((r.version, r.zip_name.as_str()), ((1, 0, 1), "squ4t-1.0.1-windows-x64.zip"));
        assert!(r.zip_url.ends_with("/v1.0.1/squ4t-1.0.1-windows-x64.zip") && r.sums_url.ends_with("/SHA256SUMS.txt"));
    }

    #[test]
    fn releases_that_are_refused() {
        // A file from another address, a pre-release, a release without checksums, and no JSON at all.
        let elsewhere = GITHUB.replace("https://github.com/braga1k/squ4t/releases/download/v1.0.1/squ4t-1.0.1", "https://example.com/squ4t-1.0.1");
        let pre = GITHUB.replace(r#""prerelease": false"#, r#""prerelease": true"#);
        let no_sums = GITHUB.replace("SHA256SUMS.txt\"", "notes.txt\"");
        for bad in [elsewhere.as_str(), pre.as_str(), no_sums.as_str(), "<html>", "{}"] {
            assert_eq!(parse_release(bad.as_bytes()), None, "{bad}");
        }
    }

    #[test]
    fn checksum_listings() {
        let listing = "c8cce5e175e1ba10de72d984d145c2a06168e1dd762701652ac79a02f2a5f005  squ4t-1.0.1-windows-x64.zip\n";
        assert_eq!(expected_sum(listing, "squ4t-1.0.1-windows-x64.zip").as_deref(), Some("c8cce5e175e1ba10de72d984d145c2a06168e1dd762701652ac79a02f2a5f005"));
        assert_eq!(expected_sum(listing, "squ4t-1.0.2-windows-x64.zip"), None);
        assert_eq!(expected_sum("abc  squ4t-1.0.1-windows-x64.zip", "squ4t-1.0.1-windows-x64.zip"), None);
    }

    #[test]
    fn swap_replaces_both_programs_and_keeps_the_old_ones_aside() {
        let root = std::env::temp_dir().join(format!("squ4t-swap-{}", std::process::id()));
        let (from, dir) = (root.join("new"), root.join("app"));
        for d in [&from, &dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        for name in PROGRAMS {
            std::fs::write(from.join(name), b"new").unwrap();
            std::fs::write(dir.join(name), b"old").unwrap();
        }
        swap(&from, &dir).unwrap();
        for name in PROGRAMS {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"new");
            assert_eq!(std::fs::read(dir.join(format!("{name}.old"))).unwrap(), b"old");
        }
        remove_old(&dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);

        // A missing program in the update leaves everything as it was.
        std::fs::remove_file(from.join("squ4t-cli.exe")).unwrap();
        std::fs::write(from.join("squ4t.exe"), b"newer").unwrap();
        assert!(swap(&from, &dir).is_err());
        for name in PROGRAMS {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), b"new");
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
