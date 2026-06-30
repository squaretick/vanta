//! Vanta developer automation (`cargo xtask <task>`). See `docs/03-repository.md`.
//!
//! Tasks:
//! * `keygen <secret-path>` — mint a minisign-compatible registry root keypair.
//! * `registry-gen` — build + sign the official registry (`registry/registry.toml`).
//! * `gen` / `dist` / `bench` — planned scaffolds.

mod registry_gen;

fn main() {
    let mut args = std::env::args().skip(1);
    let task = args.next().unwrap_or_default();
    let result: Result<(), String> = match task.as_str() {
        "keygen" => match args.next() {
            Some(path) => registry_gen::keygen(&path),
            None => Err("usage: cargo xtask keygen <secret-key-path>".to_string()),
        },
        "registry-gen" => registry_gen::run(),
        "gen" | "dist" | "bench" => {
            eprintln!("xtask: `{task}` is not yet implemented (scaffold).");
            Ok(())
        }
        "" => Err("usage: cargo xtask <keygen|registry-gen|gen|dist|bench>".to_string()),
        other => Err(format!("xtask: unknown task `{other}`")),
    };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}
