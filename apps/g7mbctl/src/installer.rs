//! Linux bundle installation and product-level service diagnostics.

use std::{
    collections::BTreeMap,
    env,
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpStream},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, bail};
use g7mb_config::Settings;
use sha2::{Digest as _, Sha256};

const SERVICE_USER: &str = "g7mediabooster";
const SERVICE_TARGET: &str = "g7mediabooster.target";
const INSTALLED_CTL: &str = "/usr/local/bin/g7mbctl";
const INSTALLED_CONFIG: &str = "/etc/g7mediabooster/g7mb.toml";
const INSTALLED_MODULE: &str =
    "/usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip";

const ACTIVE_UNITS: &[&str] = &[
    SERVICE_TARGET,
    "g7mediabooster-api.service",
    "g7mediabooster-worker.service",
    "g7mediabooster-cleanup.timer",
    "g7mediabooster-inventory.timer",
    "g7mediabooster-backup.timer",
];

#[derive(Clone, Copy)]
struct PayloadFile {
    source: &'static str,
    destination: &'static str,
    mode: u32,
    maximum_bytes: u64,
}

const PAYLOAD_FILES: &[PayloadFile] = &[
    PayloadFile {
        source: "bin/g7mbctl",
        destination: "/usr/local/bin/g7mbctl",
        mode: 0o755,
        maximum_bytes: 128 * 1024 * 1024,
    },
    PayloadFile {
        source: "bin/g7mb-api",
        destination: "/usr/local/bin/g7mb-api",
        mode: 0o755,
        maximum_bytes: 128 * 1024 * 1024,
    },
    PayloadFile {
        source: "bin/g7mb-worker",
        destination: "/usr/local/bin/g7mb-worker",
        mode: 0o755,
        maximum_bytes: 128 * 1024 * 1024,
    },
    PayloadFile {
        source: "libexec/g7mb-sandbox",
        destination: "/usr/local/libexec/g7mb-sandbox",
        mode: 0o755,
        maximum_bytes: 128 * 1024 * 1024,
    },
    PayloadFile {
        source: "VERSION",
        destination: "/usr/local/share/g7mediabooster/VERSION",
        mode: 0o644,
        maximum_bytes: 128,
    },
    PayloadFile {
        source: "MANIFEST.sha256",
        destination: "/usr/local/share/g7mediabooster/MANIFEST.sha256",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "INSTALL.md",
        destination: "/usr/local/share/g7mediabooster/INSTALL.md",
        mode: 0o644,
        maximum_bytes: 256 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/jiwonpapa-g7mediabooster.zip",
        destination: INSTALLED_MODULE,
        mode: 0o644,
        maximum_bytes: 32 * 1024 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/jiwonpapa-g7mediabooster.zip.sha256",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip.sha256",
        mode: 0o644,
        maximum_bytes: 1024,
    },
    PayloadFile {
        source: "gnuboard7/official-features-v1.json",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/official-features-v1.json",
        mode: 0o644,
        maximum_bytes: 256 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/verify-gnuboard7-media-contract.sh",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/verify-gnuboard7-media-contract.sh",
        mode: 0o755,
        maximum_bytes: 256 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/0001.patch",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/0001.patch",
        mode: 0o644,
        maximum_bytes: 2 * 1024 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/0002.patch",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/0002.patch",
        mode: 0o644,
        maximum_bytes: 2 * 1024 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/0003.patch",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/0003.patch",
        mode: 0o644,
        maximum_bytes: 2 * 1024 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/0004.patch",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/0004.patch",
        mode: 0o644,
        maximum_bytes: 2 * 1024 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/0005.patch",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/0005.patch",
        mode: 0o644,
        maximum_bytes: 2 * 1024 * 1024,
    },
    PayloadFile {
        source: "gnuboard7/0006.patch",
        destination: "/usr/local/share/g7mediabooster/gnuboard7/0006.patch",
        mode: 0o644,
        maximum_bytes: 2 * 1024 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster.target",
        destination: "/etc/systemd/system/g7mediabooster.target",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-api.service",
        destination: "/etc/systemd/system/g7mediabooster-api.service",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-worker.service",
        destination: "/etc/systemd/system/g7mediabooster-worker.service",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-cleanup.service",
        destination: "/etc/systemd/system/g7mediabooster-cleanup.service",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-cleanup.timer",
        destination: "/etc/systemd/system/g7mediabooster-cleanup.timer",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-inventory.service",
        destination: "/etc/systemd/system/g7mediabooster-inventory.service",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-inventory.timer",
        destination: "/etc/systemd/system/g7mediabooster-inventory.timer",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-backup.service",
        destination: "/etc/systemd/system/g7mediabooster-backup.service",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
    PayloadFile {
        source: "systemd/g7mediabooster-backup.timer",
        destination: "/etc/systemd/system/g7mediabooster-backup.timer",
        mode: 0o644,
        maximum_bytes: 64 * 1024,
    },
];

/// Options for the root-owned Linux bundle installation.
#[derive(Debug)]
pub(crate) struct InstallOptions {
    pub(crate) bundle_dir: Option<PathBuf>,
    pub(crate) force: bool,
    pub(crate) skip_setup: bool,
    pub(crate) skip_start: bool,
    pub(crate) skip_dependency_install: bool,
}

/// Installs an extracted release bundle and exposes one product-level target.
pub(crate) fn install(options: InstallOptions) -> anyhow::Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("서버 통합 설치는 systemd Linux에서만 지원합니다");
    }
    ensure_root()?;
    for (program, argument) in [
        ("systemctl", "--version"),
        ("useradd", "--help"),
        ("groupadd", "--help"),
        ("getent", "--help"),
        ("chown", "--help"),
        ("chgrp", "--help"),
    ] {
        require_command(program, argument)?;
    }
    ensure_native_runtime(options.skip_dependency_install)?;

    let bundle = resolve_bundle_dir(options.bundle_dir.as_deref())?;
    validate_bundle(&bundle)?;
    sandbox_doctor(&bundle.join("libexec/g7mb-sandbox"))?;
    prepare_service_account()?;
    prepare_install_directories()?;
    preflight_install_payload(&bundle, Path::new("/"), options.force)?;
    let target_was_active =
        command_succeeds("systemctl", &["is-active", "--quiet", SERVICE_TARGET])?;
    install_payload(&bundle, Path::new("/"), options.force)?;
    prepare_directories()?;
    run_checked("systemctl", &["daemon-reload"])?;

    let config_exists = Path::new(INSTALLED_CONFIG).is_file();
    if !config_exists && !options.skip_setup {
        run_checked(INSTALLED_CTL, &["setup"])?;
    }
    if !options.skip_start {
        if !Path::new(INSTALLED_CONFIG).is_file() {
            bail!(
                "설정이 없습니다. 먼저 `sudo g7mbctl setup`을 실행하거나 --skip-start를 사용하십시오"
            );
        }
        if target_was_active {
            run_checked("systemctl", &["restart", SERVICE_TARGET])?;
        } else {
            run_checked("systemctl", &["enable", "--now", SERVICE_TARGET])?;
        }
        let settings = Settings::load(Some(Path::new(INSTALLED_CONFIG)))
            .context("설치된 설정을 읽지 못했습니다")?;
        wait_until_ready(settings.server.bind_addr, Duration::from_secs(30))?;
        status(Path::new(INSTALLED_CONFIG))?;
    }

    println!("PASS install target={SERVICE_TARGET}");
    println!("PASS gnuboard7-module={INSTALLED_MODULE}");
    if options.skip_setup {
        println!("NEXT sudo g7mbctl setup");
    } else if options.skip_start {
        println!("NEXT sudo systemctl enable --now {SERVICE_TARGET}");
    } else {
        println!("NEXT sudo g7mbctl doctor");
    }
    Ok(())
}

