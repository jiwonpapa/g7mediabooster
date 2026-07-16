//! Repository-local quality and generation harness.

use std::{env, ffi::OsStr, fs, path::PathBuf, process::Command};

use anyhow::{Context as _, bail};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(version, about = "G7MediaBooster development harness")]
struct Cli {
    #[command(subcommand)]
    command: Harness,
}

#[derive(Debug, Subcommand)]
enum Harness {
    /// Fast mandatory local quality gate.
    Quick,
    /// Enforce the workspace rustdoc policy and build all first-party documentation.
    Rustdoc,
    /// Full pull-request gate excluding external credentials.
    Ci,
    /// RustSec, licenses, bans, and dependency source checks.
    SupplyChain,
    /// Use nextest while retaining separate doctests.
    Nextest,
    /// Produce coverage and enforce the initial line threshold.
    Coverage,
    /// Compile or execute Criterion benchmarks.
    Bench {
        /// Compile without executing measurements.
        #[arg(long)]
        no_run: bool,
    },
    /// Run the object-key fuzz target for a bounded duration.
    Fuzz {
        /// Maximum fuzzing time in seconds.
        #[arg(long, default_value_t = 60)]
        seconds: u64,
    },
    /// Run pure Rust domain tests under Miri nightly.
    Miri,
    /// Execute actual libvips and FFmpeg fixtures.
    NativeSmoke,
    /// Start the API binary and verify live, ready, capability, and header responses.
    ApiSmoke,
    /// Generate an offline production config twice and prove secret separation/idempotence.
    SetupSmoke,
    /// Install and verify the Gnuboard 7 PHP/TypeScript adapter harness.
    G7Adapter,
    /// Build the reproducible Gnuboard 7 module archive and SHA-256 file.
    G7ModulePackage,
    /// Install and verify the Gnuboard 5 PHP/TypeScript adapter harness.
    G5Adapter,
    /// Verify the Gnuboard 5 session/link/deletion store against MySQL and MyISAM fixtures.
    G5HostSmoke,
    /// Run 100 real JPEG jobs with process-tree RSS and expired-lease recovery gates.
    Load100,
    /// Run an actual 25,000px panorama through the heavy-image RSS gate.
    HeavyImage,
    /// Process a 64 MP AVIF and reject 200 MP before full-frame decode.
    HeavyAvif,
    /// Run worker load and API health under Linux cgroup CPU, memory, and PID limits.
    CgroupSmoke,
    /// Run live S3-compatible single/multipart/abort conformance against pinned MinIO.
    StorageConformance,
    /// Run API, MinIO, worker, and native sandbox through real single and multipart media jobs.
    FullStackSmoke,
    /// Publish and roll back a real watermark policy through the G7 PHP HMAC client.
    G7PolicySmoke,
    /// Upload an exact local 5GiB object through API-controlled direct multipart.
    LargeMultipartSmoke,
    /// Run credential-gated conformance against an existing S3-compatible provider bucket.
    LiveStorageConformance,
    /// Prove online snapshot, SHA-256, retention, read-only verify, and isolated restore.
    DatabaseRecovery,
    /// Print native package versions and FFmpeg build configuration for SBOM evidence.
    NativeInventory,
    /// Generate CycloneDX JSON files for workspace binaries.
    Sbom,
    /// Check or write the generated OpenAPI contract.
    Openapi {
        /// Contract operation.
        #[arg(value_enum)]
        action: OpenApiAction,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OpenApiAction {
    Check,
    Write,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Harness::Quick => quick(),
        Harness::Rustdoc => rustdoc(),
        Harness::Ci => ci(),
        Harness::SupplyChain => supply_chain(),
        Harness::Nextest => nextest(),
        Harness::Coverage => coverage(),
        Harness::Bench { no_run } => bench(no_run),
        Harness::Fuzz { seconds } => fuzz(seconds),
        Harness::Miri => miri(),
        Harness::NativeSmoke => run("bash", ["scripts/native-smoke.sh"]),
        Harness::ApiSmoke => run("bash", ["scripts/api-smoke.sh"]),
        Harness::SetupSmoke => run("bash", ["scripts/setup-cli-smoke.sh"]),
        Harness::G7Adapter => g7_adapter(),
        Harness::G7ModulePackage => run("bash", ["scripts/package-g7-module.sh"]),
        Harness::G5Adapter => g5_adapter(),
        Harness::G5HostSmoke => run("bash", ["scripts/gnuboard5-session-store-smoke.sh"]),
        Harness::Load100 => run("bash", ["scripts/load-100.sh"]),
        Harness::HeavyImage => run("bash", ["scripts/heavy-image.sh"]),
        Harness::HeavyAvif => run("bash", ["scripts/heavy-avif.sh"]),
        Harness::CgroupSmoke => run("bash", ["scripts/cgroup-smoke.sh"]),
        Harness::StorageConformance => run("bash", ["scripts/storage-conformance.sh"]),
        Harness::FullStackSmoke => run("bash", ["scripts/full-stack-smoke.sh"]),
        Harness::G7PolicySmoke => run("bash", ["scripts/g7-policy-smoke.sh"]),
        Harness::LargeMultipartSmoke => run("bash", ["scripts/large-multipart-smoke.sh"]),
        Harness::LiveStorageConformance => run("bash", ["scripts/live-storage-conformance.sh"]),
        Harness::DatabaseRecovery => cargo([
            "test",
            "--package",
            "g7mb-worker",
            "--bin",
            "g7mb-worker",
            "tests::backup_rotation_hash_verification_and_restore_rehearsal_are_end_to_end",
            "--",
            "--exact",
        ]),
        Harness::NativeInventory => run("bash", ["scripts/native-inventory.sh"]),
        Harness::Sbom => sbom(),
        Harness::Openapi { action } => openapi(action),
    }
}

fn quick() -> anyhow::Result<()> {
    cargo(["fmt", "--all", "--", "--check"])?;
    cargo(["check", "--workspace", "--all-targets", "--locked"])?;
    cargo([
        "check",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
    ])?;
    cargo([
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo([
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(["test", "--workspace", "--all-features", "--locked"])?;
    rustdoc()
}

fn rustdoc() -> anyhow::Result<()> {
    verify_rustdoc_policy()?;
    run_with_env(
        "cargo",
        [
            "doc",
            "--workspace",
            "--all-features",
            "--no-deps",
            "--locked",
        ],
        [("RUSTDOCFLAGS", "-D warnings")],
    )
}

fn verify_rustdoc_policy() -> anyhow::Result<()> {
    let root = workspace_root();
    let root_manifest_path = root.join("Cargo.toml");
    let root_manifest = fs::read_to_string(&root_manifest_path)
        .with_context(|| format!("failed to read {}", root_manifest_path.display()))?;

    for required in [
        "missing_docs = \"deny\"",
        "[workspace.lints.rustdoc]",
        "all = \"deny\"",
    ] {
        if !root_manifest.contains(required) {
            bail!(
                "rustdoc policy is incomplete in {}: missing `{required}`",
                root_manifest_path.display()
            );
        }
    }

    let mut manifests = vec![root.join("xtask/Cargo.toml")];
    collect_manifests(&root.join("apps"), &mut manifests)?;
    collect_manifests(&root.join("crates"), &mut manifests)?;
    manifests.sort();

    for manifest_path in manifests {
        let manifest = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        if !inherits_workspace_lints(&manifest) {
            bail!(
                "{} must contain `[lints]` with `workspace = true`",
                manifest_path.display()
            );
        }
    }

    Ok(())
}

fn collect_manifests(
    directory: &std::path::Path,
    manifests: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read directory {}", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_manifests(&entry.path(), manifests)?;
        } else if file_type.is_file() && entry.file_name() == OsStr::new("Cargo.toml") {
            manifests.push(entry.path());
        }
    }
    Ok(())
}

fn inherits_workspace_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_lints = line == "[lints]";
        } else if in_lints && line == "workspace = true" {
            return true;
        }
    }
    false
}

