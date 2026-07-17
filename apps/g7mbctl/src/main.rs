//! Interactive and automation-safe installer for G7MediaBooster.

mod installer;

use std::{
    fs::{self, OpenOptions},
    io::{self, IsTerminal as _, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use anyhow::{Context as _, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use g7mb_config::{Settings, StorageProvider, StorageSettings};
use g7mb_object_store_s3::S3StorageAdmin;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

const DEFAULT_CONFIG: &str = "/etc/g7mediabooster/g7mb.toml";
const DEFAULT_SECRETS: &str = "/etc/g7mediabooster/credentials";

#[derive(Debug, Parser)]
#[command(version, about = "G7MediaBooster installation and storage control CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Installs the complete Linux server bundle, runs setup, and starts one product target.
    Install(InstallArgs),
    /// Creates a production configuration using an interactive CUI or explicit files.
    Setup(Box<SetupArgs>),
    /// Shows the product-level service state without exposing individual unit management.
    Status(ConfigArg),
    /// Checks systemd, API, sandbox, configuration, and optional live storage access.
    Doctor(DoctorArgs),
    /// Runs storage bootstrap and live runtime checks after setup.
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },
}

#[derive(Debug, Args)]
struct InstallArgs {
    /// Extracted server bundle root; normally detected from `bin/g7mbctl`.
    #[arg(long)]
    bundle_dir: Option<PathBuf>,
    /// Replaces installed binaries and unit files that differ from this bundle.
    #[arg(long)]
    force: bool,
    /// Registers the product but does not run the interactive storage setup.
    #[arg(long)]
    skip_setup: bool,
    /// Does not enable or start the product target after configuration.
    #[arg(long)]
    skip_start: bool,
    /// Refuses missing libvips/FFmpeg instead of installing Ubuntu runtime packages.
    #[arg(long)]
    skip_dependency_install: bool,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    /// Installed TOML configuration file.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Checks local services and native media only, without provider canary objects.
    #[arg(long)]
    skip_storage: bool,
}

#[derive(Debug, Subcommand)]
enum StorageCommand {
    /// Checks or creates buckets and merges the managed browser CORS rule.
    Bootstrap(StorageBootstrapArgs),
    /// Performs bounded PUT/HEAD/GET/LIST/DELETE and multipart canaries.
    Doctor(ConfigArg),
}

#[derive(Debug, Args)]
struct ConfigArg {
    /// Installed TOML configuration file.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
}

#[derive(Debug, Args)]
struct StorageBootstrapArgs {
    /// Installed TOML configuration file.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Creates a missing bucket when the credential has management permission.
    #[arg(long)]
    create_missing: bool,
    /// Exact HTTPS G5/G7 browser origin; repeat for multiple sites.
    #[arg(long = "origin")]
    origins: Vec<String>,
    /// Checks buckets without modifying CORS.
    #[arg(long)]
    skip_cors: bool,
}

#[derive(Debug, Args)]
struct SetupArgs {
    /// Disables prompts; credentials must be supplied through input files.
    #[arg(long)]
    non_interactive: bool,
    /// Storage provider profile.
    #[arg(long, value_enum)]
    provider: Option<Provider>,
    /// Cloudflare account ID used to derive the R2 endpoint.
    #[arg(long)]
    account_id: Option<String>,
    /// Custom S3-compatible endpoint; required for the generic profile.
    #[arg(long)]
    endpoint_url: Option<String>,
    /// AWS region, `auto` for R2, or the provider signing region.
    #[arg(long)]
    region: Option<String>,
    /// Private raw bucket, or the shared private bucket.
    #[arg(long)]
    bucket: Option<String>,
    /// Optional separate derivative bucket; defaults to `--bucket`.
    #[arg(long)]
    derivative_bucket: Option<String>,
    /// Exact HTTPS G5/G7 browser origin; repeat for multiple sites.
    #[arg(long = "origin")]
    origins: Vec<String>,
    /// File from which a non-interactive setup imports the access key ID.
    #[arg(long)]
    access_key_id_file: Option<PathBuf>,
    /// File from which a non-interactive setup imports the secret access key.
    #[arg(long)]
    secret_access_key_file: Option<PathBuf>,
    /// Tenant identifier used in HMAC and object-key isolation.
    #[arg(long)]
    tenant_id: Option<String>,
    /// Generated production TOML path.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Root-only source credential directory for systemd LoadCredential.
    #[arg(long, default_value = DEFAULT_SECRETS)]
    secrets_dir: PathBuf,
    /// Creates missing buckets before the canary.
    #[arg(long)]
    create_buckets: bool,
    /// Does not modify provider CORS.
    #[arg(long)]
    skip_cors: bool,
    /// Writes configuration now and defers all provider network operations.
    #[arg(long)]
    defer_storage: bool,
    /// Uses path-style addressing for a generic S3-compatible endpoint.
    #[arg(long)]
    force_path_style: bool,
    /// Replaces an existing generated configuration atomically.
    #[arg(long)]
    force: bool,
    /// Does not apply the production `g7mediabooster` group to the config file.
    #[arg(long)]
    skip_ownership: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Provider {
    R2,
    AwsS3,
    Lightsail,
    Generic,
}

impl Provider {
    const fn storage_provider(self) -> StorageProvider {
        match self {
            Self::R2 => StorageProvider::R2,
            Self::AwsS3 => StorageProvider::AwsS3,
            Self::Lightsail => StorageProvider::Lightsail,
            Self::Generic => StorageProvider::Generic,
        }
    }

    const fn config_name(self) -> &'static str {
        match self {
            Self::R2 => "r2",
            Self::AwsS3 => "aws-s3",
            Self::Lightsail => "lightsail",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug)]
struct SetupValues {
    provider: Provider,
    endpoint_url: Option<String>,
    region: String,
    raw_bucket: String,
    derivative_bucket: String,
    origins: Vec<String>,
    access_key_id: SecretString,
    secret_access_key: SecretString,
    tenant_id: String,
    force_path_style: bool,
    create_buckets: bool,
    configure_cors: bool,
    defer_storage: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Install(args) => installer::install(installer::InstallOptions {
            bundle_dir: args.bundle_dir,
            force: args.force,
            skip_setup: args.skip_setup,
            skip_start: args.skip_start,
            skip_dependency_install: args.skip_dependency_install,
        }),
        Command::Setup(args) => setup(*args).await,
        Command::Status(args) => installer::status(&args.config).map(|_| ()),
        Command::Doctor(args) => doctor(args).await,
        Command::Storage { command } => storage(command).await,
    }
}

async fn doctor(args: DoctorArgs) -> anyhow::Result<()> {
    let settings = installer::status(&args.config)?;
    installer::sandbox_doctor(&settings.worker.sandbox_binary)?;
    if !args.skip_storage {
        let report = S3StorageAdmin::new(&settings.storage)?
            .canary(&settings.storage)
            .await
            .context("저장소 canary에 실패했습니다")?;
        println!(
            "PASS storage buckets={} single_object={} multipart={}",
            report.buckets_checked, report.single_object, report.multipart
        );
    }
    println!("PASS doctor");
    Ok(())
}

async fn storage(command: StorageCommand) -> anyhow::Result<()> {
    match command {
        StorageCommand::Bootstrap(args) => {
            let settings =
                Settings::load(Some(&args.config)).context("설정 파일을 읽지 못했습니다")?;
            let origins = if args.skip_cors {
                Vec::new()
            } else {
                validate_origins(args.origins)?
            };
            if !args.skip_cors && origins.is_empty() {
                bail!("CORS를 설정하려면 --origin이 하나 이상 필요합니다");
            }
            let reports = S3StorageAdmin::new(&settings.storage)?
                .bootstrap(&settings.storage, args.create_missing, &origins)
                .await
                .context("저장소 bootstrap에 실패했습니다")?;
            for report in reports {
                println!(
                    "PASS bucket={} created={} cors={}",
                    report.bucket, report.created, report.cors_configured
                );
            }
            Ok(())
        }
        StorageCommand::Doctor(args) => {
            let settings =
                Settings::load(Some(&args.config)).context("설정 파일을 읽지 못했습니다")?;
            let report = S3StorageAdmin::new(&settings.storage)?
                .canary(&settings.storage)
                .await
                .context("저장소 canary에 실패했습니다")?;
            println!(
                "PASS buckets={} single_object={} multipart={}",
                report.buckets_checked, report.single_object, report.multipart
            );
            Ok(())
        }
    }
}

async fn setup(args: SetupArgs) -> anyhow::Result<()> {
    if !args.non_interactive && !io::stdin().is_terminal() {
        bail!("대화형 setup에는 TTY가 필요합니다. 자동화에서는 --non-interactive를 사용하십시오");
    }
    if !args.config.is_absolute() || !args.secrets_dir.is_absolute() {
        bail!("--config와 --secrets-dir은 절대 경로여야 합니다");
    }
    let values = collect_setup_values(&args)?;
    let storage = StorageSettings {
        provider: values.provider.storage_provider(),
        endpoint_url: values.endpoint_url.clone(),
        region: values.region.clone(),
        raw_bucket: values.raw_bucket.clone(),
        derivative_bucket: values.derivative_bucket.clone(),
        access_key_id: values.access_key_id.clone(),
        access_key_id_file: None,
        secret_access_key: values.secret_access_key.clone(),
        secret_access_key_file: None,
        force_path_style: values.force_path_style,
    };
    let access_path = args.secrets_dir.join("storage-access-key-id");
    let secret_path = args.secrets_dir.join("storage-secret-access-key");
    let hmac_path = args.secrets_dir.join("g7-hmac-secret");
    let hmac_secret = if hmac_path.exists() && !args.force {
        read_secret_input(&hmac_path).context("기존 HMAC 비밀값을 읽지 못했습니다")?
    } else {
        random_secret()?
    };
    let rendered = render_config(&values, &access_path, &secret_path, &hmac_path)?;
    preflight_target(
        &access_path,
        values.access_key_id.expose_secret().as_bytes(),
        args.force,
    )?;
    preflight_target(
        &secret_path,
        values.secret_access_key.expose_secret().as_bytes(),
        args.force,
    )?;
    preflight_target(&hmac_path, hmac_secret.as_bytes(), args.force)?;
    preflight_target(&args.config, rendered.as_bytes(), args.force)?;

    if !values.defer_storage {
        let origins = if values.configure_cors {
            values.origins.clone()
        } else {
            Vec::new()
        };
        let admin = S3StorageAdmin::new(&storage)?;
        let reports = admin
            .bootstrap(&storage, values.create_buckets, &origins)
            .await
            .context("설정 저장 전 bucket/CORS bootstrap에 실패했습니다")?;
        for report in reports {
            println!(
                "PASS bucket={} created={} cors={}",
                report.bucket, report.created, report.cors_configured
            );
        }
        let report = admin
            .canary(&storage)
            .await
            .context("설정 저장 전 저장소 canary에 실패했습니다")?;
        println!(
            "PASS storage-canary buckets={} single_object={} multipart={}",
            report.buckets_checked, report.single_object, report.multipart
        );
    }

    create_directory(&args.secrets_dir, 0o700)?;
    atomic_write(
        &access_path,
        values.access_key_id.expose_secret().as_bytes(),
        0o600,
        args.force,
    )?;
    atomic_write(
        &secret_path,
        values.secret_access_key.expose_secret().as_bytes(),
        0o600,
        args.force,
    )?;
    atomic_write(&hmac_path, hmac_secret.as_bytes(), 0o600, args.force)?;

    let config_parent = args
        .config
        .parent()
        .context("설정 파일에 상위 디렉터리가 없습니다")?;
    create_directory(config_parent, 0o750)?;
    atomic_write(&args.config, rendered.as_bytes(), 0o640, args.force)?;
    if !args.skip_ownership && args.config.starts_with("/etc/g7mediabooster") {
        set_config_group(&args.config)?;
    }
    Settings::load(Some(&args.config)).context("생성한 설정의 자체 검증에 실패했습니다")?;

    println!("PASS config={}", args.config.display());
    println!("PASS credentials={}", args.secrets_dir.display());
    if values.defer_storage {
        println!("PENDING storage: g7mbctl storage bootstrap/doctor를 실행하십시오");
    }
    if !args.non_interactive {
        println!("G7 관리자에 한 번만 입력할 HMAC 비밀값: {hmac_secret}");
    } else {
        println!(
            "G7 HMAC 비밀값은 root-only {}에 생성했습니다",
            hmac_path.display()
        );
    }
    Ok(())
}

fn collect_setup_values(args: &SetupArgs) -> anyhow::Result<SetupValues> {
    let interactive = !args.non_interactive;
    let provider = match args.provider {
        Some(value) => value,
        None if interactive => prompt_provider()?,
        None => bail!("--non-interactive에는 --provider가 필요합니다"),
    };
    let lightsail = provider == Provider::Lightsail;
    if lightsail && args.create_buckets {
        bail!(
            "Lightsail bucket access key로는 버킷을 생성할 수 없습니다. Lightsail에서 먼저 생성하십시오"
        );
    }
    if lightsail && !args.skip_cors {
        if !interactive {
            bail!(
                "Lightsail CORS는 Lightsail API/콘솔에서 먼저 설정하고 --skip-cors를 명시해야 합니다"
            );
        }
        if !prompt_yes_no(
            "Lightsail API/콘솔에서 이 사이트의 PUT/GET/HEAD CORS와 ETag 노출을 설정했습니까?",
            false,
        )? {
            bail!("Lightsail 브라우저 직접 업로드에는 사전 CORS 설정이 필요합니다");
        }
    }

    let (endpoint_url, region) = match provider {
        Provider::R2 => {
            let account_id = required_value(
                args.account_id.clone(),
                interactive,
                "Cloudflare Account ID",
            )?;
            if account_id.len() != 32 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("Cloudflare Account ID는 32자리 16진수여야 합니다");
            }
            (
                Some(format!(
                    "https://{}.r2.cloudflarestorage.com",
                    account_id.to_ascii_lowercase()
                )),
                "auto".to_owned(),
            )
        }
        Provider::AwsS3 | Provider::Lightsail => {
            let region = optional_or_prompt(
                args.region.clone(),
                interactive,
                "AWS region",
                "ap-northeast-2",
            )?;
            (None, validate_region(region)?)
        }
        Provider::Generic => {
            let endpoint =
                required_value(args.endpoint_url.clone(), interactive, "S3 endpoint URL")?;
            let region = optional_or_prompt(
                args.region.clone(),
                interactive,
                "S3 signing region",
                "us-east-1",
            )?;
            (Some(validate_endpoint(endpoint)?), validate_region(region)?)
        }
    };

    let raw_bucket = validate_bucket(required_value(
        args.bucket.clone(),
        interactive,
        "Private bucket name",
    )?)?;
    let derivative_bucket = validate_bucket(match &args.derivative_bucket {
        Some(value) => value.clone(),
        None if interactive => prompt_default("Derivative bucket", &raw_bucket)?,
        None => raw_bucket.clone(),
    })?;
    let tenant_id = validate_tenant(optional_or_prompt(
        args.tenant_id.clone(),
        interactive,
        "Tenant ID",
        "g7-site",
    )?)?;
    let configure_cors = !args.skip_cors && !lightsail;
    let origins = if !configure_cors {
        Vec::new()
    } else if args.origins.is_empty() && interactive {
        validate_origins(vec![prompt_required("G5/G7 origin (https://example.com)")?])?
    } else {
        validate_origins(args.origins.clone())?
    };
    if configure_cors && origins.is_empty() {
        bail!("브라우저 직접 업로드 CORS를 위해 --origin이 필요합니다");
    }

    let access_key_id = read_or_prompt_secret(
        args.access_key_id_file.as_deref(),
        interactive,
        "S3 Access Key ID",
    )?;
    let secret_access_key = read_or_prompt_secret(
        args.secret_access_key_file.as_deref(),
        interactive,
        "S3 Secret Access Key",
    )?;
    if access_key_id.expose_secret().len() > 256 || secret_access_key.expose_secret().len() > 1024 {
        bail!("저장소 자격증명이 허용 길이를 초과합니다");
    }

    let create_buckets = if lightsail {
        false
    } else if interactive && !args.create_buckets {
        prompt_yes_no(
            "없는 버킷을 생성하시겠습니까?",
            !matches!(provider, Provider::Generic),
        )?
    } else {
        args.create_buckets
    };
    let defer_storage = if interactive && !args.defer_storage {
        !prompt_yes_no("지금 bucket/CORS와 실제 업로드를 검사하시겠습니까?", true)?
    } else {
        args.defer_storage
    };

    Ok(SetupValues {
        provider,
        endpoint_url,
        region,
        raw_bucket,
        derivative_bucket,
        origins,
        access_key_id,
        secret_access_key,
        tenant_id,
        force_path_style: args.force_path_style,
        create_buckets,
        configure_cors,
        defer_storage,
    })
}

