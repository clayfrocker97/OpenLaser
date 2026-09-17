// SPDX-License-Identifier: GPL-3.0-or-later

//! Gives the Windows executable its native icon and application metadata.

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/OpenLaser.ico");
    println!("cargo:rerun-if-env-changed=WINDRES");
    println!("cargo:rerun-if-env-changed=AR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let mut resource = winresource::WindowsResource::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && std::env::var("PROFILE").as_deref() == Ok("release")
    {
        // Release bundles omit PDBs, including references to SDK debug files.
        println!("cargo:rustc-link-arg=/DEBUG:NONE");
    }
    resource
        .set_icon("assets/OpenLaser.ico")
        .set("ProductName", "OpenLaser")
        .set("ProductVersion", env!("CARGO_PKG_VERSION"))
        .set("FileVersion", env!("CARGO_PKG_VERSION"))
        .set("FileDescription", "OpenLaser")
        .set("OriginalFilename", "OpenLaser.exe")
        .set("LegalCopyright", "OpenLaser contributors. GPL-3.0-or-later.");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") && !cfg!(windows) {
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".to_owned());
        resource.set_windres_path(
            &std::env::var("WINDRES").unwrap_or_else(|_| format!("{arch}-w64-mingw32-windres")),
        );
        resource
            .set_ar_path(&std::env::var("AR").unwrap_or_else(|_| format!("{arch}-w64-mingw32-ar")));
    }
    resource.compile()
}
