//! `vanta-cli` — the command surface behind the `vanta` binary.
//!
//! Implements the commands documented in `docs/04-cli.md`, wiring the CLI to the
//! resolver, registry, install engine, environment, and diagnostics subsystems.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use vanta_core::{Area, ExitCode, Platform, Request, StoreKey, VersionReq, VtaError, VtaResult};
use vanta_install::Engine;
use vanta_registry::Registry;
use vanta_resolve::{artifact_for, Resolver};

/// The crate version, surfaced by `vanta --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Dispatch a parsed argv (without the program name). Returns the process exit code.
pub fn run(args: &[String]) -> VtaResult<ExitCode> {
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest: &[String] = args.get(1..).unwrap_or(&[]);
    match cmd {
        "--version" | "-V" | "version" => {
            println!("vanta {VERSION}");
            Ok(ExitCode::Ok)
        }
        "--help" | "-h" | "help" => {
            print_help();
            Ok(ExitCode::Ok)
        }
        "add" => cmd_add(rest),
        "search" => cmd_search(rest),
        "info" => cmd_info(rest),
        "activate" => cmd_activate(rest),
        "list" | "ls" => cmd_list(),
        "which" => cmd_which(rest),
        "doctor" => cmd_doctor(),
        "sync" => cmd_sync(),
        "generations" | "gen" => cmd_generations(),
        "rollback" => cmd_rollback(rest),
        "gc" => cmd_gc(),
        "init" | "migrate" => cmd_import(has_flag(rest, "--force") || has_flag(rest, "-f")),
        "exec" => cmd_exec(rest),
        "x" => cmd_x(rest),
        "remove" | "rm" => cmd_remove(rest),
        "run" => cmd_run(rest),
        "bundle" => cmd_bundle(rest),
        "restore" => cmd_restore(rest),
        "use" => cmd_add(rest),
        "update" | "up" => cmd_sync(),
        "outdated" => cmd_outdated(),
        "cache" => cmd_cache(rest),
        "config" => cmd_config(),
        "completions" => cmd_completions(rest),
        "trust" => cmd_trust(rest),
        "registry" => cmd_registry(rest),
        "shell" => cmd_shell(rest),
        "self" => cmd_self(rest),
        other => {
            eprintln!("vanta: unknown command `{other}` (try `vanta help`)");
            Ok(ExitCode::Usage)
        }
    }
}

/// `vanta add <tool>[@version] ...` — resolve and install each tool.
fn cmd_add(rest: &[String]) -> VtaResult<ExitCode> {
    let tools: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
    if tools.is_empty() {
        eprintln!("usage: vanta add <tool>[@version] ...");
        return Ok(ExitCode::Usage);
    }

    let registry = load_registry()?;
    let resolver = Resolver::new(&registry);
    let platform = Platform::current();

    // Resolve everything first (fail fast, no side effects on disk).
    let mut resolutions = Vec::new();
    for tool in &tools {
        let request = Request::parse(tool)?;
        resolutions.push(resolver.resolve(&request, &[platform])?);
    }

    // Install.
    let engine = Engine::open(home()?)?;
    for resolution in &resolutions {
        let artifact = artifact_for(resolution, &platform).ok_or_else(|| {
            VtaError::new(
                Area::Res,
                5,
                format!(
                    "no artifact for `{}` on {}",
                    resolution.tool,
                    platform.token()
                ),
            )
        })?;
        println!("installing {} {}", resolution.tool, resolution.version);
        let key = engine.install_artifact(&resolution.tool, &resolution.version, artifact)?;
        println!("  ✓ {} {} → {}", resolution.tool, resolution.version, key);
    }
    Ok(ExitCode::Ok)
}

/// `vanta search <query>` — search the registry.
fn cmd_search(rest: &[String]) -> VtaResult<ExitCode> {
    let query = rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_default();
    let registry = load_registry()?;
    for name in registry.search(&query) {
        println!("{name}");
    }
    Ok(ExitCode::Ok)
}