fn render_config(
    values: &SetupValues,
    access_path: &Path,
    secret_path: &Path,
    hmac_path: &Path,
) -> anyhow::Result<String> {
    let endpoint = values
        .endpoint_url
        .as_ref()
        .map(|value| format!("endpoint_url = {}\n", toml_string(value)))
        .unwrap_or_default();
    let access_path = path_text(access_path)?;
    let secret_path = path_text(secret_path)?;
    let hmac_path = path_text(hmac_path)?;
    Ok(format!(
        r#"# Generated by g7mbctl setup for {:?}. Secrets are loaded from root-only files.
[server]
bind_addr = "127.0.0.1:8088"

[auth]
key_id = "g7-primary"
tenant_id = {}
hmac_secret_file = {}

[storage]
provider = {}
{}region = {}
raw_bucket = {}
derivative_bucket = {}
access_key_id_file = {}
secret_access_key_file = {}
force_path_style = {}

[database]
url = "sqlite:///var/lib/g7mediabooster/g7mb.db"
backup_directory = "/var/lib/g7mediabooster/backups"

[worker]
sandbox_binary = "/usr/local/libexec/g7mb-sandbox"
temp_directory = "/var/lib/g7mediabooster/tmp"
max_temp_disk_bytes = 12884901888
"#,
        values.provider,
        toml_string(&values.tenant_id),
        toml_string(hmac_path),
        toml_string(values.provider.config_name()),
        endpoint,
        toml_string(&values.region),
        toml_string(&values.raw_bucket),
        toml_string(&values.derivative_bucket),
        toml_string(access_path),
        toml_string(secret_path),
        values.force_path_style,
    ))
}