/// Prints and validates the installed product state as one logical service.
pub(crate) fn status(config: &Path) -> anyhow::Result<Settings> {
    if !cfg!(target_os = "linux") {
        bail!("설치 상태 검사는 systemd Linux에서만 지원합니다");
    }
    let settings = Settings::load(Some(config)).context("설정 검증에 실패했습니다")?;
    for unit in ACTIVE_UNITS {
        let output = Command::new("systemctl")
            .args(["is-active", unit])
            .output()
            .with_context(|| format!("{unit} 상태를 확인하지 못했습니다"))?;
        let state = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() || state != "active" {
            bail!("{unit} 상태가 active가 아닙니다: {state}");
        }
    }
    check_http_ready(settings.server.bind_addr)?;
    println!("PASS status target=active api=ready worker=active timers=3");
    Ok(settings)
}

/// Executes the credential-free native capability gate.
pub(crate) fn sandbox_doctor(sandbox: &Path) -> anyhow::Result<()> {
    validate_regular_file(sandbox, 128 * 1024 * 1024)?;
    let status = Command::new(sandbox)
        .arg("doctor")
        .status()
        .with_context(|| format!("{} doctor를 실행하지 못했습니다", sandbox.display()))?;
    if !status.success() {
        bail!("sandbox native doctor가 실패했습니다");
    }
    let output = Command::new(sandbox)
        .arg("capabilities")
        .output()
        .with_context(|| format!("{} capabilities를 실행하지 못했습니다", sandbox.display()))?;
    if !output.status.success() || output.stdout.len() > 65_536 || output.stdout.is_empty() {
        bail!("sandbox capability 검증이 실패했습니다");
    }
    println!("PASS sandbox native-capabilities=verified");
    Ok(())
}

