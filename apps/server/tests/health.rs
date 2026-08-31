// A failing assertion is the point of a test, so panicking accessors are fine.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Deployment gates for the `aircraft-server` composition root.
//!
//! These drive the shipped binary rather than calling `serve` in process. The
//! router already has an in-process test in `aircraft_api`, so what is left to
//! prove is what only a real process can show: that the binary binds a socket
//! from configuration, serves the router over TCP, and fails a bind with a
//! non-zero status instead of panicking.
//!
//! The port is probed rather than fixed, and then read back from the startup
//! log. A hard-coded port collides with whatever else the machine is running,
//! and `APP__HTTP__PORT=0` is not available either: configuration rejects it,
//! because an OS-assigned port leaves nothing able to reach a deployed process.
//! Probing binds a port and releases it, which leaves a window for another
//! process to take it, so [`start`] retries a lost port instead of failing.

use std::{net::SocketAddr, process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow};
use tokio::{
  io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
  net::{TcpListener, TcpStream},
  process::{Child, Command},
  time::timeout,
};

/// How long the binary gets to report its bound address, or to exit.
const STARTUP: Duration = Duration::from_secs(20);
const NETWORK_IO_TIMEOUT: Duration = Duration::from_secs(20);

/// How many probed ports may be lost to another process before `start` gives up.
///
/// Losing the probe-to-spawn race is rare and independent per attempt, so a
/// small bound already makes a spurious failure negligible; losing it eight
/// times running is a machine problem worth reporting rather than absorbing.
const BIND_ATTEMPTS: usize = 8;

/// Builds a command for the shipped binary with a clean configuration
/// environment, so an `APP__` variable in the developer's shell cannot decide
/// what the test binds.
fn server_command(port: &str) -> Command {
  let mut command = Command::new(env!("CARGO_BIN_EXE_aircraft-server"));
  for (key, _) in std::env::vars() {
    if key.starts_with("APP__") {
      command.env_remove(key);
    }
  }
  command
    .env("APP__HTTP__HOST", "127.0.0.1")
    .env("APP__HTTP__PORT", port)
    .env("RUST_LOG", "info")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
  command
}

/// Removes ANSI escape sequences from a log line.
///
/// `tracing_subscriber`'s human format colors its output even when stderr is a
/// pipe, which splits `address=` across escape sequences. The test reads a
/// format meant for people, so it strips the styling rather than asking the
/// server to log differently for tests.
fn strip_ansi(line: &str) -> String {
  let mut plain = String::with_capacity(line.len());
  let mut characters = line.chars();
  while let Some(character) = characters.next() {
    if character == '\u{1b}' {
      // A CSI sequence runs until its final byte in the range @ to ~.
      for escaped in characters.by_ref() {
        if escaped.is_ascii_alphabetic() {
          break;
        }
      }
    } else {
      plain.push(character);
    }
  }
  plain
}

/// Reserves a free port and releases it, so the spawned binary has a concrete
/// port to ask for.
async fn probe_port() -> Result<u16> {
  let probe = TcpListener::bind("127.0.0.1:0").await.context("probing for a free port")?;
  let port = probe.local_addr().context("reading the probed port")?.port();
  drop(probe);
  Ok(port)
}

/// Starts the binary on a probed port and returns it with the address it
/// reported listening on.
///
/// A port lost between the probe and the spawn shows up as the binary exiting
/// with its own bind diagnostic on the first stderr line, and only that is
/// retried. Any other first line is a real startup failure and is returned
/// immediately, so a broken binary fails once instead of once per attempt.
async fn start() -> Result<(Child, SocketAddr)> {
  let mut lost = Vec::new();

  for _ in 0..BIND_ATTEMPTS {
    let port = probe_port().await?;
    let mut child =
      server_command(&port.to_string()).spawn().context("spawning aircraft-server")?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr pipe"))?;
    let mut lines = BufReader::new(stderr).lines();

    let line = timeout(STARTUP, lines.next_line())
      .await
      .context("aircraft-server did not log a listening address in time")?
      .context("reading aircraft-server stderr")?
      .ok_or_else(|| anyhow!("aircraft-server exited before logging an address"))?;

    let plain = strip_ansi(&line);
    if let Some((_, reported)) = plain.rsplit_once("address=") {
      let address = reported.trim().parse().context("parsing the logged address")?;
      return Ok((child, address));
    }

    if !plain.contains(&format!("binding 127.0.0.1:{port}")) {
      return Err(anyhow!("aircraft-server failed to start: {plain}"));
    }
    child.wait().await.context("reaping a server that lost its port")?;
    lost.push(port);
  }

  Err(anyhow!("every probed port was taken before aircraft-server could bind it: {lost:?}"))
}

