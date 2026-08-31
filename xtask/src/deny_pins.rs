//! Reconciles the exact-version build-script allowlist in `deny.toml` with the
//! crates `cargo deny` actually rejects.
//!
//! `[bans.build].allow-build-scripts` pins `name@version`, so every dependency
//! bump of a crate that ships a build script invalidates its entry and reddens
//! `cargo deny check bans`. This command turns that into a mechanical edit.
//!
//! The offending crates come from `cargo deny` itself rather than from a
//! second, independently computed dependency graph. An earlier draft walked
//! `cargo metadata` instead and reported four crates `cargo deny` does not
//! object to, because `metadata.packages` is the whole lockfile universe rather
//! than the resolved graph. Deferring to the tool that owns the check keeps the
//! two views from drifting apart.
//!
//! This does not decide whether a new build script is acceptable. It reports
//! the script's path and checksum so a reviewer can judge, and leaves that
//! judgment in the diff.

use std::{
  collections::{BTreeMap, BTreeSet},
  fs,
  path::Path,
  process::Command,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const DENY_MANIFEST: &str = "deny.toml";
/// The line that opens the allowlist. Matched after trimming, so the
/// indentation of the surrounding table does not matter.
const LIST_OPENING: &str = "allow-build-scripts = [";
const LIST_CLOSING: &str = "]";
/// The `cargo deny` diagnostic this command exists to resolve.
const OFFENDING_CODE: &str = "build-script-not-allowed";
/// Indentation of one allowlist entry, matching the checked-in file.
const ENTRY_INDENT: &str = "    ";

#[derive(Debug, Deserialize)]
struct Diagnostic {
  #[serde(default)]
  fields: Fields,
}

#[derive(Debug, Default, Deserialize)]
struct Fields {
  #[serde(default)]
  code: String,
  #[serde(default)]
  graphs: Vec<Graph>,
  #[serde(default)]
  notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Graph {
  #[serde(rename = "Krate")]
  krate: Krate,
}

#[derive(Debug, Deserialize)]
struct Krate {
  name: String,
  version: String,
}

/// A crate `cargo deny` rejected for shipping an unapproved build script.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Offender {
  name: String,
  version: String,
  /// Checksum of the build script, as reported by `cargo deny`. Present so the
  /// reviewer has something stable to compare against.
  checksum: Option<String>,
}

impl Offender {
  fn pin(&self) -> String {
    format!("{}@{}", self.name, self.version)
  }
}

#[derive(Debug)]
pub struct Options {
  /// Rewrite the allowlist instead of only reporting the drift.
  pub fix: bool,
}

pub fn check(workspace_root: &Path, options: &Options) -> Result<()> {
  let manifest_path = workspace_root.join(DENY_MANIFEST);
  let manifest = fs::read_to_string(&manifest_path)
    .with_context(|| format!("failed to read {}", manifest_path.display()))?;
  let pinned = parse_pins(&manifest)?;

  let offenders = parse_diagnostics(&run_deny(workspace_root)?);
  if offenders.is_empty() {
    println!("Build-script pins match the lockfile ({} crates).", pinned.len());
    return Ok(());
  }

  let updated = apply(&pinned, &offenders);
  let report = render(&pinned, &offenders);

  if options.fix {
    let rewritten = rewrite(&manifest, &updated)?;
    fs::write(&manifest_path, rewritten)
      .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    println!("{report}\nRewrote {}. Review the diff before committing.", manifest_path.display());
    return Ok(());
  }

  bail!("{report}\nRun `cargo xtask deny-pins --fix` to update the list, then review the diff.");
}

/// Runs `cargo deny` and returns its combined output.
///
/// A non-zero exit is the normal case here: it is what `cargo deny` does when
/// it finds the very diagnostics this command parses. Only a failure to launch
/// is an error.
fn run_deny(workspace_root: &Path) -> Result<String> {
  let output = Command::new("cargo")
    .args(["deny", "--format", "json", "check", "bans"])
    .current_dir(workspace_root)
    .output()
    .context("failed to start `cargo deny`; run `cargo xtask install-deps`")?;

  let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
  combined.push('\n');
  combined.push_str(&String::from_utf8_lossy(&output.stderr));
  Ok(combined)
}

/// Reads `build-script-not-allowed` diagnostics out of `cargo deny`'s
/// newline-delimited JSON.
///
/// Lines that are not JSON, or are JSON of another shape, are skipped rather
/// than failing the run: `cargo deny` interleaves human-readable progress with
/// its diagnostics, and a future field addition should not break this.
fn parse_diagnostics(output: &str) -> BTreeSet<Offender> {
  output
    .lines()
    .filter_map(|line| serde_json::from_str::<Diagnostic>(line.trim()).ok())
    .filter(|diagnostic| diagnostic.fields.code == OFFENDING_CODE)
    .filter_map(|diagnostic| {
      let graph = diagnostic.fields.graphs.into_iter().next()?;
      Some(Offender {
        name: graph.krate.name,
        version: graph.krate.version,
        checksum: diagnostic
          .fields
          .notes
          .iter()
          .find_map(|note| note.strip_prefix("checksum = '"))
          .and_then(|note| note.strip_suffix('\''))
          .map(str::to_owned),
      })
    })
    .collect()
}

/// Produces the corrected pin set: each offender replaces any existing entry
/// for the same crate, and every unrelated pin is preserved untouched.
fn apply(pinned: &BTreeSet<String>, offenders: &BTreeSet<Offender>) -> BTreeSet<String> {
  let superseded: BTreeMap<&str, &Offender> =
    offenders.iter().map(|offender| (offender.name.as_str(), offender)).collect();

  let mut updated: BTreeSet<String> = pinned
    .iter()
    .filter(|pin| pin.split_once('@').is_none_or(|(name, _)| !superseded.contains_key(name)))
    .cloned()
    .collect();
  updated.extend(offenders.iter().map(Offender::pin));
  updated
}

/// Reads the entries between the allowlist's opening and closing lines.
///
/// A line-oriented reader keeps the surrounding comments, table headers, and
/// key order untouched, which a TOML round-trip would discard.
fn parse_pins(manifest: &str) -> Result<BTreeSet<String>> {
  let Some((_, body)) = manifest.split_once(LIST_OPENING) else {
    bail!("{DENY_MANIFEST} has no `{LIST_OPENING}` list");
  };
  let Some((entries, _)) = body.split_once(&format!("\n{LIST_CLOSING}")) else {
    bail!("{DENY_MANIFEST} has an unterminated `{LIST_OPENING}` list");
  };

  Ok(
    entries
      .lines()
      .map(str::trim)
      .filter(|line| !line.is_empty() && !line.starts_with('#'))
      .map(|line| line.trim_end_matches(',').trim_matches('"').to_owned())
      .collect(),
  )
}

/// Replaces the allowlist body, leaving every other byte of the manifest alone.
fn rewrite(manifest: &str, updated: &BTreeSet<String>) -> Result<String> {
  let Some((head, body)) = manifest.split_once(LIST_OPENING) else {
    bail!("{DENY_MANIFEST} has no `{LIST_OPENING}` list");
  };
  let closing = format!("\n{LIST_CLOSING}");
  let Some((_, tail)) = body.split_once(&closing) else {
    bail!("{DENY_MANIFEST} has an unterminated `{LIST_OPENING}` list");
  };

  let entries = updated
    .iter()
    .map(|entry| format!("{ENTRY_INDENT}\"{entry}\","))
    .collect::<Vec<_>>()
    .join("\n");

  Ok(format!("{head}{LIST_OPENING}\n{entries}{closing}{tail}"))
}

fn render(pinned: &BTreeSet<String>, offenders: &BTreeSet<Offender>) -> String {
  let mut lines = vec!["build-script pins are out of date:".to_owned()];
  for offender in offenders {
    let superseded =
      pinned.iter().find(|pin| pin.split_once('@').is_some_and(|(name, _)| name == offender.name));
    match superseded {
      Some(previous) => lines.push(format!("  {previous} -> {}", offender.pin())),
      None => lines.push(format!("  + {} (not previously pinned)", offender.pin())),
    }
    if let Some(checksum) = &offender.checksum {
      lines.push(format!("      build script checksum {checksum}"));
    }
  }
  lines.join("\n")
}

#[cfg(test)]
mod tests {
  use super::*;

  const MANIFEST: &str = "\
[bans.build]
# Exact versions approved from the current lockfile.
allow-build-scripts = [
    \"anyhow@1.0.103\",
    \"serde_json@1.0.150\",
]
executables = \"deny\"
";

  fn diagnostic(name: &str, version: &str) -> String {
    format!(
      r#"{{"fields":{{"code":"build-script-not-allowed","graphs":[{{"Krate":{{"name":"{name}","version":"{version}"}}}}],"notes":["path = '/x/build.rs'","checksum = 'abc123'"],"severity":"error"}},"type":"diagnostic"}}"#
    )
  }

  /// Returns the sole offender, failing the test rather than panicking when the
  /// set is not a singleton.
  fn single(offenders: &BTreeSet<Offender>) -> Result<&Offender> {
    let mut found = offenders.iter();
    let offender = found.next().context("expected exactly one offender, found none")?;
    if found.next().is_some() {
      bail!("expected exactly one offender, found {}", offenders.len());
    }
    Ok(offender)
  }

  /// Unwraps the error side of a result that must have failed.
  fn require_error<T>(result: Result<T>) -> Result<anyhow::Error> {
    match result {
      Ok(_) => bail!("expected the call to fail"),
      Err(error) => Ok(error),
    }
  }

  #[test]
  fn only_build_script_diagnostics_are_collected() -> Result<()> {
    let output = format!(
      "{}\n{}\n{}",
      diagnostic("anyhow", "1.0.104"),
      r#"{"fields":{"code":"duplicate","graphs":[{"Krate":{"name":"syn","version":"2.0.0"}}]},"type":"diagnostic"}"#,
      "   Compiling something"
    );

    let offenders = parse_diagnostics(&output);

    assert_eq!(single(&offenders)?.pin(), "anyhow@1.0.104");
    Ok(())
  }

  /// The checksum is the reviewer's handle on what actually changed, so losing
  /// it silently would gut the report.
  #[test]
  fn the_build_script_checksum_is_captured() -> Result<()> {
    let offenders = parse_diagnostics(&diagnostic("anyhow", "1.0.104"));

    assert_eq!(single(&offenders)?.checksum.as_deref(), Some("abc123"));
    Ok(())
  }

  #[test]
  fn non_json_output_is_skipped_rather_than_failing() {
    let offenders = parse_diagnostics("not json at all\n\n{ broken");

    assert!(offenders.is_empty(), "{offenders:?}");
  }

  #[test]
  fn pins_are_read_from_the_allowlist_body() -> Result<()> {
    let pinned = parse_pins(MANIFEST)?;

    assert_eq!(
      pinned,
      BTreeSet::from(["anyhow@1.0.103".to_owned(), "serde_json@1.0.150".to_owned()])
    );
    Ok(())
  }

  /// A comment inside the list is not an entry. Without this the parser would
  /// invent a pin and then delete the comment on the next `--fix`.
  #[test]
  fn comments_inside_the_list_are_not_read_as_pins() -> Result<()> {
    let manifest =
      MANIFEST.replace("    \"anyhow@1.0.103\",", "    # pending\n    \"anyhow@1.0.103\",");

    let pinned = parse_pins(&manifest)?;

    assert_eq!(
      pinned,
      BTreeSet::from(["anyhow@1.0.103".to_owned(), "serde_json@1.0.150".to_owned()])
    );
    Ok(())
  }

  #[test]
  fn a_manifest_without_the_list_is_rejected() -> Result<()> {
    let error = require_error(parse_pins("[bans.build]\n"))?;

    assert!(error.to_string().contains("allow-build-scripts"), "{error}");
    Ok(())
  }

  #[test]
  fn an_unterminated_list_is_rejected() -> Result<()> {
    let error = require_error(parse_pins("allow-build-scripts = [\n    \"anyhow@1.0.103\",\n"))?;

    assert!(error.to_string().contains("unterminated"), "{error}");
    Ok(())
  }

  /// The whole point of the command: the superseded version must go, and every
  /// unrelated pin must stay.
  #[test]
  fn an_offender_replaces_only_its_own_crates_pin() -> Result<()> {
    let pinned = parse_pins(MANIFEST)?;
    let offenders = parse_diagnostics(&diagnostic("anyhow", "1.0.104"));

    let updated = apply(&pinned, &offenders);

    assert!(updated.contains("anyhow@1.0.104"), "{updated:?}");
    assert!(!updated.contains("anyhow@1.0.103"), "the superseded pin must go: {updated:?}");
    assert!(updated.contains("serde_json@1.0.150"), "unrelated pins must stay: {updated:?}");
    Ok(())
  }

  #[test]
  fn a_crate_that_was_never_pinned_is_added() -> Result<()> {
    let pinned = parse_pins(MANIFEST)?;
    let offenders = parse_diagnostics(&diagnostic("libc", "0.2.186"));

    let updated = apply(&pinned, &offenders);

    assert!(updated.contains("libc@0.2.186"), "{updated:?}");
    assert_eq!(updated.len(), 3, "nothing else may be dropped: {updated:?}");
    Ok(())
  }

  #[test]
  fn rewriting_replaces_the_entries_and_preserves_the_surrounding_manifest() -> Result<()> {
    let updated = BTreeSet::from(["anyhow@1.0.104".to_owned(), "serde_json@1.0.151".to_owned()]);

    let rewritten = rewrite(MANIFEST, &updated)?;

    assert!(rewritten.contains("    \"anyhow@1.0.104\","), "{rewritten}");
    assert!(!rewritten.contains("1.0.103"), "the stale pin must be gone: {rewritten}");
    assert!(
      rewritten.contains("# Exact versions approved from the current lockfile."),
      "the comment above the list must survive: {rewritten}"
    );
    assert!(
      rewritten.contains("executables = \"deny\""),
      "content after the list must survive: {rewritten}"
    );
    Ok(())
  }

  /// `--fix` must converge: rewriting an already-correct manifest changes nothing.
  #[test]
  fn rewriting_an_up_to_date_manifest_is_a_no_op() -> Result<()> {
    let pinned = parse_pins(MANIFEST)?;

    let rewritten = rewrite(MANIFEST, &pinned)?;

    assert_eq!(rewritten, MANIFEST);
    Ok(())
  }

  #[test]
  fn the_report_names_the_supersession_and_the_checksum() -> Result<()> {
    let pinned = parse_pins(MANIFEST)?;
    let offenders = parse_diagnostics(&diagnostic("anyhow", "1.0.104"));

    let report = render(&pinned, &offenders);

    assert!(report.contains("anyhow@1.0.103 -> anyhow@1.0.104"), "{report}");
    assert!(report.contains("abc123"), "{report}");
    Ok(())
  }
}
