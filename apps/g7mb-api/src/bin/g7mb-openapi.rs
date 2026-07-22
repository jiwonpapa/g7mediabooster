//! Generates or verifies the committed HTTP OpenAPI contract.

use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, bail};

fn main() -> anyhow::Result<()> {
    let mut arguments = env::args().skip(1);
    let action = arguments.next().context("expected `check` or `write`")?;
    if arguments.next().is_some() {
        bail!("expected exactly one OpenAPI action");
    }
    let path = workspace_root()?.join("openapi/g7mediabooster-v1.json");
    let generated = format!("{}\n", g7mb_api::openapi_json()?);
    match action.as_str() {
        "write" => {
            let parent = path
                .parent()
                .context("OpenAPI output path has no parent directory")?;
            fs::create_dir_all(parent)?;
            fs::write(path, generated)?;
            Ok(())
        }
        "check" => {
            let committed = fs::read_to_string(&path)
                .with_context(|| format!("missing OpenAPI snapshot: {}", path.display()))?;
            if committed != generated {
                bail!("OpenAPI drift detected; run `cargo xtask openapi write`");
            }
            Ok(())
        }
        _ => bail!("unknown OpenAPI action `{action}`; expected `check` or `write`"),
    }
}

fn workspace_root() -> anyhow::Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .context("API manifest directory is not inside the workspace")
}
