//! Builds Mallet's icon and version details into the Windows executable, so Explorer, the
//! taskbar and Alt+Tab show the mallet before the window has even opened.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let icon = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
        .join("../../branding/mallet.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = fs::canonicalize(&icon).expect("branding/mallet.ico exists; run branding/make_icons.py");
    let version = env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let numbers: Vec<u16> = version
        .split(['.', '-', '+'])
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    let [major, minor, patch] = [0, 1, 2].map(|index| numbers.get(index).copied().unwrap_or(0));
    // Resource scripts want backslashes doubled, and rc.exe does not understand the
    // \\?\ prefix canonicalize adds.
    let icon = icon.display().to_string();
    let icon = icon.trim_start_matches(r"\\?\").replace('\\', "\\\\");
    let script = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "FileDescription", "Mallet"
            VALUE "ProductName", "Mallet"
            VALUE "FileVersion", "{version}"
            VALUE "ProductVersion", "{version}"
            VALUE "OriginalFilename", "mallet.exe"
            VALUE "InternalName", "mallet"
            VALUE "Comments", "A Hammer-style level editor for Source engine maps"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("mallet.rc");
    fs::write(&out, script).expect("the resource script can be written");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("the icon resource compiles");
}
