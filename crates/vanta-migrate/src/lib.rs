//! `vanta-migrate` — import foreign version files into a Vanta `[tools]` set.
//!
//! Reads the common version files (`.mise.toml`/`mise.toml`, `.tool-versions`,
//! `.nvmrc`, `.python-version`, `rust-toolchain.toml`, …) and produces
//! tool→version pairs plus the source they
//! came from, which the CLI renders into a `vanta.toml`. See `docs/30-migration.md`.
//! Pure-std parsing; no foreign tool is invoked.
#![forbid(unsafe_code)]

use std::path::Path;

/// One imported tool pin and the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub tool: String,
    pub version: String,
    pub source: String,
}

/// Translate a foreign tool name to Vanta's canonical name.
fn canonical_name(name: &str) -> String {
    match name {
        "nodejs" => "node",
        "golang" => "go",
        other => other,
    }
    .to_string()
}

fn clean_version(v: &str) -> String {
    v.trim().trim_start_matches('v').to_string()
}

/// Parse an asdf/mise `.tool-versions` body ("name version" per line).
pub fn parse_tool_versions(body: &str, source: &str) -> Vec<Imported> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let (Some(name), Some(version)) = (parts.next(), parts.next()) {
            out.push(Imported {
                tool: canonical_name(name),
                version: clean_version(version),
                source: source.to_string(),
            });
        }
    }
    out
}

fn single(tool: &str, body: &str, source: &str) -> Option<Imported> {
    let version = clean_version(body);
    let version = version.lines().next().unwrap_or("").trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(Imported {
            tool: tool.to_string(),
            version,
            source: source.to_string(),
        })
    }
}

/// Extract a version string from a mise `[tools]` value. Handles quoted strings
/// (`"3.3.0"`), arrays (`["3.12", "3.11"]` → first), inline tables
/// (`{ version = "3.3.0" }`), and bare tokens (`20`). Returns `None` for values
/// with no usable version (e.g. `"system"`, an empty array).
fn mise_value_to_version(raw: &str) -> Option<String> {
    let v = raw.trim();
    let unquote = |s: &str| -> String {
        let s = s.trim();
        s.trim_matches(|c| c == '"' || c == '\'').to_string()
    };
    let out = if let Some(rest) = v.strip_prefix('[') {
        // Array: take the first element.
        let inner = rest.split(']').next().unwrap_or("");
        let first = inner.split(',').next().unwrap_or("").trim();
        if first.is_empty() {
            return None;
        }
        unquote(first)
    } else if v.starts_with('{') {
        // Inline table: find `version = "…"`.
        let ver = v.split(',').find_map(|part| {
            let part = part.trim_matches(|c| c == '{' || c == '}').trim();
            let (k, val) = part.split_once('=')?;
            if k.trim() == "version" {
                Some(unquote(val))
            } else {
                None
            }
        })?;
        ver
    } else if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
        // Quoted scalar: take the content up to the closing quote (ignoring any
        // trailing inline comment).
        let rest = &v[1..];
        match rest.find(q) {
            Some(i) => rest[..i].to_string(),
            None => rest.to_string(),
        }
    } else {
        // Bare scalar: strip an inline comment.
        v.split('#').next().unwrap_or(v).trim().to_string()
    };
    let out = clean_version(&out);
    if out.is_empty() || out == "system" || out == "latest" {
        None
    } else {
        Some(out)
    }
}

/// Parse a mise / rtx config (`.mise.toml`, `mise.toml`, `.config/mise/config.toml`)
/// `[tools]` table. Supports the flat form (`node = "20"`, `python = ["3.12"]`,
/// `ruby = { version = "3.3" }`) and the `[tools.<name>]` sub-table form
/// (`version = "20"`). Pure-std line parsing — only the `[tools]` section is read.
pub fn parse_mise_toml(body: &str, source: &str) -> Vec<Imported> {
    #[derive(PartialEq)]
    enum Sec {
        Other,
        Tools,
        ToolNamed(String),
    }
    let mut sec = Sec::Other;
    let mut out = Vec::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            // Ignore array-of-tables double brackets by trimming a stray '['/']'.
            let header = header.trim().trim_matches(|c| c == '[' || c == ']').trim();
            sec = if header == "tools" {
                Sec::Tools
            } else if let Some(name) = header.strip_prefix("tools.") {
                Sec::ToolNamed(name.trim().trim_matches('"').to_string())
            } else {
                Sec::Other
            };
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        match &sec {
            Sec::Tools => {
                let name = key.trim().trim_matches('"');
                if let Some(version) = mise_value_to_version(val) {
                    out.push(Imported {
                        tool: canonical_name(name),
                        version,
                        source: source.to_string(),
                    });
                }
            }
            Sec::ToolNamed(name) => {
                if key.trim() == "version" {
                    if let Some(version) = mise_value_to_version(val) {
                        out.push(Imported {
                            tool: canonical_name(name),
                            version,
                            source: source.to_string(),
                        });
                    }
                }
            }
            Sec::Other => {}
        }
    }
    out
}