/// `vanta info <tool>` — show a tool's provider and available versions.
fn cmd_info(rest: &[String]) -> VtaResult<ExitCode> {
    let name = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(n) => n,
        None => {
            eprintln!("usage: vanta info <tool>");
            return Ok(ExitCode::Usage);
        }
    };
    let registry = load_registry()?;
    let entry = registry
        .tool(name)
        .ok_or_else(|| VtaError::new(Area::Res, 3, format!("unknown tool `{name}`")))?;
    println!("{name}  (provider: {})", entry.provider.id);
    if let Some(summary) = &entry.summary {
        println!("  {summary}");
    }
    println!("  versions:");
    for v in &entry.versions {
        let chan = v.channel.as_deref().unwrap_or("");
        println!("    {} {}", v.version, chan);
    }
    Ok(ExitCode::Ok)
}

/// `vanta activate <shell>` — print the shell hook for `eval`.
fn cmd_activate(rest: &[String]) -> VtaResult<ExitCode> {
    let shell = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(s) => s,
        None => {
            eprintln!("usage: vanta activate <bash|zsh|fish|pwsh>");
            return Ok(ExitCode::Usage);
        }
    };
    match vanta_env::activate_hook(shell) {
        Some(hook) => {
            print!("{hook}");
            Ok(ExitCode::Ok)
        }
        None => {
            eprintln!("vanta: unsupported shell `{shell}`");
            Ok(ExitCode::Usage)
        }
    }
}

/// `vanta list` — show the tools in the active generation.
fn cmd_list() -> VtaResult<ExitCode> {
    let engine = Engine::open(home()?)?;
    match engine.state().current()? {
        Some(id) => match engine.state().get_generation(id)? {
            Some(gen) if !gen.tools.is_empty() => {
                for (tool, key) in &gen.tools {
                    println!("{tool}  ({key})");
                }
            }
            _ => println!("(no tools installed)"),
        },
        None => println!("(no tools installed)"),
    }
    Ok(ExitCode::Ok)
}

/// `vanta which <tool>` — print the store path of the active tool.
fn cmd_which(rest: &[String]) -> VtaResult<ExitCode> {
    let name = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(n) => n,
        None => {
            eprintln!("usage: vanta which <tool>");
            return Ok(ExitCode::Usage);
        }
    };
    let engine = Engine::open(home()?)?;
    let id = engine
        .state()
        .current()?
        .ok_or_else(|| VtaError::new(Area::Env, 2, format!("`{name}` is not active")))?;
    let gen = engine
        .state()
        .get_generation(id)?
        .ok_or_else(|| VtaError::new(Area::Env, 2, format!("`{name}` is not active")))?;
    let (_, key) = gen
        .tools
        .iter()
        .find(|(t, _)| t == name)
        .ok_or_else(|| VtaError::new(Area::Env, 2, format!("`{name}` is not active")))?;
    let store_key = StoreKey::new(key.clone())?;
    println!("{}", engine.store().entry_path(&store_key).display());
    Ok(ExitCode::Ok)
}

/// `vanta generations` — list the generation history (`*` marks the active one).
fn cmd_generations() -> VtaResult<ExitCode> {
    let engine = Engine::open(home()?)?;
    match engine.state().current()? {
        None => println!("(no generations)"),
        Some(current) => {
            for id in 1..=current {
                if let Some(gen) = engine.state().get_generation(id)? {
                    let mark = if id == current { "*" } else { " " };
                    println!("{mark} {id:04}  {}  [{}]", gen.command, gen.reason);
                }
            }
        }
    }
    Ok(ExitCode::Ok)
}

/// `vanta rollback [gen]` — switch the active generation (defaults to the previous).
fn cmd_rollback(rest: &[String]) -> VtaResult<ExitCode> {
    let engine = Engine::open(home()?)?;
    let current = engine
        .state()
        .current()?
        .ok_or_else(|| VtaError::new(Area::Env, 2, "no generations to roll back".to_string()))?;
    let target = match rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .and_then(|s| s.parse::<u64>().ok())
    {
        Some(n) => n,
        None if current > 1 => current - 1,
        None => {
            return Err(VtaError::new(
                Area::Env,
                2,
                "already at the earliest generation".to_string(),
            ))
        }
    };
    if engine.state().get_generation(target)?.is_none() {
        return Err(VtaError::new(
            Area::Env,
            2,
            format!("generation {target} not found"),
        ));
    }
    engine.state().set_current(target)?;
    println!("rolled back to generation {target:04}");
    Ok(ExitCode::Ok)
}

