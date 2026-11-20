//! Compiles the panel and gives both programs a Windows resource: the version information, whose
//! FileDescription is the name Windows shows on squ4t's notifications and in Task Manager, and
//! (for squ4t.exe) the ember ring as the program icon. The resource file is written here byte by
//! byte, so the build needs no resource compiler; MSVC's linker takes a .res file as an input.

use std::path::Path;

fn main() {
    slint_build::compile_with_config("ui/panel.slint", slint_build::CompilerConfiguration::new()).expect("ui/panel.slint");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let version = std::env::var("CARGO_PKG_VERSION").expect("version");
    for (bin, description, icon) in [("squ4t", "squ4t", true), ("squ4t-cli", "squ4t command line", false)] {
        let path = Path::new(&out).join(format!("{bin}.res"));
        std::fs::write(&path, resources(&version, description, &format!("{bin}.exe"), icon)).expect("write the resource file");
        println!("cargo:rustc-link-arg-bin={bin}={}", path.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
}

// ------------------------------------------------------------ the .res file

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const EN_US: u16 = 0x0409;

fn pad4(b: &mut Vec<u8>) {
    while b.len() % 4 != 0 {
        b.push(0);
    }
}

/// One resource: its header (numeric type and name) and its data, padded to four bytes.
fn entry(b: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    b.extend((data.len() as u32).to_le_bytes());
    b.extend(32u32.to_le_bytes()); // header size
    for v in [0xFFFF, kind, 0xFFFF, name] {
        b.extend(v.to_le_bytes());
    }
    b.extend(0u32.to_le_bytes()); // data version
    b.extend(flags.to_le_bytes());
    b.extend((if kind == 0 { 0 } else { EN_US }).to_le_bytes());
    b.extend(0u32.to_le_bytes()); // version
    b.extend(0u32.to_le_bytes()); // characteristics
    b.extend(data);
    pad4(b);
}

fn resources(version: &str, description: &str, file: &str, icon: bool) -> Vec<u8> {
    let mut b = Vec::new();
    entry(&mut b, 0, 0, 0, &[]); // every .res file starts with an empty entry
    if icon {
        const SIZES: [u32; 7] = [16, 20, 24, 32, 40, 48, 64];
        let mut group = Vec::new();
        group.extend([0u16, 1, SIZES.len() as u16].iter().flat_map(|v| v.to_le_bytes()));
        for (i, &size) in SIZES.iter().enumerate() {
            let image = icon_image(size);
            entry(&mut b, RT_ICON, i as u16 + 1, 0x1010, &image);
            group.extend([size as u8, size as u8, 0, 0]);
            group.extend(1u16.to_le_bytes());
            group.extend(32u16.to_le_bytes());
            group.extend((image.len() as u32).to_le_bytes());
            group.extend((i as u16 + 1).to_le_bytes());
        }
        entry(&mut b, RT_GROUP_ICON, 1, 0x1030, &group);
    }
    entry(&mut b, RT_VERSION, 1, 0x0030, &version_info(version, description, file));
    b
}

// ------------------------------------------------------------ version information

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect()
}

/// A VS_VERSIONINFO node: length, value length, type, key, value, children, each part aligned.
fn node(key: &str, value: &[u8], value_len: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0];
    b.extend(value_len.to_le_bytes());
    b.extend((text as u16).to_le_bytes());
    b.extend(utf16z(key));
    pad4(&mut b);
    b.extend(value);
    for child in children {
        pad4(&mut b);
        b.extend(child);
    }
    let length = b.len() as u16;
    b[..2].copy_from_slice(&length.to_le_bytes());
    b
}

fn string(key: &str, value: &str) -> Vec<u8> {
    let v = utf16z(value);
    node(key, &v, (v.len() / 2) as u16, true, &[])
}

fn version_info(version: &str, description: &str, file: &str) -> Vec<u8> {
    let mut parts = version.split(['.', '-']).map(|p| p.parse::<u16>().unwrap_or(0)).chain(std::iter::repeat(0));
    let v: Vec<u32> = (0..4).map(|_| parts.next().unwrap_or(0) as u32).collect();
    let (ms, ls) = ((v[0] << 16) | v[1], (v[2] << 16) | v[3]);
    let mut fixed = Vec::new();
    // Signature, struct version, file and product versions, flags mask, flags, NT Win32, app.
    for x in [0xFEEF04BDu32, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0] {
        fixed.extend(x.to_le_bytes());
    }
    let table = node(
        "040904B0",
        &[],
        0,
        true,
        &[
            string("CompanyName", "braga1k"),
            string("FileDescription", description),
            string("FileVersion", version),
            string("InternalName", file.trim_end_matches(".exe")),
            string("LegalCopyright", "Copyright (c) 2026 braga1k. MIT licence."),
            string("OriginalFilename", file),
            string("ProductName", "squ4t"),
            string("ProductVersion", version),
        ],
    );
    let strings = node("StringFileInfo", &[], 0, true, &[table]);
    let translation: Vec<u8> = [EN_US, 0x04B0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let vars = node("VarFileInfo", &[], 0, true, &[node("Translation", &translation, 4, false, &[])]);
    node("VS_VERSION_INFO", &fixed, fixed.len() as u16, false, &[strings, vars])
}

// ------------------------------------------------------------ the icon

/// The ember ring (the dark theme's accent, #F07A3A), full: a 32-bit DIB with its empty mask.
fn icon_image(size: u32) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [40u32, size, size * 2] {
        b.extend(v.to_le_bytes());
    }
    b.extend(1u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    for v in [0u32, size * size * 4, 0, 0, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    let s = size as f64;
    let outer = s * 0.46;
    let inner = outer - (s * 0.14).max(1.6);
    // Rows bottom-up, four by four samples per pixel.
    for y in (0..size).rev() {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = x as f64 + (sx as f64 + 0.5) / 4.0 - s / 2.0;
                    let dy = y as f64 + (sy as f64 + 0.5) / 4.0 - s / 2.0;
                    let r = (dx * dx + dy * dy).sqrt();
                    hits += (r <= outer && r >= inner) as u32;
                }
            }
            let a = (hits * 255 / 16) as u8;
            b.extend([0x3A, 0x7A, 0xF0, a]);
        }
    }
    let mask_row = size.div_ceil(32) * 4;
    b.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    b
}