fn validate_bucket(value: String) -> anyhow::Result<String> {
    let bytes = value.as_bytes();
    if !(3..=63).contains(&bytes.len())
        || !bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
        || !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        || value.contains("..")
        || value.contains(".-")
        || value.contains("-.")
        || value.parse::<std::net::Ipv4Addr>().is_ok()
        || value.starts_with("xn--")
        || value.ends_with("-s3alias")
        || value.ends_with("--ol-s3")
    {
        bail!("버킷 이름은 3~63자의 소문자, 숫자, 점, 하이픈만 사용할 수 있습니다");
    }
    Ok(value)
}

fn validate_region(value: String) -> anyhow::Result<String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        bail!("region 값이 올바르지 않습니다");
    }
    Ok(value)
}

fn validate_tenant(value: String) -> anyhow::Result<String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("Tenant ID는 1~64자의 영문, 숫자, 하이픈, 밑줄만 허용합니다");
    }
    Ok(value)
}

fn validate_endpoint(value: String) -> anyhow::Result<String> {
    let url = Url::parse(&value).context("endpoint URL 형식이 올바르지 않습니다")?;
    let host = url.host_str().context("endpoint URL에 host가 없습니다")?;
    let loopback = host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
    {
        bail!("endpoint는 인증정보·경로가 없는 HTTPS URL이어야 합니다");
    }
    Ok(value.trim_end_matches('/').to_owned())
}