/// `vanta gc` — remove store entries unreachable from the retained generations
/// (the active one plus the previous few, per the retention policy).
fn cmd_gc() -> VtaResult<ExitCode> {
    const RETAIN: u64 = 5;
    let engine = Engine::open(home()?)?;
    let mut roots: HashSet<StoreKey> = HashSet::new();
    if let Some(current) = engine.state().current()? {
        let start = current.saturating_sub(RETAIN - 1).max(1);
        for id in start..=current {
            if let Some(gen) = engine.state().get_generation(id)? {
                for (_, key) in &gen.tools {
                    if let Ok(k) = StoreKey::new(key.clone()) {
                        roots.insert(k);
                    }
                }
            }
        }
    }
    let removed = engine.store().gc(&roots)?;
    println!(
        "removed {removed} unreferenced store entr{}",
        if removed == 1 { "y" } else { "ies" }
    );
    Ok(ExitCode::Ok)
}

/// `vanta doctor` — run health checks and print fixes.
fn cmd_doctor() -> VtaResult<ExitCode> {
    let home = home()?;
    let checks = vanta_diag::run(&home);
    for c in &checks {
        let mark = if c.ok { "✓" } else { "✗" };
        println!("{mark} {} — {}", c.name, c.detail);
    }
    Ok(if vanta_diag::all_ok(&checks) {
        ExitCode::Ok
    } else {
        ExitCode::Failure
    })
}

/// `vanta sync` — reconcile to the nearest `vanta.toml`: install each tool for the
/// current platform and write a **cross-platform** `vanta.lock` pinning every
/// declared target the registry can serve (`docs/11-reproducibility.md`).
fn cmd_sync() -> VtaResult<ExitCode> {
    let manifest_path = find_manifest()?;
    let manifest = vanta_config::load_file(&manifest_path)?;
    if manifest.tools.is_empty() {
        println!(
            "nothing to sync ({} has no [tools])",
            manifest_path.display()
        );
        return Ok(ExitCode::Ok);
    }

    // Target platforms: the manifest's `[settings] targets`, else a default set;
    // always include the current platform so this machine can install.
    let current = Platform::current();
    let mut platforms: Vec<Platform> = manifest
        .settings
        .targets
        .clone()
        .unwrap_or_else(default_targets)
        .iter()
        .filter_map(|t| Platform::parse(t).ok())
        .collect();
    if !platforms.contains(&current) {
        platforms.push(current);
    }

    let registry = load_registry()?;
    let resolver = Resolver::new(&registry);
    let engine = Engine::open(home()?)?;
    let mut lock = vanta_lock::Lock::new(
        format!("vanta {VERSION}"),
        platforms.iter().map(|p| p.token()).collect(),
    );

    for (tool, spec) in &manifest.tools {
        let request_str = spec.version().to_string();
        let request = Request {
            tool: tool.clone(),
            version: VersionReq::parse(&request_str),
        };
        let resolution = resolver.resolve(&request, &platforms)?;

        // Install only the current platform; lock pins all resolved platforms.
        let current_artifact = artifact_for(&resolution, &current).ok_or_else(|| {
            VtaError::new(
                Area::Res,
                5,
                format!("no artifact for `{tool}` on {}", current.token()),
            )
        })?;
        println!("syncing {} {}", resolution.tool, resolution.version);
        let key =
            engine.install_artifact(&resolution.tool, &resolution.version, current_artifact)?;

        let mut platform_map = BTreeMap::new();
        for (plat, art) in &resolution.per_platform {
            // Materialized only for the current platform; others pin url+hash and
            // get a store key when that platform later syncs.
            let store_key = if *plat == current {
                key.as_str().to_string()
            } else {
                String::new()
            };
            platform_map.insert(
                plat.token(),
                vanta_lock::PlatformPin {
                    store_key,
                    url: art.url.clone(),
                    size: art.size,
                    sha256: art.checksum.value.clone(),
                    blake3: None,
                    signature: art.signature.clone(),
                    bin: art.bin.clone(),
                },
            );
        }
        lock.tools.push(vanta_lock::LockedTool {
            name: tool.clone(),
            request: request_str,
            version: resolution.version.clone(),
            provider: resolution.provider.clone(),
            platform: platform_map,
        });
    }

    let lock_path = manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("vanta.lock");
    lock.write_file(&lock_path)?;
    println!(
        "✓ wrote {} ({} targets)",
        lock_path.display(),
        platforms.len()
    );
    Ok(ExitCode::Ok)
}