/// Issues one HTTP/1.1 request and returns the status code and body.
///
/// `Connection: close` makes the server close the socket after responding,
/// which is what lets `read_to_string` terminate without parsing lengths.
async fn get(address: SocketAddr, path: &str) -> Result<(u16, String)> {
  let mut stream = timeout(NETWORK_IO_TIMEOUT, TcpStream::connect(address))
    .await
    .context("timed out connecting to the server")?
    .context("connecting to the server")?;
  timeout(
    NETWORK_IO_TIMEOUT,
    stream.write_all(
      format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
    ),
  )
  .await
  .context("timed out sending the request")?
  .context("sending the request")?;

  let mut response = String::new();
  timeout(NETWORK_IO_TIMEOUT, stream.read_to_string(&mut response))
    .await
    .context("timed out reading the response")?
    .context("reading the response")?;

  let status = response
    .split_whitespace()
    .nth(1)
    .ok_or_else(|| anyhow!("no status code in response: {response:?}"))?
    .parse()
    .context("parsing the status code")?;
  let body = response
    .split_once("\r\n\r\n")
    .ok_or_else(|| anyhow!("no body in response: {response:?}"))?
    .1
    .to_owned();

  Ok((status, body))
}

#[tokio::test]
async fn the_running_binary_serves_health_over_a_real_socket() -> Result<()> {
  let (mut server, address) = start().await?;

  let (status, body) = get(address, "/health").await?;
  assert_eq!(status, 200, "health must be served: {body}");
  assert_eq!(body, r#"{"status":"ok"}"#);

  server.kill().await?;
  Ok(())
}

/// The composition root mounts `aircraft_api::router()` and nothing else, so a
/// path that crate does not declare must reach no handler. Without this, the
/// gate above would still pass against a root that had quietly bolted on a
/// catch-all.
#[tokio::test]
async fn an_undeclared_route_is_not_served() -> Result<()> {
  let (mut server, address) = start().await?;

  let (status, _) = get(address, "/not-a-route").await?;
  assert_eq!(status, 404);

  server.kill().await?;
  Ok(())
}

/// A port already in use must end the process, not leave it running without a
/// listener. The diagnostic has to name the address so an operator can act on
/// it, and must stay an ordinary error: a panic here would mean the binary
/// reports a routine misconfiguration as a crash.
#[tokio::test]
async fn a_port_already_in_use_exits_non_zero_with_a_safe_diagnostic() -> Result<()> {
  let occupied = TcpListener::bind("127.0.0.1:0").await.context("occupying a port")?;
  let port = occupied.local_addr().context("reading the occupied port")?.port();

  let child = server_command(&port.to_string()).spawn().context("spawning aircraft-server")?;
  let output = timeout(STARTUP, child.wait_with_output())
    .await
    .context("aircraft-server did not exit after failing to bind")?
    .context("collecting aircraft-server output")?;

  assert!(!output.status.success(), "a failed bind must not exit zero: {:?}", output.status);

  let stderr = String::from_utf8(output.stderr).context("aircraft-server stderr is not UTF-8")?;
  assert!(
    stderr.contains(&format!("binding 127.0.0.1:{port}")),
    "the diagnostic must name the address it could not bind: {stderr}"
  );
  assert!(!stderr.contains("panicked"), "a bind failure must not panic: {stderr}");
  Ok(())
}

/// Criterion 3 at the deployment boundary. The unit test in `aircraft_config`
/// proves the rejection; this proves nothing downstream re-accepts it, and that
/// the binary treats it as ordinary misconfiguration rather than a crash.
#[tokio::test]
async fn an_os_assigned_port_request_is_refused_before_binding() -> Result<()> {
  let child = server_command("0").spawn().context("spawning aircraft-server")?;
  let output = timeout(STARTUP, child.wait_with_output())
    .await
    .context("aircraft-server did not exit after rejecting port zero")?
    .context("collecting aircraft-server output")?;

  assert!(!output.status.success(), "port zero must not start a server: {:?}", output.status);

  let stderr = String::from_utf8(output.stderr).context("aircraft-server stderr is not UTF-8")?;
  assert!(stderr.contains("http.port"), "the diagnostic must name the setting path: {stderr}");
  assert!(!stderr.contains("panicked"), "a rejected setting must not panic: {stderr}");
  Ok(())
}
