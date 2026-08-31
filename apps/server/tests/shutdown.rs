// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Drain gates for [`aircraft_server::serve`].
//!
//! These call `serve` in process rather than driving the shipped binary. The
//! drain window is a race between a request and a deadline, and only a caller
//! that owns both ends of it can decide the winner: the readiness port is a
//! trait, so a probe here parks a handler exactly where the test needs it, and
//! the shutdown future is a parameter, so the signal arrives when the test says
//! it does. No container is started, because no query is issued.
//!
//! `apps/server/tests/health.rs` owns the other half -- that the shipped binary
//! installs the signal handler at all.

use std::{
  sync::{Arc, Mutex},
  time::Duration,
};

use aircraft_api::{ApiState, shutdown::ShutdownState};
use aircraft_app::{ingestion::PersistenceError, readiness::ReadinessProbe};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use tokio::{
  io::{AsyncReadExt as _, AsyncWriteExt as _},
  net::{TcpListener, TcpStream},
  sync::{Notify, mpsc, oneshot},
  time::timeout,
};
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::fmt::MakeWriter;

/// Bounds every wait in this file. Nothing here should take milliseconds, so
/// the value only decides how long a broken `serve` hangs before it is reported
/// as a failure rather than a stalled suite.
const DEADLINE: Duration = Duration::from_secs(20);

/// Parks a request inside the handler until the test releases it.
///
/// Announcing entry is what removes every sleep from these gates: the test
/// knows the request is in flight because the handler said so, not because
/// enough time passed.
struct BlockingProbe {
  entered: mpsc::Sender<()>,
  release: Arc<Notify>,
}

/// Keeps callsites enabled when a sibling test reaches them without a capture.
///
/// `tracing` caches global callsite interest. The sink makes every callsite
/// interesting; the expiry test then replaces it for its server future with a
/// subscriber that records the event.
fn enable_callsites() {
  static ENABLED: std::sync::Once = std::sync::Once::new();
  ENABLED.call_once(|| {
    let sink = tracing_subscriber::fmt().with_writer(std::io::sink).finish();
    let _ = tracing::subscriber::set_global_default(sink);
  });
}

#[async_trait]
impl ReadinessProbe for BlockingProbe {
  async fn check(&self) -> Result<(), PersistenceError> {
    let _ = self.entered.send(()).await;
    self.release.notified().await;
    Ok(())
  }
}

/// Collects formatted `tracing` output so a test can assert on an event.
///
/// The expiry warning is the only place the cancelled-request count exists;
/// without reading it back, "counted in a warning event" would be prose.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
  fn contents(&self) -> String {
    String::from_utf8_lossy(&self.0.lock().expect("the log buffer lock is poisoned")).into_owned()
  }
}