/// The default lock target set when a manifest declares none.
fn default_targets() -> Vec<String> {
    [
        "macos/aarch64",
        "macos/x86_64",
        "linux/x86_64/gnu",
        "linux/aarch64/gnu",
        "windows/x86_64",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Find the nearest `vanta.toml`, walking up from the current directory.
fn find_manifest() -> VtaResult<PathBuf> {
    let mut dir = std::env::current_dir()
        .map_err(|e| VtaError::new(Area::Cfg, 1, format!("cannot read current directory: {e}")))?;
    loop {
        let candidate = dir.join("vanta.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            return Err(VtaError::new(
                Area::Cfg,
                1,
                "no vanta.toml found in this directory or any parent".to_string(),
            ));
        }
    }
}

/// `vanta exec -- <cmd>` — run a command with `~/.vanta/bin` on PATH.
fn cmd_exec(rest: &[String]) -> VtaResult<ExitCode> {
    let cmdv: &[String] = match rest.iter().position(|a| a == "--") {
        Some(i) => &rest[i + 1..],
        None => rest,
    };
    if cmdv.is_empty() {
        eprintln!("usage: vanta exec -- <command> [args]");
        return Ok(ExitCode::Usage);
    }
    let status = Command::new(&cmdv[0])
        .args(&cmdv[1..])
        .env("PATH", env_path_with_bin()?)
        .status()
        .map_err(|e| VtaError::new(Area::Env, 1, format!("running {}: {e}", cmdv[0])))?;
    Ok(status_exit(status))
}

/// `vanta x <tool>[@ver] [args]` — resolve+install if needed, then run it.
fn cmd_x(rest: &[String]) -> VtaResult<ExitCode> {
    let spec = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(s) => s.clone(),
        None => {
            eprintln!("usage: vanta x <tool>[@version] [args]");
            return Ok(ExitCode::Usage);
        }
    };
    let request = Request::parse(&spec)?;
    let registry = load_registry()?;
    let resolver = Resolver::new(&registry);
    let platform = Platform::current();
    let resolution = resolver.resolve(&request, &[platform])?;
    let engine = Engine::open(home()?)?;
    let artifact = artifact_for(&resolution, &platform).ok_or_else(|| {
        VtaError::new(
            Area::Res,
            5,
            format!("no artifact for `{}`", resolution.tool),
        )
    })?;
    engine.install_artifact(&resolution.tool, &resolution.version, artifact)?;

    let idx = rest.iter().position(|a| a == &spec).unwrap_or(0);
    let args: &[String] = rest.get(idx + 1..).unwrap_or(&[]);
    let tool_bin = home()?.join("bin").join(&resolution.tool);
    let status = Command::new(&tool_bin)
        .args(args)
        .env("PATH", env_path_with_bin()?)
        .status()
        .map_err(|e| VtaError::new(Area::Env, 1, format!("running {}: {e}", resolution.tool)))?;
    Ok(status_exit(status))
}

/// `vanta remove <tool>` — drop a tool (new generation) and unlink it.
fn cmd_remove(rest: &[String]) -> VtaResult<ExitCode> {
    let tool = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(t) => t,
        None => {
            eprintln!("usage: vanta remove <tool>");
            return Ok(ExitCode::Usage);
        }
    };
    let engine = Engine::open(home()?)?;
    if engine.remove(tool)? {
        println!("removed {tool}");
        Ok(ExitCode::Ok)
    } else {
        Err(VtaError::new(
            Area::Env,
            2,
            format!("`{tool}` is not installed"),
        ))
    }
}