fn validate_origins(values: Vec<String>) -> anyhow::Result<Vec<String>> {
    if values.len() > 32 {
        bail!("origin은 최대 32개까지 허용합니다");
    }
    let mut origins = Vec::with_capacity(values.len());
    for value in values {
        let url = Url::parse(&value).context("origin URL 형식이 올바르지 않습니다")?;
        let host = url.host_str().context("origin URL에 host가 없습니다")?;
        let loopback = host == "localhost"
            || host == "::1"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
            || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        {
            bail!("origin은 경로 없는 HTTPS URL이어야 합니다");
        }
        let origin = url.origin().ascii_serialization();
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    Ok(origins)
}

fn read_or_prompt_secret(
    path: Option<&Path>,
    interactive: bool,
    label: &str,
) -> anyhow::Result<SecretString> {
    let value = if let Some(path) = path {
        read_secret_input(path).with_context(|| format!("{label} 파일을 읽지 못했습니다"))?
    } else if interactive {
        rpassword::prompt_password(format!("{label}: "))?
    } else {
        bail!("--non-interactive에는 {label} 입력 파일이 필요합니다");
    };
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n', '\0']) || value.trim() != value {
        bail!("{label} 값이 비어 있거나 올바르지 않습니다");
    }
    Ok(SecretString::from(value.to_owned()))
}

