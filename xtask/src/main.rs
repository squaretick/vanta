//! Vanta developer automation (`cargo xtask <task>`). See `docs/03-repository.md`.
//!
//! Planned tasks: `gen` (error-code/registry codegen), `dist` (release artifacts),
//! `bench` (the performance harness + perf-gate, docs/16/28).

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "gen" | "dist" | "bench" => {
            eprintln!("xtask: `{task}` is not yet implemented (scaffold).");
        }
        "" => eprintln!("usage: cargo xtask <gen|dist|bench>"),
        other => eprintln!("xtask: unknown task `{other}`"),
    }
}