/// `vanta run <task|tool> [args]` — run a manifest task, else a tool binary.
fn cmd_run(rest: &[String]) -> VtaResult<ExitCode> {
    let name = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(n) => n.clone(),
        None => {
            eprintln!("usage: vanta run <task|tool> [args]");
            return Ok(ExitCode::Usage);
        }
    };
    // A defined task wins over a tool of the same name.
    if let Ok(manifest_path) = find_manifest() {
        if let Ok(manifest) = vanta_config::load_file(&manifest_path) {
            if let Some(task) = manifest.tasks.get(&name) {
                let cmd = match task {
                    vanta_config::model::Task::Command(s) => s.clone(),
                    vanta_config::model::Task::Detailed(d) => d.run.clone(),
                };
                let status = shell_command(&cmd)
                    .env("PATH", env_path_with_bin()?)
                    .status()
                    .map_err(|e| {
                        VtaError::new(Area::Env, 1, format!("running task `{name}`: {e}"))
                    })?;
                return Ok(status_exit(status));
            }
        }
    }
    let idx = rest.iter().position(|a| a == &name).unwrap_or(0);
    let args: &[String] = rest.get(idx + 1..).unwrap_or(&[]);
    let tool_bin = home()?.join("bin").join(&name);
    let status = Command::new(&tool_bin)
        .args(args)
        .env("PATH", env_path_with_bin()?)
        .status()
        .map_err(|e| VtaError::new(Area::Env, 1, format!("running `{name}`: {e}")))?;
    Ok(status_exit(status))
}

/// `vanta bundle [--out file]` — pack the active generation for offline transfer.
fn cmd_bundle(rest: &[String]) -> VtaResult<ExitCode> {
    let out = rest
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| rest.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "vanta-bundle.vbundle".to_string());
    let engine = Engine::open(home()?)?;
    let n = engine.bundle_current(Path::new(&out))?;
    println!("bundled {n} store entries → {out}");
    Ok(ExitCode::Ok)
}

/// `vanta restore <file>` — import a bundle (verifying integrity).
fn cmd_restore(rest: &[String]) -> VtaResult<ExitCode> {
    let file = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(f) => f,
        None => {
            eprintln!("usage: vanta restore <file>");
            return Ok(ExitCode::Usage);
        }
    };
    let engine = Engine::open(home()?)?;
    let n = engine.restore(Path::new(file))?;
    println!("restored {n} store entries");
    Ok(ExitCode::Ok)
}

/// `vanta outdated` — show current (locked) vs allowed vs latest per manifest tool.
#[allow(clippy::print_literal)] // aligned header columns read clearer as args
fn cmd_outdated() -> VtaResult<ExitCode> {
    let manifest_path = find_manifest()?;
    let manifest = vanta_config::load_file(&manifest_path)?;
    let registry = load_registry()?;
    let resolver = Resolver::new(&registry);
    let platform = Platform::current();

    let lock_path = manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("vanta.lock");
    let locked: BTreeMap<String, String> = if lock_path.exists() {
        vanta_lock::Lock::load_file(&lock_path)
            .map(|l| l.tools.into_iter().map(|t| (t.name, t.version)).collect())
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };

    println!(
        "{:<16} {:<12} {:<12} {}",
        "tool", "current", "allowed", "latest"
    );
    for (tool, spec) in &manifest.tools {
        let allowed = resolver
            .resolve(
                &Request {
                    tool: tool.clone(),
                    version: VersionReq::parse(spec.version()),
                },
                &[platform],
            )
            .map(|r| r.version)
            .unwrap_or_else(|_| "-".to_string());
        let latest = resolver
            .resolve(
                &Request {
                    tool: tool.clone(),
                    version: VersionReq::Latest,
                },
                &[platform],
            )
            .map(|r| r.version)
            .unwrap_or_else(|_| "-".to_string());
        let current = locked.get(tool).cloned().unwrap_or_else(|| "-".to_string());
        println!("{tool:<16} {current:<12} {allowed:<12} {latest}");
    }
    Ok(ExitCode::Ok)
}

