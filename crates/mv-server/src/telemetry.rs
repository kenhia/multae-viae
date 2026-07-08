//! Tracing / OpenTelemetry initialization and shutdown.
//!
//! Mirrors `mv-cli`'s telemetry wiring so a span exported from the server is
//! shaped like one from the CLI (same `gen_ai.*` conventions, same OTLP/HTTP
//! exporter), differing only in `service.name`. The per-request HTTP server
//! span is added as a tower layer in [`crate::build_router`]; the `gen_ai.*`
//! completion spans created inside `mv_core::runtime` nest under it.

use std::sync::OnceLock;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

static TRACER_PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> = OnceLock::new();

/// Initialize the global subscriber: a stderr fmt layer plus, when
/// `otlp_endpoint` is set, an OTLP/HTTP exporter layer.
pub fn init_tracing(otlp_endpoint: Option<&str>) {
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    use std::io::IsTerminal as _;
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_filter(console_filter);

    let otel_layer = otlp_endpoint.and_then(|endpoint| match init_otel_layer(endpoint) {
        Ok(layer) => Some(layer.with_filter(EnvFilter::new("info"))),
        Err(e) => {
            eprintln!("Warning: Failed to initialize OTLP exporter: {e}");
            None
        }
    });

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_layer)
        .init();
}

fn init_otel_layer<S>(
    endpoint: &str,
) -> Result<
    tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>,
    Box<dyn std::error::Error>,
>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};

    // with_endpoint() uses the URL as-is; append /v1/traces for OTLP/HTTP.
    let traces_endpoint = if endpoint.ends_with("/v1/traces") {
        endpoint.to_string()
    } else {
        format!("{}/v1/traces", endpoint.trim_end_matches('/'))
    };

    let exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(&traces_endpoint)
        .build()?;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name("mv-server")
                .build(),
        )
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer("mv-server");
    let _ = TRACER_PROVIDER.set(provider);

    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}

/// Flush and shut down the tracer provider. Called during graceful shutdown so
/// the last batch of spans is exported before exit.
pub fn shutdown_tracing() {
    if let Some(provider) = TRACER_PROVIDER.get() {
        if let Err(e) = provider.force_flush() {
            eprintln!("Warning: failed to flush traces: {e:?}");
        }
        if let Err(e) = provider.shutdown() {
            eprintln!("Warning: failed to shutdown tracer: {e:?}");
        }
    }
}
