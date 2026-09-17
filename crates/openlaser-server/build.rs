// SPDX-License-Identifier: GPL-3.0-or-later

//! Embeds the already-built UI when producing a standalone executable.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_EMBEDDED_UI");
    if std::env::var_os("CARGO_FEATURE_EMBEDDED_UI").is_none() {
        return Ok(());
    }
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").ok_or("manifest directory")?)
        .join("../..");
    let dist = root.join("ui/dist");
    if !dist.join("index.html").is_file() {
        return Err(
            "embedded-ui needs a built UI; run `npm ci` and `npm run build` in ui/ first".into()
        );
    }
    let mut files = Vec::new();
    collect(&dist, &dist, &mut files)?;
    files.push(("licenses/OpenLaser-GPL-3.0.txt".to_owned(), root.join("LICENSE")));
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut generated = String::from("const ASSETS: &[(&str, &[u8])] = &[\n");
    for (name, path) in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let path = path.to_str().ok_or("UI path is not UTF-8")?;
        writeln!(generated, "({name:?}, include_bytes!({path:?})),")?;
    }
    generated.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("build output directory")?);
    std::fs::write(out.join("ui-assets.rs"), generated)?;
    Ok(())
}

fn collect(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else if path.is_file() {
            let name =
                path.strip_prefix(root)?.to_str().ok_or("UI path is not UTF-8")?.replace('\\', "/");
            files.push((name, path));
        }
    }
    Ok(())
}
