//! Manifest parsing for `.current.toml` files.
//!
//! Contract: a manifest names a bounded list of checks. Each check is an
//! argv vector (never a shell string), a workdir relative to the repo
//! root, an optional timeout in seconds, and optional environment
//! entries. Unknown fields are rejected so a typo cannot silently
//! change what runs. All sizes are validated before any process spawns.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Maximum manifest file size, in bytes.
pub const MANIFEST_BYTES_MAX: usize = 65_536;
/// Maximum number of checks per manifest.
pub const CHECKS_MAX: usize = 32;
/// Maximum number of argv entries per check.
pub const ARGV_ENTRIES_MAX: usize = 64;
/// Maximum byte length of one argv entry.
pub const ARG_BYTES_MAX: usize = 4_096;
/// Default per-check timeout, in seconds.
pub const TIMEOUT_SECS_DEFAULT: u64 = 300;
/// Maximum per-check timeout, in seconds (24 hours).
pub const TIMEOUT_SECS_MAX: u64 = 86_400;

/// A validated `.current.toml` manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Optional human-readable repo name; informational only.
    pub repo: Option<String>,
    /// The checks to run, in manifest order.
    pub checks: Vec<Check>,
}

/// One named check: an argv vector plus its execution bounds.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// Unique check name: ASCII alphanumerics, `-`, and `_`.
    pub name: String,
    /// Command and arguments, executed directly with no shell.
    pub argv: Vec<String>,
    /// Working directory relative to the repo root. Default: `"."`.
    #[serde(default = "default_workdir")]
    pub workdir: String,
    /// Hard timeout in seconds. Default: [`TIMEOUT_SECS_DEFAULT`].
    pub timeout_secs: Option<u64>,
    /// Extra environment entries for this check only.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

fn default_workdir() -> String {
    ".".to_owned()
}

impl Check {
    /// The effective timeout for this check, in seconds.
    pub fn timeout_secs_effective(&self) -> u64 {
        self.timeout_secs.unwrap_or(TIMEOUT_SECS_DEFAULT)
    }
}

