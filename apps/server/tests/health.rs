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

use aircraft_testsupport::{DockerPostgres, start_postgres};
use anyhow::{Context, Result, anyhow};
use tokio::{
  io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
  net::{TcpListener, TcpStream},
  process::{Child, Command},
  time::{Instant, timeout, timeout_at},
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

/// A password distinctive enough that a substring search cannot match it by
/// accident, and that must never reach the binary's diagnostics.
const PASSWORD: &str = "n0t-in-any-diagnostic";

/// Builds a command for the shipped binary with a clean configuration
/// environment, so an `APP__` variable in the developer's shell cannot decide
/// what the test binds or connects to.
///
/// `database_url` is `None` only for the gate that asserts what happens when the
/// setting is absent; every other case must supply one, because the binary now
/// builds a pool before it binds.
fn server_command(port: &str, database_url: Option<&str>) -> Command {
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
  if let Some(url) = database_url {
    command.env("APP__DATABASE__URL", url);
  }
  command
}

/// Starts a disposable `PostgreSQL` for a gate that needs the binary to get past
/// pool construction.
///
/// The schema is deliberately not installed. The server answers `SELECT 1` and
/// issues no application query, so the migrations would only cost time.
async fn database() -> Result<DockerPostgres> {
  let (container, _ready) = start_postgres(2, Duration::from_secs(2))
    .await
    .map_err(|error| anyhow!("starting a disposable PostgreSQL: {error}"))?;
  Ok(container)
}

/// A URL that parses and names a host, pointing at a port nothing listens on.
///
/// Used by the gates that must reach a *connection* failure rather than a
/// configuration one, and carrying [`PASSWORD`] so the diagnostic can be checked
/// for it.
fn unreachable_database_url() -> String {
  format!("postgres://aircraft_api_app:{PASSWORD}@127.0.0.1:1/aircraft")
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
/// with its own bind diagnostic, and only that is retried. A reported failure is
/// returned immediately, so a broken binary fails once instead of once per
/// attempt.
///
/// Startup logs more than one line -- the pool reports itself ready before the
/// listener reports its address -- so lines are read until one of the three
/// outcomes appears rather than matching on the first. Unrecognized lines are
/// skipped, which is safe because a real failure always prints `Error:` and a
/// silent exit ends the stream.
async fn start(database_url: &str) -> Result<(Child, SocketAddr)> {
  let mut lost = Vec::new();

  for _ in 0..BIND_ATTEMPTS {
    let port = probe_port().await?;
    let mut child = server_command(&port.to_string(), Some(database_url))
      .spawn()
      .context("spawning aircraft-server")?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr pipe"))?;
    let mut lines = BufReader::new(stderr).lines();
    let deadline = Instant::now() + STARTUP;

    loop {
      let line = timeout_at(deadline, lines.next_line())
        .await
        .context("aircraft-server did not log a listening address in time")?
        .context("reading aircraft-server stderr")?
        .ok_or_else(|| anyhow!("aircraft-server exited before logging an address"))?;

      let plain = strip_ansi(&line);
      if let Some((_, reported)) = plain.rsplit_once("address=") {
        let address = reported.trim().parse().context("parsing the logged address")?;
        // The reader is kept alive and drained for the rest of the child's
        // life. Dropping it here would close the read end of the pipe, and a
        // server that logs anything afterwards -- everything a shutdown emits
        // -- would be writing to a broken pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        return Ok((child, address));
      }
      if plain.contains(&format!("binding 127.0.0.1:{port}")) {
        child.wait().await.context("reaping a server that lost its port")?;
        lost.push(port);
        break;
      }
      if plain.contains("Error:") {
        return Err(anyhow!("aircraft-server failed to start: {plain}"));
      }
    }
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
  let container = database().await?;
  let (mut server, address) = start(&container.database_url).await?;

  let (status, body) = get(address, "/health").await?;
  assert_eq!(status, 200, "health must be served: {body}");
  assert_eq!(body, r#"{"status":"ok"}"#);

  server.kill().await?;
  Ok(())
}

/// The readiness route through the shipped binary, against the pool the binary
/// actually built. `aircraft_db`'s own gates prove the probe; this proves the
/// composition root wired that probe rather than something that always answers.
#[tokio::test]
async fn the_running_binary_reports_readiness_against_its_own_pool() -> Result<()> {
  let container = database().await?;
  let (mut server, address) = start(&container.database_url).await?;

  let (status, body) = get(address, "/ready").await?;
  assert_eq!(status, 200, "readiness must be served: {body}");
  assert_eq!(body, r#"{"status":"ready"}"#);

  server.kill().await?;
  Ok(())
}

/// The version the binary reports is its own package version, not the API
/// crate's. They match today, which is exactly why this asserts the value from
/// `CARGO_PKG_VERSION` rather than a literal: the gate has to keep meaning
/// something after the two versions diverge.
#[tokio::test]
async fn the_running_binary_reports_its_own_package_version() -> Result<()> {
  let container = database().await?;
  let (mut server, address) = start(&container.database_url).await?;

  let (status, body) = get(address, "/version").await?;
  assert_eq!(status, 200, "version must be served: {body}");
  assert!(
    body.contains(&format!(r#""version":"{}""#, env!("CARGO_PKG_VERSION"))),
    "the binary must report its own version: {body}"
  );

  server.kill().await?;
  Ok(())
}

/// The composition root mounts `aircraft_api::router()` and nothing else, so a
/// path that crate does not declare must reach no handler. Without this, the
/// gate above would still pass against a root that had quietly bolted on a
/// catch-all.
#[tokio::test]
async fn an_undeclared_route_is_not_served() -> Result<()> {
  let container = database().await?;
  let (mut server, address) = start(&container.database_url).await?;

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
  let container = database().await?;
  let occupied = TcpListener::bind("127.0.0.1:0").await.context("occupying a port")?;
  let port = occupied.local_addr().context("reading the occupied port")?.port();

  let child = server_command(&port.to_string(), Some(&container.database_url))
    .spawn()
    .context("spawning aircraft-server")?;
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
///
/// The database URL points nowhere reachable on purpose. Reaching the port
/// diagnostic anyway is what shows configuration is validated before anything is
/// connected: were the order reversed, this would fail on the pool instead.
#[tokio::test]
async fn an_os_assigned_port_request_is_refused_before_binding() -> Result<()> {
  let child = server_command("0", Some(&unreachable_database_url()))
    .spawn()
    .context("spawning aircraft-server")?;
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

/// The boundary gate for the setting itself, mirroring the port-zero gate above.
/// `aircraft_config` proves the rejection; this proves the binary surfaces it as
/// a startup failure naming the setting, rather than starting without a pool.
#[tokio::test]
async fn a_missing_database_url_exits_non_zero_naming_its_setting() -> Result<()> {
  let child = server_command("8080", None).spawn().context("spawning aircraft-server")?;
  let output = timeout(STARTUP, child.wait_with_output())
    .await
    .context("aircraft-server did not exit without a database URL")?
    .context("collecting aircraft-server output")?;

  assert!(!output.status.success(), "a missing URL must not start a server: {:?}", output.status);

  let stderr = String::from_utf8(output.stderr).context("aircraft-server stderr is not UTF-8")?;
  assert!(stderr.contains("database.url"), "the diagnostic must name the setting path: {stderr}");
  assert!(!stderr.contains("panicked"), "a missing setting must not panic: {stderr}");
  Ok(())
}

/// Acceptance criterion 4. A database the server cannot reach must end the
/// process, and the diagnostic an operator pastes into a ticket must be safe to
/// paste.
///
/// All four assertions are load-bearing. The absence of the password would hold
/// for a binary that printed nothing at all, so the gate also requires the exit
/// to be non-zero, the failure to name the database, and the process not to have
/// panicked its way there.
#[tokio::test]
async fn an_unreachable_database_exits_non_zero_without_echoing_the_credential() -> Result<()> {
  let child = server_command("8080", Some(&unreachable_database_url()))
    .spawn()
    .context("spawning aircraft-server")?;
  let output = timeout(STARTUP, child.wait_with_output())
    .await
    .context("aircraft-server did not exit after failing to reach the database")?
    .context("collecting aircraft-server output")?;

  assert!(
    !output.status.success(),
    "an unreachable database must not start a server: {:?}",
    output.status
  );

  let stderr = String::from_utf8(output.stderr).context("aircraft-server stderr is not UTF-8")?;
  assert!(
    stderr.contains("database"),
    "the diagnostic must name what could not be reached: {stderr}"
  );
  assert!(!stderr.contains("panicked"), "an unreachable database must not panic: {stderr}");
  // Deliberately does not render stderr: printing it on failure would put the
  // credential in the very output this gate exists to keep clean.
  assert!(!stderr.contains(PASSWORD), "the startup diagnostic echoed the credential");
  Ok(())
}

/// The pool is built before the listener, so a server that cannot reach its
/// database never takes the port. Without this, that ordering would be a comment
/// in `main` that nothing checks: reversing it leaves every other gate green,
/// because both failures still exit non-zero.
#[tokio::test]
async fn an_unreachable_database_is_reported_before_the_port_is_taken() -> Result<()> {
  let occupied = TcpListener::bind("127.0.0.1:0").await.context("occupying a port")?;
  let port = occupied.local_addr().context("reading the occupied port")?.port();

  let child = server_command(&port.to_string(), Some(&unreachable_database_url()))
    .spawn()
    .context("spawning aircraft-server")?;
  let output = timeout(STARTUP, child.wait_with_output())
    .await
    .context("aircraft-server did not exit with both the port and the database unusable")?
    .context("collecting aircraft-server output")?;

  assert!(!output.status.success(), "neither failure may exit zero: {:?}", output.status);

  let stderr = String::from_utf8(output.stderr).context("aircraft-server stderr is not UTF-8")?;
  assert!(
    !stderr.contains(&format!("binding 127.0.0.1:{port}")),
    "the bind must not be attempted before the pool: {stderr}"
  );
  assert!(stderr.contains("database"), "the database failure must be the one reported: {stderr}");
  Ok(())
}

/// The only gate that proves `main` asked for the signal at all. Every drain
/// test in `apps/server/tests/shutdown.rs` hands `serve` its own shutdown
/// future, so all of them would still pass against a binary that installed no
/// handler and died on the default disposition.
///
/// The health check before the signal is load-bearing: without it, a zero exit
/// could be a process that had already finished starting and stopped on its
/// own.
#[tokio::test]
async fn sigterm_shuts_the_binary_down_and_exits_zero() -> Result<()> {
  let container = database().await?;
  let (mut server, address) = start(&container.database_url).await?;

  let (status, body) = get(address, "/health").await?;
  assert_eq!(status, 200, "the server must be serving before it is signalled: {body}");

  let pid = server.id().ok_or_else(|| anyhow!("the running server reported no process id"))?;
  // Discrete checked arguments, never a shell string: the only value here is a
  // process id this test just read back from the child it spawned.
  let signalled = Command::new("kill")
    .arg("-TERM")
    .arg(pid.to_string())
    .status()
    .await
    .context("sending SIGTERM to aircraft-server")?;
  assert!(signalled.success(), "kill -TERM did not succeed: {signalled:?}");

  let exit = timeout(STARTUP, server.wait())
    .await
    .context("aircraft-server did not exit after SIGTERM")?
    .context("reaping aircraft-server")?;
  assert!(exit.success(), "a graceful shutdown must exit zero: {exit:?}");
  Ok(())
}