fn ci() -> anyhow::Result<()> {
    quick()?;
    openapi(OpenApiAction::Check)?;
    run("bash", ["scripts/setup-cli-smoke.sh"])?;
    run("bash", ["scripts/live-storage-preflight-smoke.sh"])?;
    run("bash", ["scripts/package-g7-module.sh"])?;
    bench(true)
}

fn supply_chain() -> anyhow::Result<()> {
    cargo(["audit", "--deny", "warnings"])?;
    cargo(["deny", "check"])
}

fn nextest() -> anyhow::Result<()> {
    cargo([
        "nextest",
        "run",
        "--workspace",
        "--all-features",
        "--locked",
        "--profile",
        "ci",
    ])?;
    cargo(["test", "--workspace", "--all-features", "--doc", "--locked"])
}

fn coverage() -> anyhow::Result<()> {
    fs::create_dir_all(workspace_root().join("reports"))?;
    // cargo-llvm-cov keeps instrumented objects between invocations. An explicit
    // workspace clean prevents stale binaries from unrelated packages from
    // diluting or inflating the enforced package report.
    cargo(["llvm-cov", "clean", "--workspace"])?;
    cargo([
        "llvm-cov",
        "--package",
        "g7mb-api",
        "--package",
        "g7mb-application",
        "--package",
        "g7mb-auth",
        "--package",
        "g7mb-config",
        "--package",
        "g7mb-domain",
        "--package",
        "g7mb-media",
        "--package",
        "g7mb-object-store-s3",
        "--package",
        "g7mb-persistence-sqlite",
        "--package",
        "g7mb-worker",
        "--lib",
        "--locked",
        "--lcov",
        "--output-path",
        "reports/lcov.info",
        "--fail-under-lines",
        "80",
    ])
}