fn resolve_bundle_dir(explicit: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("bundle 경로를 찾지 못했습니다: {}", path.display()))?;
        validate_bundle(&canonical)?;
        return Ok(canonical);
    }
    let executable = env::current_exe().context("현재 g7mbctl 경로를 확인하지 못했습니다")?;
    let parent = executable
        .parent()
        .context("g7mbctl 상위 경로가 없습니다")?;
    let mut candidates = vec![parent.to_path_buf()];
    if parent.file_name().is_some_and(|name| name == "bin")
        && let Some(bundle) = parent.parent()
    {
        candidates.insert(0, bundle.to_path_buf());
    }
    for candidate in candidates {
        if validate_bundle(&candidate).is_ok() {
            return Ok(candidate);
        }
    }
    bail!("서버 번들을 자동 탐지하지 못했습니다. --bundle-dir을 지정하십시오")
}

fn validate_bundle(bundle: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(bundle)
        .with_context(|| format!("bundle 경로가 없습니다: {}", bundle.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("bundle 경로는 비심볼릭링크 디렉터리여야 합니다");
    }
    for payload in PAYLOAD_FILES {
        validate_regular_file(&bundle.join(payload.source), payload.maximum_bytes)?;
    }
    verify_payload_manifest(bundle)?;
    Ok(())
}

fn verify_payload_manifest(bundle: &Path) -> anyhow::Result<()> {
    let manifest_path = bundle.join("MANIFEST.sha256");
    let manifest =
        fs::read_to_string(&manifest_path).context("payload manifest를 읽지 못했습니다")?;
    let mut declared = BTreeMap::new();
    for line in manifest.lines() {
        let (digest, source) = line
            .split_once("  ")
            .context("payload manifest 행 형식이 올바르지 않습니다")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || source.is_empty()
            || Path::new(source).is_absolute()
            || Path::new(source)
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            bail!("payload manifest 값이 올바르지 않습니다");
        }
        if declared
            .insert(source.to_owned(), digest.to_owned())
            .is_some()
        {
            bail!("payload manifest에 중복 경로가 있습니다: {source}");
        }
    }

    let expected = PAYLOAD_FILES
        .iter()
        .filter(|payload| payload.source != "MANIFEST.sha256")
        .map(|payload| payload.source)
        .collect::<Vec<_>>();
    if declared.len() != expected.len() {
        bail!("payload manifest 파일 수가 설치 계약과 다릅니다");
    }
    for source in expected {
        let expected_digest = declared
            .get(source)
            .with_context(|| format!("payload manifest 항목이 없습니다: {source}"))?;
        let actual_digest = sha256_file(&bundle.join(source))?;
        if &actual_digest != expected_digest {
            bail!("payload SHA-256 검증이 실패했습니다: {source}");
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = open_regular(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes = file.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn validate_regular_file(path: &Path, maximum_bytes: u64) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("필수 파일이 없습니다: {}", path.display()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > maximum_bytes
    {
        bail!(
            "필수 파일이 일반 파일이 아니거나 크기 제한을 위반합니다: {}",
            path.display()
        );
    }
    Ok(())
}

fn install_payload(bundle: &Path, root: &Path, force: bool) -> anyhow::Result<()> {
    verify_payload_manifest(bundle)?;
    for payload in PAYLOAD_FILES {
        let destination = rooted(root, Path::new(payload.destination))?;
        copy_file_atomic(
            &bundle.join(payload.source),
            &destination,
            payload.mode,
            force,
        )?;
    }
    verify_payload_manifest(bundle)?;
    for payload in PAYLOAD_FILES {
        if payload.source == "MANIFEST.sha256" {
            continue;
        }
        let destination = rooted(root, Path::new(payload.destination))?;
        if sha256_file(&bundle.join(payload.source))? != sha256_file(&destination)? {
            bail!("설치 후 payload 검증이 실패했습니다: {}", payload.source);
        }
    }
    Ok(())
}

fn preflight_install_payload(bundle: &Path, root: &Path, force: bool) -> anyhow::Result<()> {
    for payload in PAYLOAD_FILES {
        let source = bundle.join(payload.source);
        let destination = rooted(root, Path::new(payload.destination))?;
        let mut source_file = open_regular(&source)?;
        let Ok(metadata) = fs::symlink_metadata(&destination) else {
            continue;
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            bail!(
                "설치 대상은 일반 비심볼릭링크 파일이어야 합니다: {}",
                destination.display()
            );
        }
        if !files_equal(&mut source_file, &destination)? && !force {
            bail!(
                "설치 대상이 이미 다릅니다. 교체하려면 --force가 필요합니다: {}",
                destination.display()
            );
        }
    }
    Ok(())
}

fn rooted(root: &Path, destination: &Path) -> anyhow::Result<PathBuf> {
    if !destination.is_absolute()
        || destination.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        bail!(
            "설치 대상 경로가 안전하지 않습니다: {}",
            destination.display()
        );
    }
    let relative = destination
        .strip_prefix(Path::new("/"))
        .context("설치 대상에서 root prefix를 제거하지 못했습니다")?;
    Ok(root.join(relative))
}

fn copy_file_atomic(
    source: &Path,
    destination: &Path,
    mode: u32,
    force: bool,
) -> anyhow::Result<()> {
    let mut input = open_regular(source)?;
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            bail!(
                "설치 대상은 일반 비심볼릭링크 파일이어야 합니다: {}",
                destination.display()
            );
        }
        if files_equal(&mut input, destination)? {
            set_mode(destination, mode)?;
            return Ok(());
        }
        if !force {
            bail!(
                "설치 대상이 이미 다릅니다. 교체하려면 --force가 필요합니다: {}",
                destination.display()
            );
        }
        input = open_regular(source)?;
    }
    let parent = destination
        .parent()
        .context("설치 대상에 상위 경로가 없습니다")?;
    ensure_directory(parent, 0o755)?;
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .context("설치 대상 파일명이 UTF-8이 아닙니다")?;
    let temporary = parent.join(format!(".{name}.install-{}", std::process::id()));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .with_context(|| format!("설치 임시 파일 생성 실패: {}", temporary.display()))?;
    let result = (|| -> anyhow::Result<()> {
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        set_mode(&temporary, mode)?;
        fs::rename(&temporary, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ignored = fs::remove_file(&temporary);
    }
    result
}

fn open_regular(path: &Path) -> anyhow::Result<File> {
    let path_metadata = fs::symlink_metadata(path)?;
    if !path_metadata.file_type().is_file() || path_metadata.file_type().is_symlink() {
        bail!(
            "payload는 일반 비심볼릭링크 파일이어야 합니다: {}",
            path.display()
        );
    }
    let file = File::open(path)?;
    let file_metadata = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        if path_metadata.dev() != file_metadata.dev() || path_metadata.ino() != file_metadata.ino()
        {
            bail!("payload 파일이 검사 중 교체되었습니다: {}", path.display());
        }
    }
    if !file_metadata.is_file() {
        bail!("payload는 일반 파일이어야 합니다: {}", path.display());
    }
    Ok(file)
}

fn files_equal(source: &mut File, destination: &Path) -> anyhow::Result<bool> {
    let mut target = File::open(destination)?;
    if source.metadata()?.len() != target.metadata()?.len() {
        return Ok(false);
    }
    let mut left = [0_u8; 16 * 1024];
    let mut right = [0_u8; 16 * 1024];
    loop {
        let left_read = source.read(&mut left)?;
        let right_read = target.read(&mut right)?;
        if left_read != right_read || left[..left_read] != right[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

fn prepare_service_account() -> anyhow::Result<()> {
    if !command_succeeds("getent", &["group", SERVICE_USER])? {
        run_checked("groupadd", &["--system", SERVICE_USER])?;
    }
    if !command_succeeds("id", &["-u", SERVICE_USER])? {
        let shell = ["/usr/sbin/nologin", "/sbin/nologin", "/bin/false"]
            .into_iter()
            .find(|candidate| Path::new(candidate).is_file())
            .context("nologin shell을 찾지 못했습니다")?;
        run_checked(
            "useradd",
            &[
                "--system",
                "--gid",
                SERVICE_USER,
                "--home-dir",
                "/var/lib/g7mediabooster",
                "--shell",
                shell,
                SERVICE_USER,
            ],
        )?;
    }
    let group = command_stdout("id", &["-gn", SERVICE_USER])?;
    if group.trim() != SERVICE_USER {
        bail!("기존 g7mediabooster 사용자의 primary group이 올바르지 않습니다");
    }
    Ok(())
}

fn prepare_directories() -> anyhow::Result<()> {
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

fn prepare_install_directories() -> anyhow::Result<()> {
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
    ] {
        create_directory(Path::new(path), 0o755)?;
    }
    Ok(())
}

fn require_command(program: &str, version_argument: &str) -> anyhow::Result<()> {
    let status = Command::new(program)
        .arg(version_argument)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("필수 명령을 실행하지 못했습니다: {program}"))?;
    if !status.success() {
        bail!("필수 명령의 사전검사가 실패했습니다: {program}");
    }
    Ok(())
}

fn ensure_native_runtime(skip_install: bool) -> anyhow::Result<()> {
    if native_runtime_available()? {
        return Ok(());
    }
    if skip_install {
        bail!("libvips·FFmpeg runtime이 없고 --skip-dependency-install이 지정됐습니다");
    }
    let os_release = fs::read_to_string("/etc/os-release")
        .context("native runtime 자동 설치를 위한 /etc/os-release를 읽지 못했습니다")?;
    if !is_supported_ubuntu(&os_release) {
        bail!(
            "native runtime 자동 설치는 Ubuntu 24.04에서만 지원합니다. libvips와 FFmpeg를 먼저 설치하십시오"
        );
    }
    require_command("apt-get", "--version")?;
    run_checked("apt-get", &["update"])?;
    let status = Command::new("apt-get")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .args([
            "install",
            "-y",
            "--no-install-recommends",
            "libvips-tools",
            "ffmpeg",
            "libheif-plugin-aomdec",
            "libheif-plugin-aomenc",
            "libheif-plugin-libde265",
            "libheif-plugin-x265",
        ])
        .status()
        .context("Ubuntu native runtime package 설치를 실행하지 못했습니다")?;
    if !status.success() || !native_runtime_available()? {
        bail!("Ubuntu native runtime package 설치 후 검증이 실패했습니다");
    }
    println!("PASS dependencies libvips=installed ffmpeg=installed");
    Ok(())
}

fn native_runtime_available() -> anyhow::Result<bool> {
    Ok(optional_command_succeeds("vips", &["--version"])?
        && optional_command_succeeds("ffmpeg", &["-version"])?
        && optional_command_succeeds("ffprobe", &["-version"])?)
}

fn is_supported_ubuntu(os_release: &str) -> bool {
    let mut id = None;
    let mut version = None;
    for line in os_release.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            id = Some(value.trim_matches('"'));
        } else if let Some(value) = line.strip_prefix("VERSION_ID=") {
            version = Some(value.trim_matches('"'));
        }
    }
    id == Some("ubuntu") && version == Some("24.04")
}

fn ensure_root() -> anyhow::Result<()> {
    let uid = command_stdout("id", &["-u"])?;
    if uid.trim() != "0" {
        bail!("통합 설치는 root 권한이 필요합니다: sudo ./bin/g7mbctl install");
    }
    Ok(())
}

fn command_succeeds(program: &str, args: &[&str]) -> anyhow::Result<bool> {
    let status = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("명령을 실행하지 못했습니다: {program}"))?;
    Ok(status.success())
}

fn optional_command_succeeds(program: &str, args: &[&str]) -> anyhow::Result<bool> {
    match Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => Ok(status.success()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("명령을 실행하지 못했습니다: {program}")),
    }
}