fn read_secret_input(path: &Path) -> anyhow::Result<String> {
    if !path.is_absolute() {
        bail!("자격증명 입력 파일은 절대 경로여야 합니다");
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096
    {
        bail!("자격증명 입력은 4096바이트 이하의 일반 파일이어야 합니다");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        if metadata.permissions().mode() & 0o037 != 0 {
            bail!("자격증명 입력 파일은 group 쓰기 또는 other 접근을 허용하면 안 됩니다");
        }
    }
    let file = fs::File::open(path)?;
    let mut value = String::new();
    file.take(4097).read_to_string(&mut value)?;
    if value.len() > 4096 {
        bail!("자격증명 입력이 4096바이트를 초과합니다");
    }
    Ok(value)
}

fn random_secret() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("OS 난수 생성에 실패했습니다")?;
    Ok(hex::encode(bytes))
}

fn atomic_write(path: &Path, contents: &[u8], mode: u32, force: bool) -> anyhow::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.file_type().is_file() || metadata.file_type().is_symlink())
    {
        bail!(
            "{} 대상은 일반 비심볼릭링크 파일이어야 합니다",
            path.display()
        );
    }
    if let Ok(existing) = fs::read(path) {
        if existing == contents {
            set_mode(path, mode)?;
            return Ok(());
        }
        if !force {
            bail!(
                "{} 파일이 이미 있습니다. 교체하려면 --force가 필요합니다",
                path.display()
            );
        }
    }
    let parent = path
        .parent()
        .context("대상 파일에 상위 디렉터리가 없습니다")?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .context("대상 파일명이 UTF-8이 아닙니다")?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .with_context(|| format!("임시 파일 {} 생성에 실패했습니다", temporary.display()))?;
    let result = (|| -> anyhow::Result<()> {
        file.write_all(contents)?;
        file.sync_all()?;
        set_mode(&temporary, mode)?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ignored = fs::remove_file(&temporary);
    }
    result
}

