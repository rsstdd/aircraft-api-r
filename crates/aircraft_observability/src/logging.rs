pub fn init() {
  tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
    .init();
}
