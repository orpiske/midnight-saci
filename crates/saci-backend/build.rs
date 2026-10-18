//! Embeds the built web console (`web/dist`, see docs/web-ui.md) when
//! `SACI_WEB_DIST` names its directory at build time. Without it the
//! backend serves a page that says how to build the console.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

fn files(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files(&path, base, out)?;
        } else if let Ok(relative) = path.strip_prefix(base) {
            let name = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((name, fs::canonicalize(&path)?));
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    println!("cargo::rerun-if-env-changed=SACI_WEB_DIST");
    let mut found = Vec::new();
    if let Some(dist) = env::var_os("SACI_WEB_DIST").filter(|value| !value.is_empty()) {
        let dist = PathBuf::from(dist);
        println!("cargo::rerun-if-changed={}", dist.display());
        if !dist.join("index.html").is_file() {
            return Err(io::Error::other(format!(
                "SACI_WEB_DIST={} has no index.html; run `npm run build` in web/ first",
                dist.display()
            )));
        }
        files(&dist, &dist, &mut found)?;
        found.sort();
    }
    let mut code = String::from("&[\n");
    for (name, path) in &found {
        let _ = writeln!(
            code,
            "    Asset {{ path: {name:?}, bytes: include_bytes!({:?}) }},",
            path.display().to_string()
        );
    }
    code.push(']');
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR"))?);
    fs::write(out.join("console_assets.rs"), code)
}
