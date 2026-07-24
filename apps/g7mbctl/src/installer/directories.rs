//! System account and filesystem layout preparation.

use std::path::Path;

use super::{SERVICE_USER, create_directory, ensure_directory, run_checked};

pub(super) fn prepare_directories() -> anyhow::Result<()> {
    for (path, mode) in [
        ("/etc/g7mediabooster", 0o750),
        ("/etc/g7mediabooster/credentials", 0o700),
        ("/var/lib/g7mediabooster", 0o750),
        ("/var/lib/g7mediabooster/tmp", 0o700),
        ("/var/lib/g7mediabooster/backups", 0o700),
    ] {
        create_directory(Path::new(path), mode)?;
    }
    run_checked("chown", &["root:root", "/etc/g7mediabooster/credentials"])?;
    run_checked("chgrp", &[SERVICE_USER, "/etc/g7mediabooster"])?;
    for path in [
        "/var/lib/g7mediabooster",
        "/var/lib/g7mediabooster/tmp",
        "/var/lib/g7mediabooster/backups",
    ] {
        run_checked("chown", &["g7mediabooster:g7mediabooster", path])?;
    }
    Ok(())
}

pub(super) fn prepare_install_directories() -> anyhow::Result<()> {
    for path in [
        "/usr/local/bin",
        "/usr/local/libexec",
        "/etc/systemd/system",
    ] {
        ensure_directory(Path::new(path), 0o755)?;
    }
    for path in [
        "/usr/local/share/g7mediabooster",
        "/usr/local/share/g7mediabooster/gnuboard7",
        "/usr/local/share/g7mediabooster/nginx",
    ] {
        create_directory(Path::new(path), 0o755)?;
    }
    Ok(())
}