impl std::io::Write for CapturedLogs {
  fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
    self.0.lock().expect("the log buffer lock is poisoned").extend_from_slice(buffer);
    Ok(buffer.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
  type Writer = Self;

  fn make_writer(&'writer self) -> Self::Writer {
    self.clone()
  }
}

struct Harness {
  address: std::net::SocketAddr,
  shutdown: ShutdownState,
  entered: mpsc::Receiver<()>,
  release: Arc<Notify>,
  stop: oneshot::Sender<()>,
  served: tokio::task::JoinHandle<Result<()>>,
}

/// Binds a loopback port, starts `serve` on it, and returns the controls.
///
/// Port zero is asked of the OS here rather than probed and released, because
/// the listener is handed straight to `serve` -- there is no window for another
/// process to take it. Configuration refuses port zero for the deployed binary
/// for the opposite reason: nothing could be told how to reach it.
async fn start(grace: Duration, dispatch: Option<tracing::Dispatch>) -> Result<Harness> {
  enable_callsites();
  let listener = TcpListener::bind("127.0.0.1:0").await.context("binding a loopback port")?;
  let address = listener.local_addr().context("reading the bound address")?;

  let (entered_tx, entered) = mpsc::channel(1);
  let release = Arc::new(Notify::new());
  let shutdown = ShutdownState::new();
  let state = ApiState {
    readiness: Arc::new(BlockingProbe { entered: entered_tx, release: Arc::clone(&release) }),
    version: "9.9.9-test",
    build_commit: None,
    shutdown: shutdown.clone(),
  };

  let (stop, stop_rx) = oneshot::channel::<()>();
  let serving = aircraft_server::serve(
    listener,
    state,
    async move {
      let _ = stop_rx.await;
    },
    grace,
  );
  let served = match dispatch {
    Some(dispatch) => tokio::spawn(serving.with_subscriber(dispatch)),
    None => tokio::spawn(serving),
  };

  Ok(Harness { address, shutdown, entered, release, stop, served })
}

/// Waits for the drain flag without sleeping.
///
/// It yields to the runtime and re-reads, so it returns on the first poll after
/// `serve` sets the flag; the deadline is what bounds a `serve` that never does.
async fn await_draining(shutdown: &ShutdownState) -> Result<()> {
  timeout(DEADLINE, async {
    while !shutdown.is_draining() {
      tokio::task::yield_now().await;
    }
  })
  .await
  .context("the server never began draining")
}

/// Issues one HTTP/1.1 request and returns whatever came back, which may be
/// nothing at all when the connection is cut mid-request.
///
/// `Connection: close` makes the server close the socket after responding,
/// which is what lets `read_to_string` terminate without parsing lengths.
async fn get(address: std::net::SocketAddr, path: &str) -> Result<String> {
  let mut stream = TcpStream::connect(address).await.context("connecting to the server")?;
  stream
    .write_all(
      format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .context("sending the request")?;

  let mut response = String::new();
  stream.read_to_string(&mut response).await.context("reading the response")?;
  Ok(response)
}

fn status_of(response: &str) -> Result<u16> {
  response
    .split_whitespace()
    .nth(1)
    .ok_or_else(|| anyhow!("no status code in response: {response:?}"))?
    .parse()
    .context("parsing the status code")
}

/// Acceptance criterion 2. The request is admitted before the signal and must
/// still finish: the drain window exists to let it.
///
/// The handshake is what makes this mean something. Waiting for the handler to
/// announce itself, then for the drain flag to be set, fixes the order as
/// admitted -> draining -> completed; without both waits the request could
/// finish before shutdown ever began and the gate would pass against a `serve`
/// that drains nothing.
#[tokio::test]
async fn an_in_flight_request_completes_inside_the_drain_window() -> Result<()> {
  let mut harness = start(Duration::from_secs(30), None).await?;
  let address = harness.address;
  let request = tokio::spawn(async move { get(address, "/ready").await });

  harness.entered.recv().await.context("the request never reached the handler")?;
  harness.stop.send(()).map_err(|()| anyhow!("serve stopped before the signal was sent"))?;
  await_draining(&harness.shutdown).await?;
  harness.release.notify_one();

  let response = timeout(DEADLINE, request).await.context("the request never completed")???;
  assert_eq!(status_of(&response)?, 200, "a request admitted before the drain must finish");

  let served = timeout(DEADLINE, harness.served).await.context("serve never returned")??;
  assert!(served.is_ok(), "a completed drain is not a server failure: {served:?}");
  Ok(())
}

/// Acceptance criterion 3, forced without sleeping for synchronization.
///
/// `Duration::ZERO` is the deadline: `tokio::time::timeout` polls its inner
/// future once against an already-elapsed timer, so expiry is decided by the
/// value under test rather than by how fast the machine is. The request is
/// parked in the handler and never released, so the count at expiry is exactly
/// one.
///
/// The log assertion is the criterion. `serve` returning `Ok` would also hold
/// for a shutdown that cancelled the request silently, and it is the number an
/// operator needs to know a rollout dropped work.
///
/// The handler is never released, so the 503 can only come from the
/// cancellation path. The empty body distinguishes this cross-cutting response
/// from `/ready`'s structured shutdown problem.
#[tokio::test]
async fn a_request_still_running_at_expiry_is_cancelled_and_counted() -> Result<()> {
  let logs = CapturedLogs::default();
  let subscriber =
    tracing_subscriber::fmt().with_writer(logs.clone()).with_ansi(false).without_time().finish();
  let mut harness = start(Duration::ZERO, Some(tracing::Dispatch::new(subscriber))).await?;
  let address = harness.address;
  let request = tokio::spawn(async move { get(address, "/ready").await });

  harness.entered.recv().await.context("the request never reached the handler")?;
  harness.stop.send(()).map_err(|()| anyhow!("serve stopped before the signal was sent"))?;

  let served = timeout(DEADLINE, harness.served).await.context("serve never returned")??;
  assert!(served.is_ok(), "an expired drain is a completed shutdown: {served:?}");
  assert_eq!(
    harness.shutdown.in_flight(),
    0,
    "serve returned before cancelled handlers released their guards"
  );

  let response = timeout(DEADLINE, request).await.context("the request never completed")???;
  assert_eq!(status_of(&response)?, 503, "a cancelled request must not report success");
  let (_, body) = response.split_once("\r\n\r\n").context("the response had no header boundary")?;
  assert!(body.is_empty(), "forced cancellation must return a bodyless 503: {response:?}");

  let captured = logs.contents();
  assert!(
    captured.contains("shutdown grace expired before requests drained"),
    "the expiry must be warned about: {captured}"
  );
  assert!(
    captured.contains("cancelled=1"),
    "the warning must count the requests it cut short: {captured}"
  );

  Ok(())
}

/// Pins `hyper`'s behaviour, not this crate's bound.
///
/// A client that stops midway through its next request never reaches the
/// middleware, so cancellation cannot wake it -- and `hyper` closes a
/// connection that is between requests when graceful shutdown begins, which is
/// what keeps it from outliving the deadline. Mutation-checked: this passes
/// with the post-cancellation flush bounded and unbounded alike, so it does not
/// pin `CANCELLED_FLUSH`. It is here to catch an `axum` or `hyper` upgrade that
/// starts waiting on such a connection instead, which is what would make that
/// bound load-bearing.
#[tokio::test]
async fn a_partial_request_cannot_keep_the_server_alive_after_cancellation() -> Result<()> {
  let mut harness = start(Duration::ZERO, None).await?;
  let address = harness.address;

  let parked = tokio::spawn(async move { get(address, "/ready").await });
  harness.entered.recv().await.context("the request never reached the handler")?;

  let mut partial = TcpStream::connect(address).await.context("opening a keep-alive connection")?;
  partial
    .write_all(format!("GET /health HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
    .await
    .context("sending the complete request")?;

  timeout(DEADLINE, async {
    let mut response = Vec::new();
    loop {
      let mut chunk = [0_u8; 256];
      let read = partial.read(&mut chunk).await.context("reading the complete response")?;
      if read == 0 {
        return Err(anyhow!("the keep-alive connection closed before serving health"));
      }
      response.extend_from_slice(chunk.get(..read).unwrap_or_default());
      if String::from_utf8_lossy(&response).contains(r#"{"status":"ok"}"#) {
        return Ok::<(), anyhow::Error>(());
      }
    }
  })
  .await
  .context("the complete request was not served")??;

  partial
    .write_all(format!("GET /health HTTP/1.1\r\nHost: {address}\r\n").as_bytes())
    .await
    .context("sending the partial request")?;
  harness.stop.send(()).map_err(|()| anyhow!("serve stopped before the signal was sent"))?;

  let served = timeout(Duration::from_secs(2), harness.served)
    .await
    .context("a partial request kept the server alive after forced cancellation")??;
  assert!(served.is_ok(), "forced cancellation is not a server failure: {served:?}");

  drop(partial);
  drop(parked);
  Ok(())
}