fn preflight_target(path: &Path, contents: &[u8], force: bool) -> anyhow::Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!(
            "{} 대상은 일반 비심볼릭링크 파일이어야 합니다",
            path.display()
        );
    }
    let existing = fs::read(path)?;
    if existing != contents && !force {
        bail!(
            "{} 파일이 이미 있습니다. 교체하려면 --force가 필요합니다",
            path.display()
        );
    }
    Ok(())
}

fn create_directory(path: &Path, mode: u32) -> anyhow::Result<()> {
    fs::create_dir_all(path)?;
    set_mode(path, mode)
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

fn set_config_group(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("설정 파일에 상위 디렉터리가 없습니다")?;
    let status = ProcessCommand::new("chgrp")
        .arg("g7mediabooster")
        .arg(parent)
        .arg(path)
        .status()
        .context("chgrp를 실행하지 못했습니다")?;
    if !status.success() {
        bail!("설정 파일의 g7mediabooster 그룹 적용에 실패했습니다");
    }
    Ok(())
}

fn required_value(value: Option<String>, interactive: bool, label: &str) -> anyhow::Result<String> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ if interactive => prompt_required(label),
        _ => bail!("--non-interactive에는 {label} 값이 필요합니다"),
    }
}

fn optional_or_prompt(
    value: Option<String>,
    interactive: bool,
    label: &str,
    default: &str,
) -> anyhow::Result<String> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ if interactive => prompt_default(label, default),
        _ => Ok(default.to_owned()),
    }
}

fn prompt_provider() -> anyhow::Result<Provider> {
    println!("저장소 공급자를 선택하십시오:");
    println!("  1) Cloudflare R2");
    println!("  2) AWS S3");
    println!("  3) Amazon Lightsail bucket");
    println!("  4) Generic S3-compatible");
    loop {
        match prompt_required("선택 [1-4]")?.as_str() {
            "1" => return Ok(Provider::R2),
            "2" => return Ok(Provider::AwsS3),
            "3" => return Ok(Provider::Lightsail),
            "4" => return Ok(Provider::Generic),
            _ => println!("1~4 중 하나를 입력하십시오."),
        }
    }
}

fn prompt_required(label: &str) -> anyhow::Result<String> {
    loop {
        let value = prompt(label)?;
        if !value.is_empty() {
            return Ok(value);
        }
        println!("필수 값입니다.");
    }
}

fn prompt_default(label: &str, default: &str) -> anyhow::Result<String> {
    let value = prompt(&format!("{label} [{default}]"))?;
    Ok(if value.is_empty() {
        default.to_owned()
    } else {
        value
    })
}