/// `vanta cache <stats|prune>` — inspect or clear the download cache.
fn cmd_cache(rest: &[String]) -> VtaResult<ExitCode> {
    let sub = rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("stats");
    let downloads = home()?.join("cache").join("downloads");
    match sub {
        "prune" => {
            let mut n = 0;
            if let Ok(rd) = std::fs::read_dir(&downloads) {
                for e in rd.flatten() {
                    if std::fs::remove_file(e.path()).is_ok() {
                        n += 1;
                    }
                }
            }
            println!("pruned {n} cached downloads");
        }
        _ => {
            let (mut files, mut bytes) = (0u64, 0u64);
            if let Ok(rd) = std::fs::read_dir(&downloads) {
                for e in rd.flatten() {
                    if let Ok(m) = e.metadata() {
                        if m.is_file() {
                            files += 1;
                            bytes += m.len();
                        }
                    }
                }
            }
            println!("download cache: {files} files, {} KB", bytes / 1024);
        }
    }
    Ok(ExitCode::Ok)
}

/// `vanta config` — show the global config path and contents.
fn cmd_config() -> VtaResult<ExitCode> {
    let path = home()?.join("config.toml");
    println!("config: {}", path.display());
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            println!("---");
            print!("{contents}");
        }
        Err(_) => println!("(no global config; create it to set [tools]/[settings])"),
    }
    Ok(ExitCode::Ok)
}

/// `vanta completions <shell>` — emit a basic completion script.
fn cmd_completions(rest: &[String]) -> VtaResult<ExitCode> {
    let shell = rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("bash");
    let cmds = "add remove update sync list which search info outdated init migrate doctor activate gc rollback generations run exec x bundle restore cache config completions use";
    match shell {
        "bash" => println!("complete -W \"{cmds}\" vanta vt"),
        "zsh" => println!("#compdef vanta vt\n_values 'vanta command' {cmds}"),
        "fish" => {
            for c in cmds.split(' ') {
                println!("complete -c vanta -a {c}");
            }
        }
        other => {
            eprintln!("vanta: no completions for `{other}`");
            return Ok(ExitCode::Usage);
        }
    }
    Ok(ExitCode::Ok)
}

/// `vanta trust [path]` — record a manifest's content hash as trusted (TOFU).
fn cmd_trust(rest: &[String]) -> VtaResult<ExitCode> {
    let trust_dir = home()?.join("trust");
    if has_flag(rest, "--list") {
        match std::fs::read_dir(&trust_dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    if let Ok(target) = std::fs::read_to_string(e.path()) {
                        println!("{}  {}", e.file_name().to_string_lossy(), target);
                    }
                }
            }
            Err(_) => println!("(nothing trusted yet)"),
        }
        return Ok(ExitCode::Ok);
    }
    let path = match rest.iter().find(|a| !a.starts_with('-')) {
        Some(p) => PathBuf::from(p),
        None => find_manifest()?,
    };
    let hash = vanta_security::sha256_file(&path)?;
    std::fs::create_dir_all(&trust_dir)
        .map_err(|e| VtaError::new(Area::Vrf, 3, format!("trust dir: {e}")))?;
    std::fs::write(trust_dir.join(&hash), path.display().to_string())
        .map_err(|e| VtaError::new(Area::Vrf, 3, format!("recording trust: {e}")))?;
    println!("trusted {} ({hash})", path.display());
    Ok(ExitCode::Ok)
}

