//! Structured logging and Prometheus recorder setup.

use std::net::SocketAddr;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use thiserror::Error;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

/// Telemetry initialization failure.
#[derive(Debug, Error)]
pub enum TelemetryError {
    /// A global tracing subscriber already exists or could not be installed.
    #[error("failed to install tracing subscriber: {0}")]
    Tracing(String),
    /// A global metrics recorder already exists or could not be installed.
    #[error("failed to install metrics recorder: {0}")]
    Metrics(String),
}

/// Installs JSON structured tracing using `RUST_LOG` or an info-level default.
pub fn init_tracing() -> Result<(), TelemetryError> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => EnvFilter::new("info,tower_http=info"),
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(std::io::stderr),
        )
        .try_init()
        .map_err(|error| TelemetryError::Tracing(error.to_string()))
}

/// Installs the process-wide Prometheus recorder and returns its render handle.
pub fn install_metrics() -> Result<PrometheusHandle, TelemetryError> {
    PrometheusBuilder::new()
        .install_recorder()
        .map_err(|error| TelemetryError::Metrics(error.to_string()))
}

/// Installs a loopback HTTP Prometheus exporter for a long-running worker process.
pub fn install_metrics_http(bind_addr: SocketAddr) -> Result<(), TelemetryError> {
    if !bind_addr.ip().is_loopback() {
        return Err(TelemetryError::Metrics(
            "worker metrics listener must be loopback-only".to_owned(),
        ));
    }
    PrometheusBuilder::new()
        .with_http_listener(bind_addr)
        .install()
        .map_err(|error| TelemetryError::Metrics(error.to_string()))
}