fn command_stdout(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("명령을 실행하지 못했습니다: {program}"))?;
    if !output.status.success() {
        bail!("명령이 실패했습니다: {program}");
    }
    String::from_utf8(output.stdout).context("명령 출력이 UTF-8이 아닙니다")
}

fn run_checked(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("명령을 실행하지 못했습니다: {program}"))?;
    if !status.success() {
        bail!("명령이 실패했습니다: {program}");
    }
    Ok(())
}

fn wait_until_ready(address: SocketAddr, timeout: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    while Instant::now() < deadline {
        match check_http_ready(address) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        thread::sleep(Duration::from_millis(500));
    }
    if let Some(error) = last_error {
        bail!("API ready 확인 시간 초과: {error}");
    }
    bail!("API ready 확인 시간 초과")
}

fn check_http_ready(address: SocketAddr) -> anyhow::Result<()> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .with_context(|| format!("API에 연결하지 못했습니다: {address}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let host = if address.is_ipv6() {
        format!("[{}]:{}", address.ip(), address.port())
    } else {
        address.to_string()
    };
    write!(
        stream,
        "GET /health/ready HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = [0_u8; 4096];
    let bytes = stream.read(&mut response)?;
    let response =
        std::str::from_utf8(&response[..bytes]).context("API 응답이 UTF-8이 아닙니다")?;
    if !response.starts_with("HTTP/1.1 200 ") && !response.starts_with("HTTP/1.0 200 ") {
        bail!("API가 ready 200을 반환하지 않았습니다");
    }
    Ok(())
}

fn create_directory(path: &Path, mode: u32) -> anyhow::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_dir() || metadata.file_type().is_symlink())
    {
        bail!(
            "설치 디렉터리는 비심볼릭링크 디렉터리여야 합니다: {}",
            path.display()
        );
    }
    fs::create_dir_all(path)?;
    set_mode(path, mode)
}