/// Why a manifest was rejected. Messages carry bounded context only:
/// never the full manifest text.
#[derive(Debug)]
pub enum ManifestError {
    /// The manifest file could not be read.
    Io(std::io::Error),
    /// The manifest exceeded [`MANIFEST_BYTES_MAX`].
    TooLarge {
        /// Observed size in bytes.
        size_bytes: usize,
    },
    /// The TOML did not parse, or a field had the wrong type.
    Parse(String),
    /// A parsed manifest violated a validation rule.
    Validation(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "cannot read manifest: {error}"),
            Self::TooLarge { size_bytes } => write!(
                f,
                "manifest is {size_bytes} bytes; limit is {MANIFEST_BYTES_MAX}"
            ),
            Self::Parse(message) => write!(f, "manifest parse error: {message}"),
            Self::Validation(message) => write!(f, "manifest invalid: {message}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Load and validate the manifest at `path`.
///
/// # Errors
/// Returns [`ManifestError`] when the file is unreadable, oversized,
/// malformed TOML, or fails validation (see [`Manifest::validate`]).
pub fn load_manifest(path: &Path) -> Result<Manifest, ManifestError> {
    let metadata = std::fs::metadata(path).map_err(ManifestError::Io)?;
    let size_bytes = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if size_bytes > MANIFEST_BYTES_MAX {
        return Err(ManifestError::TooLarge { size_bytes });
    }
    let text = std::fs::read_to_string(path).map_err(ManifestError::Io)?;
    parse_manifest(&text)
}

/// Parse and validate manifest text.
///
/// # Errors
/// Returns [`ManifestError::Parse`] for malformed TOML and
/// [`ManifestError::Validation`] for contract violations.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    if text.len() > MANIFEST_BYTES_MAX {
        return Err(ManifestError::TooLarge {
            size_bytes: text.len(),
        });
    }
    let manifest: Manifest =
        toml::from_str(text).map_err(|error| ManifestError::Parse(error.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

impl Manifest {
    /// Validate every bound a correct manifest must satisfy.
    ///
    /// # Errors
    /// Returns [`ManifestError::Validation`] naming the first violated
    /// rule. Nothing is mutated on failure.
    pub fn validate(&self) -> Result<(), ManifestError> {
        let invalid = |message: &str| Err(ManifestError::Validation(message.to_owned()));
        if self.checks.is_empty() {
            return invalid("manifest declares no checks");
        }
        if self.checks.len() > CHECKS_MAX {
            return invalid("manifest declares too many checks");
        }
        for (index, check) in self.checks.iter().enumerate() {
            if self.checks[..index]
                .iter()
                .any(|prior| prior.name == check.name)
            {
                return invalid("duplicate check name");
            }
            check.validate()?;
        }
        Ok(())
    }
}

impl Check {
    fn validate(&self) -> Result<(), ManifestError> {
        let invalid = |message: &str| Err(ManifestError::Validation(message.to_owned()));
        if self.name.is_empty()
            || !self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return invalid("check name must be non-empty [A-Za-z0-9_-]");
        }
        if self.argv.is_empty() || self.argv.len() > ARGV_ENTRIES_MAX {
            return invalid("argv must hold 1..=64 entries");
        }
        if self
            .argv
            .iter()
            .any(|arg| arg.is_empty() || arg.len() > ARG_BYTES_MAX)
        {
            return invalid("argv entries must be non-empty and <= 4096 bytes");
        }
        let timeout_secs = self.timeout_secs_effective();
        if timeout_secs == 0 || timeout_secs > TIMEOUT_SECS_MAX {
            return invalid("timeout_secs must be within 1..=86400");
        }
        if self.workdir.is_empty() || Path::new(&self.workdir).is_absolute() {
            return invalid("workdir must be a non-empty relative path");
        }
        for (key, value) in &self.env {
            let key_ok = !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            if !key_ok || value.len() > ARG_BYTES_MAX {
                return invalid("env keys must be [A-Z0-9_] and values <= 4096 bytes");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{MANIFEST_BYTES_MAX, parse_manifest};

    const ONE_CHECK: &str = r#"
[[checks]]
name = "hello"
argv = ["echo", "hi"]
"#;

    #[test]
    fn parses_minimal_manifest_with_defaults() {
        let manifest = parse_manifest(ONE_CHECK).expect("valid manifest");
        assert_eq!(manifest.checks.len(), 1);
        assert_eq!(manifest.checks[0].workdir, ".");
        assert_eq!(manifest.checks[0].timeout_secs_effective(), 300);
    }

    #[test]
    fn parses_env_and_timeout() {
        let text = r#"
[[checks]]
name = "gated"
argv = ["make", "check"]
timeout_secs = 60
env = { MODE = "strict" }
"#;
        let manifest = parse_manifest(text).expect("valid manifest");
        assert_eq!(manifest.checks[0].timeout_secs, Some(60));
        assert_eq!(manifest.checks[0].env["MODE"], "strict");
    }

    #[test]
    fn rejects_malformed_toml() {
        assert!(parse_manifest("[[checks]\nname = ").is_err());
    }

    #[test]
    fn rejects_empty_argv() {
        let text = "[[checks]]\nname = \"x\"\nargv = []\n";
        assert!(parse_manifest(text).is_err());
    }

    #[test]
    fn rejects_zero_timeout() {
        let text = "[[checks]]\nname = \"x\"\nargv = [\"true\"]\ntimeout_secs = 0\n";
        assert!(parse_manifest(text).is_err());
    }

    #[test]
    fn rejects_duplicate_check_names() {
        let text = "[[checks]]\nname = \"x\"\nargv = [\"true\"]\n\
                    [[checks]]\nname = \"x\"\nargv = [\"false\"]\n";
        assert!(parse_manifest(text).is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        let text = "[[checks]]\nname = \"x\"\nargv = [\"true\"]\nshell = \"bash\"\n";
        assert!(parse_manifest(text).is_err());
    }

    #[test]
    fn rejects_oversized_manifest() {
        let text = "x".repeat(MANIFEST_BYTES_MAX + 1);
        assert!(parse_manifest(&text).is_err());
    }

    #[test]
    fn rejects_absolute_workdir() {
        let text = "[[checks]]\nname = \"x\"\nargv = [\"true\"]\nworkdir = \"/tmp\"\n";
        assert!(parse_manifest(text).is_err());
    }
}
