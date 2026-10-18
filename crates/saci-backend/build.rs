//! Embeds the built web console (docs/web-ui.md) in every build, debug or
//! release: `web/dist` when it has been built, or the directory
//! `SACI_WEB_DIST` names. Without either the backend serves a page that
//! says how to build the console.

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

/// The client directory to embed: the one `SACI_WEB_DIST` names, which
/// must be built, or `web/dist` when it is. Registers what Cargo watches.
fn client_dir() -> io::Result<Option<PathBuf>> {
    println!("cargo::rerun-if-env-changed=SACI_WEB_DIST");
    let named = env::var_os("SACI_WEB_DIST").filter(|value| !value.is_empty());
    let dist = named.as_ref().map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist"),
        PathBuf::from,
    );
    // `npm run build` then triggers a rebuild. A missing path would make
    // Cargo rebuild every time, so until web/dist exists watch web/.
    let watched = match dist.parent() {
        Some(parent) if !dist.is_dir() => parent,
        _ => &dist,
    };
    println!("cargo::rerun-if-changed={}", watched.display());
    let built = dist.join("index.html").is_file();
    if named.is_some() && !built {
        return Err(io::Error::other(format!(
            "SACI_WEB_DIST={} has no index.html; run `npm run build` in web/ first",
            dist.display()
        )));
    }
    Ok(built.then_some(dist))
}

fn main() -> io::Result<()> {
    let mut found = Vec::new();
    if let Some(dist) = client_dir()? {
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