fn bench(no_run: bool) -> anyhow::Result<()> {
    // Add packages here as real benchmark targets are introduced. Compiling every
    // service binary in the bench profile provides no benchmark coverage.
    let mut args = vec!["bench", "--package", "g7mb-domain", "--locked"];
    if no_run {
        args.push("--no-run");
    }
    cargo(args)
}

fn fuzz(seconds: u64) -> anyhow::Result<()> {
    let max_total_time = format!("-max_total_time={seconds}");
    cargo([
        "+nightly",
        "fuzz",
        "run",
        "object_key",
        "--manifest-path",
        "fuzz/Cargo.toml",
        "--",
        &max_total_time,
    ])
}

fn miri() -> anyhow::Result<()> {
    cargo(["+nightly", "miri", "test", "--package", "g7mb-domain"])
}

fn sbom() -> anyhow::Result<()> {
    cargo([
        "cyclonedx",
        "--format",
        "json",
        "--describe",
        "binaries",
        "--all-features",
        "--spec-version",
        "1.5",
        "--override-filename",
        "g7mb.cdx.json",
    ])
}

fn g7_adapter() -> anyhow::Result<()> {
    let module = workspace_root().join("adapters/gnuboard7/jiwonpapa-g7mediabooster");
    run_in(
        &module,
        "composer",
        ["validate", "--strict", "--no-check-publish"],
    )?;
    run_in(
        &module,
        "composer",
        ["install", "--no-interaction", "--prefer-dist"],
    )?;
    run_in(&module, "vendor/bin/phpunit", ["-c", "phpunit.xml"])?;
    run_in(&module, "npm", ["ci", "--ignore-scripts"])?;
    run_in(&module, "npm", ["run", "typecheck"])?;
    run_in(&module, "npm", ["test"])?;
    run_in(&module, "npm", ["run", "build"])
}

fn g5_adapter() -> anyhow::Result<()> {
    let module = workspace_root().join("adapters/gnuboard5/jiwonpapa-g7mediabooster");
    run_in(
        &module,
        "composer",
        ["validate", "--strict", "--no-check-publish"],
    )?;
    run_in(
        &module,
        "composer",
        ["install", "--no-interaction", "--prefer-dist"],
    )?;
    run_in(&module, "vendor/bin/phpunit", ["-c", "phpunit.xml"])?;
    run_in(&module, "npm", ["ci", "--ignore-scripts"])?;
    run_in(&module, "npm", ["run", "typecheck"])?;
    run_in(&module, "npm", ["test"])?;
    run_in(&module, "npm", ["run", "build"])
}

fn openapi(action: OpenApiAction) -> anyhow::Result<()> {
    let path = workspace_root().join("openapi/g7mediabooster-v1.json");
    let generated = format!("{}\n", g7mb_api::openapi_json()?);
    match action {
        OpenApiAction::Write => {
            let parent = path
                .parent()
                .context("OpenAPI output path has no parent directory")?;
            fs::create_dir_all(parent)?;
            fs::write(&path, generated)?;
            Ok(())
        }
        OpenApiAction::Check => {
            let committed = fs::read_to_string(&path)
                .with_context(|| format!("missing OpenAPI snapshot: {}", path.display()))?;
            if committed != generated {
                bail!("OpenAPI drift detected; run `cargo xtask openapi write`");
            }
            Ok(())
        }
    }
}

fn cargo<I, S>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run(cargo, args)
}

fn run<P, I, S>(program: P, args: I) -> anyhow::Result<()>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(program.as_ref())
        .args(args)
        .status()
        .with_context(|| format!("failed to start {}", program.as_ref().to_string_lossy()))?;
    if !status.success() {
        bail!(
            "{} exited unsuccessfully: {status}",
            program.as_ref().to_string_lossy()
        );
    }
    Ok(())
}

fn run_with_env<P, I, S, E, K, V>(program: P, args: I, envs: E) -> anyhow::Result<()>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let status = Command::new(program.as_ref())
        .args(args)
        .envs(envs)
        .status()
        .with_context(|| format!("failed to start {}", program.as_ref().to_string_lossy()))?;
    if !status.success() {
        bail!(
            "{} exited unsuccessfully: {status}",
            program.as_ref().to_string_lossy()
        );
    }
    Ok(())
}

fn run_in<P, I, S>(directory: &std::path::Path, program: P, args: I) -> anyhow::Result<()>
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(program.as_ref())
        .args(args)
        .current_dir(directory)
        .status()
        .with_context(|| {
            format!(
                "failed to start {} in {}",
                program.as_ref().to_string_lossy(),
                directory.display()
            )
        })?;
    if !status.success() {
        bail!(
            "{} exited unsuccessfully in {}: {status}",
            program.as_ref().to_string_lossy(),
            directory.display()
        );
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}