/// `vanta registry <list|add <name> <url>>` — manage configured registries.
fn cmd_registry(rest: &[String]) -> VtaResult<ExitCode> {
    let nonflags: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
    let cfg = home()?.join("config.toml");
    match nonflags.first().map(|s| s.as_str()) {
        Some("add") => {
            if nonflags.len() < 3 {
                eprintln!("usage: vanta registry add <name> <url>");
                return Ok(ExitCode::Usage);
            }
            let (name, url) = (nonflags[1], nonflags[2]);
            let block = format!("\n[registries.{name}]\nurl = \"{url}\"\n");
            if let Some(parent) = cfg.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut existing = std::fs::read_to_string(&cfg).unwrap_or_default();
            existing.push_str(&block);
            std::fs::write(&cfg, existing)
                .map_err(|e| VtaError::new(Area::Cfg, 1, format!("writing config: {e}")))?;
            println!("added registry {name} → {url}");
            Ok(ExitCode::Ok)
        }
        _ => {
            if cfg.exists() {
                let manifest = vanta_config::load_file(&cfg)?;
                if manifest.registries.is_empty() {
                    println!("(no registries configured; the official registry is used)");
                } else {
                    for (name, reg) in &manifest.registries {
                        println!("{name}  {}", reg.url);
                    }
                }
            } else {
                println!("(no config; the official registry is used by default)");
            }
            Ok(ExitCode::Ok)
        }
    }
}

/// `vanta shell <tool>@<ver> ...` — install (if needed) and start a subshell with
/// the tools on PATH.
fn cmd_shell(rest: &[String]) -> VtaResult<ExitCode> {
    let specs: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
    if specs.is_empty() {
        eprintln!("usage: vanta shell <tool>[@version] ...");
        return Ok(ExitCode::Usage);
    }
    let registry = load_registry()?;
    let resolver = Resolver::new(&registry);
    let platform = Platform::current();
    let engine = Engine::open(home()?)?;
    for spec in &specs {
        let request = Request::parse(spec)?;
        let resolution = resolver.resolve(&request, &[platform])?;
        let artifact = artifact_for(&resolution, &platform).ok_or_else(|| {
            VtaError::new(
                Area::Res,
                5,
                format!("no artifact for `{}`", resolution.tool),
            )
        })?;
        engine.install_artifact(&resolution.tool, &resolution.version, artifact)?;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(windows) {
            "cmd".to_string()
        } else {
            "/bin/sh".to_string()
        }
    });
    println!(
        "entering vanta subshell ({}); type `exit` to leave",
        specs.len()
    );
    let status = Command::new(shell)
        .env("PATH", env_path_with_bin()?)
        .status()
        .map_err(|e| VtaError::new(Area::Env, 1, format!("starting subshell: {e}")))?;
    Ok(status_exit(status))
}

/// `vanta self <uninstall|update>` — manage the Vanta installation itself.
fn cmd_self(rest: &[String]) -> VtaResult<ExitCode> {
    match rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
    {
        Some("uninstall") => {
            let h = home()?;
            if !has_flag(rest, "--yes") {
                eprintln!(
                    "this will permanently remove {} — re-run with --yes",
                    h.display()
                );
                return Ok(ExitCode::Usage);
            }
            std::fs::remove_dir_all(&h).map_err(|e| {
                VtaError::new(Area::Sys, 2, format!("removing {}: {e}", h.display()))
            })?;
            println!("removed {}", h.display());
            Ok(ExitCode::Ok)
        }
        Some("update") => {
            println!(
                "self-update is handled by the channel you installed from; \
                 see docs/32-release-engineering.md"
            );
            Ok(ExitCode::Ok)
        }
        _ => {
            eprintln!("usage: vanta self <uninstall|update>");
            Ok(ExitCode::Usage)
        }
    }
}

fn env_path_with_bin() -> VtaResult<String> {
    let bin = home()?.join("bin");
    let sep = if cfg!(windows) { ';' } else { ':' };
    Ok(format!(
        "{}{}{}",
        bin.display(),
        sep,
        std::env::var("PATH").unwrap_or_default()
    ))
}

fn shell_command(cmd: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    }
}

fn status_exit(status: std::process::ExitStatus) -> ExitCode {
    if status.success() {
        ExitCode::Ok
    } else {
        ExitCode::Failure
    }
}

