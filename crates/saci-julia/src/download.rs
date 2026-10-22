//! Downloads the Julia-1-ONNX files from Hugging Face.

use std::path::Path;

use anyhow::{Context, bail};
use tokio::io::AsyncWriteExt;

/// The model repository.
pub const REPOSITORY: &str = "SupersonicLabs/Julia-1-ONNX";
/// The commit this release is tested with.
pub const REVISION: &str = "82a2fadf8fccfccdc5fd4e1009ba8f1a265eb7a8";
/// The files `serve` and `parity` read. `model.onnx.data` holds the weights.
pub const FILES: [&str; 4] = [
    "model.onnx",
    "model.onnx.data",
    "tokenizer.json",
    "parity-cases.json",
];

/// Downloads each missing file of `revision` into `model_dir`. A file is
/// written as `NAME.part` and renamed when it is complete.
///
/// `token` is an optional Hugging Face token.
///
/// # Errors
///
/// Fails when a request fails or a file cannot be written.
pub async fn download(model_dir: &Path, revision: &str, token: Option<&str>) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(model_dir)
        .await
        .with_context(|| format!("cannot create {}", model_dir.display()))?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()?;
    for name in FILES {
        let target = model_dir.join(name);
        if tokio::fs::try_exists(&target).await? {
            tracing::info!(file = name, "already downloaded");
            continue;
        }
        let url = format!("https://huggingface.co/{REPOSITORY}/resolve/{revision}/{name}");
        fetch(&client, &url, token, &target).await?;
    }
    Ok(())
}

/// Writes `url` to `target` through `target.part`.
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    target: &Path,
) -> anyhow::Result<()> {
    tracing::info!(%url, "downloading");
    let mut request = client.get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let mut response = request
        .send()
        .await
        .with_context(|| format!("cannot get {url}"))?;
    if !response.status().is_success() {
        bail!("{url} returned {}", response.status());
    }
    let mut partial = target.as_os_str().to_owned();
    partial.push(".part");
    let partial = std::path::PathBuf::from(partial);
    let mut file = tokio::fs::File::create(&partial)
        .await
        .with_context(|| format!("cannot write {}", partial.display()))?;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&partial, target).await?;
    Ok(())
}
