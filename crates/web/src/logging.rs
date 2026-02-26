use rust_template_lib::{LogFormat, LogLevel};
use tracing_subscriber::{
  fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
};

// Web services write program output to stdout and all log output to stderr so
// that the two streams can be separated by process supervisors and log
// aggregators.
pub fn init_logging(level: LogLevel, format: LogFormat) {
  let env_filter = EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| EnvFilter::new(level.to_string()));

  match format {
    LogFormat::Text => {
      tracing_subscriber::registry()
        .with(
          fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_line_number(true)
            .with_filter(env_filter),
        )
        .init();
    }
    LogFormat::Json => {
      tracing_subscriber::registry()
        .with(
          fmt::layer()
            .json()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_line_number(true)
            .with_filter(env_filter),
        )
        .init();
    }
  }
}
