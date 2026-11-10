//! `dx-compat` — the Cargo compatibility contract (ADR-0025).
//!
//! DX composes Cargo (ADR-0002/0020): it never reimplements the resolver or
//! compiler, it **delegates** to a set of stable `cargo`/`rustc` interfaces. The
//! flip side is coupling — a change in `cargo metadata`'s JSON or `cargo tree`'s
//! shape can break DX. This crate pins that contract explicitly: the interfaces
//! DX relies on, the Cargo version range DX is tested against, and a pure
//! classifier for the host version. The CLI (`dx compat`) probes the live host
//! against it, and CI runs the suite across a toolchain matrix.

/// The `cargo metadata` JSON schema version DX parses.
pub const REQUIRED_METADATA_FORMAT: u64 = 1;

/// Minimum supported Rust/Cargo (the workspace MSRV).
pub const MSRV: (u64, u64) = (1, 82);
/// Highest Cargo minor DX has been tested against. Newer is allowed but flagged
/// as "untested" rather than silently trusted.
pub const MAX_TESTED: (u64, u64) = (1, 96);

/// A `major.minor.patch` version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// Parse the leading `x.y.z` out of a `cargo --version`/`rustc --version` line
/// (e.g. `cargo 1.96.0 (abc 2026-05-25)` or `1.82.0-beta.1`).
pub fn parse_version(s: &str) -> Option<Version> {
    // Find the first token that starts with a digit.
    let tok = s.split_whitespace().find(|t| {
        t.chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    })?;
    let mut nums = tok.split('.').map(|part| {
        // Stop at the first non-digit (handles `1-beta`, `0-nightly`).
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u64>().ok()
    });
    let major = nums.next()??;
    let minor = nums.next().flatten().unwrap_or(0);
    let patch = nums.next().flatten().unwrap_or(0);
    Some(Version {
        major,
        minor,
        patch,
    })
}

/// How a host Cargo version relates to the tested range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Below the MSRV — unsupported.
    TooOld,
    /// Within [MSRV, MAX_TESTED] — tested.
    Supported,
    /// Above MAX_TESTED — allowed but not yet tested (flag, don't block).
    NewerUntested,
}

/// Classify a version against the tested range (by `major.minor`).
pub fn classify(v: &Version) -> Support {
    let mm = (v.major, v.minor);
    if mm < MSRV {
        Support::TooOld
    } else if mm > MAX_TESTED {
        Support::NewerUntested
    } else {
        Support::Supported
    }
}

/// One Cargo/rustc interface DX depends on, for the probe report and the ADR.
pub struct Interface {
    /// The command shape DX invokes.
    pub probe: &'static str,
    /// Why DX needs it.
    pub purpose: &'static str,
    /// `true` if DX cannot function without it (a break here is fatal).
    pub required: bool,
}

/// The full contract. Keep in sync with the ADR and the actual call sites.
pub const INTERFACES: &[Interface] = &[
    Interface {
        probe: "cargo metadata --format-version 1",
        purpose: "graph/package data for plan, release topo order, Seiva adapter",
        required: true,
    },
    Interface {
        probe: "cargo tree -i / -d / -e",
        purpose: "why/explain and the inverse-dependency path in audit findings",
        required: true,
    },
    Interface {
        probe: "cargo generate-lockfile",
        purpose: "resolve the candidate graph in staging without compiling",
        required: true,
    },
    Interface {
        probe: "cargo update",
        purpose: "compute the minimal updated graph for `dx update`",
        required: true,
    },
    Interface {
        probe: "cargo vendor",
        purpose: "offline verifiable bundle (`dx bundle`)",
        required: false,
    },
    Interface {
        probe: "rustc -vV",
        purpose: "toolchain provenance (`dx toolchain`, `dx environment`)",
        required: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cargo_version_line() {
        let v = parse_version("cargo 1.96.0 (abc123 2026-05-25)").unwrap();
        assert_eq!(
            v,
            Version {
                major: 1,
                minor: 96,
                patch: 0
            }
        );
    }

    #[test]
    fn parses_bare_and_prerelease() {
        assert_eq!(parse_version("1.82.0").unwrap().minor, 82);
        let beta = parse_version("rustc 1.83.0-beta.1 (xyz)").unwrap();
        assert_eq!((beta.major, beta.minor, beta.patch), (1, 83, 0));
    }

    #[test]
    fn rejects_non_version() {
        assert!(parse_version("cargo unknown").is_none());
    }

    #[test]
    fn classify_boundaries() {
        assert_eq!(
            classify(&Version {
                major: 1,
                minor: 81,
                patch: 9
            }),
            Support::TooOld
        );
        assert_eq!(
            classify(&Version {
                major: 1,
                minor: 82,
                patch: 0
            }),
            Support::Supported
        );
        assert_eq!(
            classify(&Version {
                major: 1,
                minor: 96,
                patch: 0
            }),
            Support::Supported
        );
        assert_eq!(
            classify(&Version {
                major: 1,
                minor: 99,
                patch: 0
            }),
            Support::NewerUntested
        );
    }

    #[test]
    fn version_ordering() {
        assert!(
            Version {
                major: 1,
                minor: 82,
                patch: 0
            } < Version {
                major: 1,
                minor: 96,
                patch: 0
            }
        );
    }

    #[test]
    fn contract_lists_the_required_interfaces() {
        assert!(INTERFACES
            .iter()
            .any(|i| i.probe.contains("cargo metadata") && i.required));
        assert!(INTERFACES.iter().filter(|i| i.required).count() >= 4);
    }
}