fn ensure_directory(path: &Path, mode: u32) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => bail!(
            "설치 경로는 비심볼릭링크 디렉터리여야 합니다: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            set_mode(path, mode)
        }
        Err(error) => Err(error)
            .with_context(|| format!("설치 디렉터리를 확인하지 못했습니다: {}", path.display())),
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::{
        PAYLOAD_FILES, install_payload, is_supported_ubuntu, sha256_file, validate_bundle,
    };

    fn fake_bundle(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
        for payload in PAYLOAD_FILES {
            if payload.source == "MANIFEST.sha256" {
                continue;
            }
            let path = root.join(payload.source);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, format!("payload:{}", payload.source))?;
        }
        let mut manifest = String::new();
        for payload in PAYLOAD_FILES {
            if payload.source == "MANIFEST.sha256" {
                continue;
            }
            manifest.push_str(&format!(
                "{}  {}\n",
                sha256_file(&root.join(payload.source))?,
                payload.source
            ));
        }
        fs::write(root.join("MANIFEST.sha256"), manifest)?;
        Ok(())
    }

    #[test]
    fn payload_install_is_complete_idempotent_and_force_guarded()
    -> Result<(), Box<dyn std::error::Error>> {
        let bundle = tempfile::tempdir()?;
        let root = tempfile::tempdir()?;
        fake_bundle(bundle.path())?;
        validate_bundle(bundle.path())?;
        install_payload(bundle.path(), root.path(), false)?;
        install_payload(bundle.path(), root.path(), false)?;

        let changed_source = bundle.path().join("bin/g7mb-api");
        fs::write(&changed_source, "changed")?;
        assert!(validate_bundle(bundle.path()).is_err());
        fake_bundle(bundle.path())?;
        fs::write(&changed_source, "changed")?;
        let manifest = fs::read_to_string(bundle.path().join("MANIFEST.sha256"))?;
        let previous = sha256_file(&bundle.path().join("bin/g7mb-api"))?;
        let manifest = manifest
            .lines()
            .map(|line| {
                if line.ends_with("  bin/g7mb-api") {
                    format!("{previous}  bin/g7mb-api")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            bundle.path().join("MANIFEST.sha256"),
            format!("{manifest}\n"),
        )?;
        validate_bundle(bundle.path())?;
        assert!(install_payload(bundle.path(), root.path(), false).is_err());
        install_payload(bundle.path(), root.path(), true)?;
        let installed = root.path().join("usr/local/bin/g7mb-api");
        assert_eq!(fs::read_to_string(installed)?, "changed");
        Ok(())
    }

    #[test]
    fn bundle_rejects_missing_and_symbolic_link_payloads() -> Result<(), Box<dyn std::error::Error>>
    {
        let bundle = tempfile::tempdir()?;
        assert!(validate_bundle(bundle.path()).is_err());
        fake_bundle(bundle.path())?;
        fs::remove_file(bundle.path().join("bin/g7mb-api"))?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                bundle.path().join("bin/g7mbctl"),
                bundle.path().join("bin/g7mb-api"),
            )?;
            assert!(validate_bundle(bundle.path()).is_err());
        }
        Ok(())
    }

    #[test]
    fn dependency_bootstrap_is_limited_to_exact_ubuntu_release() {
        assert!(is_supported_ubuntu("ID=ubuntu\nVERSION_ID=\"24.04\"\n"));
        assert!(!is_supported_ubuntu("ID=ubuntu\nVERSION_ID=\"22.04\"\n"));
        assert!(!is_supported_ubuntu("ID=debian\nVERSION_ID=\"12\"\n"));
    }
}
