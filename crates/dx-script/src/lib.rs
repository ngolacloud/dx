//! `dx-script` — single-file Rust script support (cargo `-Zscript` style).
//!
//! A script is a `.rs` file with an optional **frontmatter** manifest fenced by
//! `---` lines (after an optional `#!` shebang):
//!
//! ```text
//! #!/usr/bin/env dx
//! ---
//! [dependencies]
//! serde = "1"
//! ---
//! fn main() { /* ... */ }
//! ```
//!
//! This crate is pure: it splits the frontmatter from the code and synthesizes
//! an **ephemeral** `Cargo.toml` for the script. The CLI (`dx-cli::runcmd`) then
//! resolves that manifest, runs it through the **same security gate** as
//! `dx add`, and only executes it if the gate approves — ephemeral scripts are
//! audited before execution, which is the DX differentiator over `cargo -Zscript`.

use toml_edit::{DocumentMut, Item, Table};

/// A parsed script: the frontmatter TOML (possibly empty) and the Rust code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// The TOML manifest fragment between the `---` fences (empty if none).
    pub manifest: String,
    /// The Rust source, with shebang and frontmatter removed.
    pub code: String,
}

/// Parse a single-file script into its frontmatter manifest and code.
///
/// Rules: an optional leading `#!` shebang line is dropped. If the next
/// non-empty line is a `---` fence (optionally with an infostring like
/// `---cargo`), everything up to the next `---` line is the manifest; the rest
/// is the code. With no fence, the whole file (minus the shebang) is the code.
pub fn parse(src: &str) -> Script {
    let mut lines = src.lines().peekable();

    // Drop a leading shebang.
    let mut consumed_shebang = false;
    if let Some(first) = lines.peek() {
        if first.starts_with("#!") {
            lines.next();
            consumed_shebang = true;
        }
    }

    // Look for an opening fence, skipping blank lines before it.
    let mut pending_blanks: Vec<&str> = Vec::new();
    let mut opened = false;
    while let Some(line) = lines.peek() {
        let t = line.trim_end();
        if t.is_empty() {
            pending_blanks.push(lines.next().unwrap());
            continue;
        }
        if is_fence(t) {
            lines.next();
            opened = true;
        }
        break;
    }

    if !opened {
        // No frontmatter: reconstruct the code (minus the shebang).
        let mut code = String::new();
        for b in &pending_blanks {
            code.push_str(b);
            code.push('\n');
        }
        for line in lines {
            code.push_str(line);
            code.push('\n');
        }
        // If we only consumed a shebang and nothing else changed, keep as-is.
        let _ = consumed_shebang;
        return Script {
            manifest: String::new(),
            code,
        };
    }

    // Capture manifest until the closing fence.
    let mut manifest = String::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if is_fence(line.trim_end()) {
            closed = true;
            break;
        }
        manifest.push_str(line);
        manifest.push('\n');
    }
    let _ = closed; // an unterminated fence just yields an empty-ish code tail

    // The rest is code.
    let mut code = String::new();
    for line in lines {
        code.push_str(line);
        code.push('\n');
    }

    Script { manifest, code }
}

/// A line is a frontmatter fence if, trimmed, it is three or more dashes
/// followed by an optional infostring (e.g. `---`, `----`, `---cargo`).
fn is_fence(line: &str) -> bool {
    let t = line.trim();
    let dashes = t.chars().take_while(|c| *c == '-').count();
    dashes >= 3 && t[dashes..].chars().all(|c| c.is_alphanumeric())
}

/// Sanitize a file stem into a valid crate/package name.
pub fn sanitize_pkg_name(stem: &str) -> String {
    let mut out: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || !out.chars().next().unwrap().is_ascii_alphabetic() {
        out.insert_str(0, "dx_script_");
    }
    out.to_lowercase()
}

/// Build an ephemeral `Cargo.toml` from a script's frontmatter.
///
/// Ensures a `[package]` with `name`/`version`/`edition` (synthesizing what the
/// frontmatter omits) and an empty `[workspace]` table so the ephemeral package
/// is a standalone workspace root — never absorbed by a parent workspace.
/// Returns the rendered manifest, or a parse error string.
pub fn ephemeral_manifest(frontmatter: &str, pkg_name: &str) -> Result<String, String> {
    let mut doc = frontmatter
        .parse::<DocumentMut>()
        .map_err(|e| format!("invalid script frontmatter TOML: {e}"))?;

    if doc.get("package").and_then(Item::as_table_like).is_none() {
        doc["package"] = Item::Table(Table::new());
    }
    let pkg = doc["package"].as_table_mut().expect("package table");
    pkg.set_implicit(false);
    if pkg.get("name").is_none() {
        pkg["name"] = toml_edit::value(pkg_name);
    }
    if pkg.get("version").is_none() {
        pkg["version"] = toml_edit::value("0.0.0");
    }
    if pkg.get("edition").is_none() {
        pkg["edition"] = toml_edit::value("2021");
    }

    // Standalone workspace root: isolate from any enclosing workspace.
    if doc.get("workspace").is_none() {
        doc["workspace"] = Item::Table(Table::new());
    }

    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_code() {
        let src = "#!/usr/bin/env dx\n---\n[dependencies]\nserde = \"1\"\n---\nfn main() {}\n";
        let s = parse(src);
        assert!(s.manifest.contains("[dependencies]"));
        assert!(s.manifest.contains("serde = \"1\""));
        assert_eq!(s.code.trim(), "fn main() {}");
    }

    #[test]
    fn no_frontmatter_is_all_code() {
        let src = "fn main() { println!(\"hi\"); }\n";
        let s = parse(src);
        assert!(s.manifest.is_empty());
        assert_eq!(s.code.trim(), "fn main() { println!(\"hi\"); }");
    }

    #[test]
    fn shebang_without_frontmatter_is_stripped() {
        let src = "#!/usr/bin/env dx\nfn main() {}\n";
        let s = parse(src);
        assert!(s.manifest.is_empty());
        assert_eq!(s.code.trim(), "fn main() {}");
    }

    #[test]
    fn infostring_fence_is_accepted() {
        let src = "---cargo\n[dependencies]\nx = \"1\"\n---\nfn main() {}\n";
        let s = parse(src);
        assert!(s.manifest.contains("x = \"1\""));
        assert_eq!(s.code.trim(), "fn main() {}");
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(
            sanitize_pkg_name("hello-world.rs".trim_end_matches(".rs")),
            "hello_world"
        );
        assert_eq!(sanitize_pkg_name("123abc"), "dx_script_123abc");
        assert_eq!(sanitize_pkg_name(""), "dx_script_");
    }

    #[test]
    fn ephemeral_synthesizes_package_and_workspace() {
        let out = ephemeral_manifest("[dependencies]\nserde = \"1\"\n", "demo").unwrap();
        assert!(out.contains("[package]"));
        assert!(out.contains("name = \"demo\""));
        assert!(out.contains("edition = \"2021\""));
        assert!(out.contains("[workspace]"));
        assert!(out.contains("serde = \"1\""));
    }

    #[test]
    fn ephemeral_preserves_explicit_package() {
        let out = ephemeral_manifest(
            "[package]\nname = \"keep\"\nedition = \"2024\"\n",
            "ignored",
        )
        .unwrap();
        assert!(out.contains("name = \"keep\""));
        assert!(out.contains("edition = \"2024\""));
    }

    #[test]
    fn ephemeral_rejects_bad_toml() {
        assert!(ephemeral_manifest("this is not toml = = =", "x").is_err());
    }
}