/// `vanta init` / `vanta migrate` — detect foreign version files and write a
/// `vanta.toml` (`docs/30-migration.md`).
fn cmd_import(force: bool) -> VtaResult<ExitCode> {
    let cwd = std::env::current_dir()
        .map_err(|e| VtaError::new(Area::Cfg, 1, format!("cannot read current directory: {e}")))?;
    let imported = vanta_migrate::import_dir(&cwd);
    if imported.is_empty() {
        println!("no version files detected in {}", cwd.display());
        return Ok(ExitCode::Ok);
    }
    let target = cwd.join("vanta.toml");
    if target.exists() && !force {
        eprintln!("vanta.toml already exists (use --force to overwrite)");
        return Ok(ExitCode::Usage);
    }
    println!("detected:");
    for i in &imported {
        println!("  {} = \"{}\"  (from {})", i.tool, i.version, i.source);
    }
    let body = vanta_migrate::to_manifest_toml(&imported);
    std::fs::write(&target, body)
        .map_err(|e| VtaError::new(Area::Cfg, 1, format!("writing {}: {e}", target.display())))?;
    println!(
        "✓ wrote {} — run `vanta sync` to install + lock",
        target.display()
    );
    Ok(ExitCode::Ok)
}

fn has_flag(rest: &[String], flag: &str) -> bool {
    rest.iter().any(|a| a == flag)
}

/// Resolve `$VANTA_HOME` (or `~/.vanta`).
fn home() -> VtaResult<PathBuf> {
    if let Ok(h) = std::env::var("VANTA_HOME") {
        return Ok(PathBuf::from(h));
    }
    let base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            VtaError::new(
                Area::Sys,
                2,
                "cannot determine home directory; set VANTA_HOME".to_string(),
            )
        })?;
    Ok(PathBuf::from(base).join(".vanta"))
}

/// Load the registry: an HTTP(S) URL or a local file via `$VANTA_REGISTRY`, else
/// the built-in index. A network registry is fetched (mirror/retry-aware) and parsed.
fn load_registry() -> VtaResult<Registry> {
    match std::env::var("VANTA_REGISTRY") {
        Ok(loc) if loc.starts_with("http://") || loc.starts_with("https://") => {
            let tmp =
                std::env::temp_dir().join(format!("vanta-registry-{}.toml", std::process::id()));
            vanta_net::Downloader::new()?.download(&loc, &tmp)?;
            let registry = Registry::load_file(&tmp);
            let _ = std::fs::remove_file(&tmp);
            registry
        }
        Ok(path) => Registry::load_file(Path::new(&path)),
        Err(_) => Ok(Registry::builtin()),
    }
}

fn print_help() {
    println!(
        "vanta — every developer tool, one command\n\
         \n\
         USAGE:\n    vanta <command> [args]\n\
         \n\
         COMMON COMMANDS:\n\
         \x20   add <tool>[@ver]    resolve and install a tool\n\
         \x20   search <query>      search the registry\n\
         \x20   info <tool>         show a tool's versions\n\
         \x20   remove <tool>       remove a tool\n\
         \x20   update [tool]       update within constraints\n\
         \x20   sync                reconcile to vanta.toml + vanta.lock\n\
         \x20   doctor              diagnose the installation\n\
         \n\
         See docs/04-cli.md for the full reference."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ok() {
        assert_eq!(run(&["--version".into()]).unwrap(), ExitCode::Ok);
    }

    #[test]
    fn unknown_is_usage() {
        assert_eq!(run(&["frobnicate".into()]).unwrap(), ExitCode::Usage);
    }

    #[test]
    fn add_no_args_is_usage() {
        assert_eq!(run(&["add".into()]).unwrap(), ExitCode::Usage);
    }

    #[test]
    fn add_unknown_tool_resolves_to_error() {
        // Resolution fails for an unknown tool before any disk/network side effect.
        let err = run(&["add".into(), "totally-unknown-tool".into()]).unwrap_err();
        assert_eq!(err.area, Area::Res);
    }

    #[test]
    fn search_succeeds() {
        assert_eq!(
            run(&["search".into(), "node".into()]).unwrap(),
            ExitCode::Ok
        );
    }
}