fn prompt_yes_no(label: &str, default: bool) -> anyhow::Result<bool> {
    let suffix = if default { "Y/n" } else { "y/N" };
    loop {
        let value = prompt(&format!("{label} [{suffix}]"))?.to_ascii_lowercase();
        match value.as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("y 또는 n을 입력하십시오."),
        }
    }
}

fn prompt(label: &str) -> anyhow::Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_owned())
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn path_text(path: &Path) -> anyhow::Result<&str> {
    path.to_str().context("설정 경로가 UTF-8이 아닙니다")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use secrecy::{ExposeSecret as _, SecretString};

    use super::{
        Provider, SetupArgs, SetupValues, atomic_write, collect_setup_values, render_config,
        validate_endpoint, validate_origins,
    };

    #[test]
    fn lightsail_setup_rejects_s3_bucket_and_cors_management() {
        let mut args = SetupArgs {
            non_interactive: true,
            provider: Some(Provider::Lightsail),
            account_id: None,
            endpoint_url: None,
            region: Some("ap-northeast-2".to_owned()),
            bucket: Some("one-private-bucket".to_owned()),
            derivative_bucket: None,
            origins: Vec::new(),
            access_key_id_file: None,
            secret_access_key_file: None,
            tenant_id: Some("site-a".to_owned()),
            config: PathBuf::from("/tmp/g7mb.toml"),
            secrets_dir: PathBuf::from("/tmp/g7mb-secrets"),
            create_buckets: true,
            skip_cors: true,
            defer_storage: true,
            force_path_style: false,
            force: false,
            skip_ownership: true,
        };
        assert!(
            collect_setup_values(&args)
                .err()
                .is_some_and(|error| error.to_string().contains("버킷을 생성할 수 없습니다"))
        );

        args.create_buckets = false;
        args.skip_cors = false;
        assert!(
            collect_setup_values(&args)
                .err()
                .is_some_and(|error| error.to_string().contains("--skip-cors"))
        );
    }

    #[test]
    fn generated_config_loads_secrets_from_files_and_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let access = directory.path().join("access");
        let secret = directory.path().join("secret");
        let hmac = directory.path().join("hmac");
        atomic_write(&access, b"access-id", 0o600, false)?;
        atomic_write(&secret, b"secret-key", 0o600, false)?;
        atomic_write(&hmac, b"0123456789abcdef0123456789abcdef", 0o600, false)?;
        atomic_write(&access, b"access-id", 0o600, false)?;
        let values = SetupValues {
            provider: Provider::R2,
            endpoint_url: Some(
                "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com".to_owned(),
            ),
            region: "auto".to_owned(),
            raw_bucket: "g7mb-private".to_owned(),
            derivative_bucket: "g7mb-private".to_owned(),
            origins: vec!["https://example.com".to_owned()],
            access_key_id: SecretString::from("access-id".to_owned()),
            secret_access_key: SecretString::from("secret-key".to_owned()),
            tenant_id: "site-a".to_owned(),
            force_path_style: false,
            create_buckets: true,
            configure_cors: true,
            defer_storage: true,
        };
        let config = directory.path().join("g7mb.toml");
        fs::write(&config, render_config(&values, &access, &secret, &hmac)?)?;
        let settings = g7mb_config::Settings::load(Some(&config))?;
        assert_eq!(settings.storage.provider, g7mb_config::StorageProvider::R2);
        assert_eq!(settings.storage.access_key_id.expose_secret(), "access-id");
        assert_eq!(
            settings.auth.hmac_secret.expose_secret(),
            "0123456789abcdef0123456789abcdef"
        );
        Ok(())
    }

    #[test]
    fn endpoint_and_origin_validation_are_fail_closed() {
        assert!(validate_endpoint("https://s3.example.com".to_owned()).is_ok());
        assert!(validate_endpoint("http://127.0.0.1:9000".to_owned()).is_ok());
        assert!(validate_endpoint("http://s3.example.com".to_owned()).is_err());
        assert!(validate_endpoint("https://user:pass@s3.example.com".to_owned()).is_err());
        assert!(validate_origins(vec!["https://example.com/path".to_owned()]).is_err());
        assert_eq!(
            validate_origins(vec![
                "https://example.com".to_owned(),
                "https://example.com/".to_owned(),
            ])
            .ok()
            .map(|values| values.len()),
            Some(1)
        );
    }
}