fn parse_rust_toolchain(body: &str, source: &str) -> Option<Imported> {
    // Minimal: find `channel = "<x>"`; default to "stable".
    let channel = body
        .lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("channel")
                .and_then(|r| r.split('=').nth(1))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "stable".to_string());
    Some(Imported {
        tool: "rust".to_string(),
        version: channel,
        source: source.to_string(),
    })
}

/// Scan a directory for known version files and import them. The first file to
/// mention a tool wins (specific files are scanned before broad ones).
pub fn import_dir(dir: &Path) -> Vec<Imported> {
    let mut found: Vec<Imported> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();

    let push =
        |imp: Imported, acc: &mut Vec<Imported>, seen: &mut std::collections::BTreeSet<String>| {
            if seen.insert(imp.tool.clone()) {
                acc.push(imp);
            }
        };

    // Specific single-tool files first (they express intent most precisely).
    if let Some(b) = read(".nvmrc").or_else(|| read(".node-version")) {
        if let Some(i) = single("node", &b, ".nvmrc") {
            push(i, &mut found, &mut seen);
        }
    }
    if let Some(b) = read(".python-version") {
        if let Some(i) = single("python", &b, ".python-version") {
            push(i, &mut found, &mut seen);
        }
    }
    if let Some(b) = read(".ruby-version") {
        if let Some(i) = single("ruby", &b, ".ruby-version") {
            push(i, &mut found, &mut seen);
        }
    }
    if let Some(b) = read(".go-version") {
        if let Some(i) = single("go", &b, ".go-version") {
            push(i, &mut found, &mut seen);
        }
    }
    if let Some(b) = read("rust-toolchain.toml") {
        if let Some(i) = parse_rust_toolchain(&b, "rust-toolchain.toml") {
            push(i, &mut found, &mut seen);
        }
    }
    // Broad multi-tool files last. mise/rtx config before the legacy
    // asdf-style .tool-versions.
    for (name, src) in [
        (".mise.toml", ".mise.toml"),
        ("mise.toml", "mise.toml"),
        (".config/mise/config.toml", ".config/mise/config.toml"),
        (".rtx.toml", ".rtx.toml"),
    ] {
        if let Some(b) = read(name) {
            for i in parse_mise_toml(&b, src) {
                push(i, &mut found, &mut seen);
            }
        }
    }
    if let Some(b) = read(".tool-versions") {
        for i in parse_tool_versions(&b, ".tool-versions") {
            push(i, &mut found, &mut seen);
        }
    }

    found
}

/// Render imported tools into a `vanta.toml` `[tools]` table.
pub fn to_manifest_toml(tools: &[Imported]) -> String {
    let mut s = String::from("[tools]\n");
    for t in tools {
        s.push_str(&format!("{} = \"{}\"\n", t.tool, t.version));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_versions_parsing_and_aliases() {
        let body = "# comment\nnodejs 20.11.0\npython 3.12.2\n\ngolang 1.23.0\n";
        let imported = parse_tool_versions(body, ".tool-versions");
        assert_eq!(imported.len(), 3);
        assert_eq!(imported[0].tool, "node"); // nodejs → node
        assert_eq!(imported[0].version, "20.11.0");
        assert_eq!(imported[2].tool, "go"); // golang → go
    }

    #[test]
    fn single_file_strips_v_prefix() {
        let i = single("node", "v20.11.0\n", ".nvmrc").unwrap();
        assert_eq!(i.version, "20.11.0");
    }

    #[test]
    fn rust_toolchain_channel() {
        let i = parse_rust_toolchain("[toolchain]\nchannel = \"1.79.0\"\n", "rust-toolchain.toml")
            .unwrap();
        assert_eq!(i.tool, "rust");
        assert_eq!(i.version, "1.79.0");
    }

    #[test]
    fn mise_toml_flat_array_inline_and_subtable() {
        let body = "\
[env]
FOO = \"bar\"

[tools]
node = \"20.11.0\"
python = [\"3.12\", \"3.11\"]
ruby = { version = \"3.3.0\" }
go = \"system\"           # should be skipped

[tools.terraform]
version = \"1.9.0\"

[tasks.build]
run = \"make\"
";
        let imported = parse_mise_toml(body, ".mise.toml");
        let get = |t: &str| imported.iter().find(|i| i.tool == t).map(|i| i.version.as_str());
        assert_eq!(get("node"), Some("20.11.0"));
        assert_eq!(get("python"), Some("3.12")); // first of the array
        assert_eq!(get("ruby"), Some("3.3.0")); // inline table
        assert_eq!(get("terraform"), Some("1.9.0")); // [tools.<name>] sub-table
        assert_eq!(get("go"), None); // "system" ignored
        // Keys outside [tools] must not leak in.
        assert!(imported.iter().all(|i| i.tool != "FOO" && i.tool != "build"));
    }

    #[test]
    fn renders_manifest() {
        let tools = vec![Imported {
            tool: "node".into(),
            version: "24".into(),
            source: "x".into(),
        }];
        assert_eq!(to_manifest_toml(&tools), "[tools]\nnode = \"24\"\n");
    }
}
