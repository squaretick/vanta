//! The sandboxed execution primitive for WASM provider hooks
//! (`docs/22-provider-sdk.md`, `docs/15-security.md`).
//!
//! Guest modules run under Wasmtime with **no ambient authority** — no WASI, no
//! host imports are provided unless explicitly granted — and with a **fuel
//! budget** so a malicious or runaway hook traps instead of hanging the host.
//! This is the security core of the provider model; the full component-model WIT
//! ABI (scoped `http-get`, `hash`) builds on this primitive.

use vanta_core::{Area, VtaError, VtaResult};
use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimitsBuilder};

/// Memory ceiling for a guest instance (audit L10): caps `memory.grow` so a
/// guest cannot exhaust host RAM (`memory.grow` past this fails instead of
/// climbing toward the 4 GiB wasm32 maximum).
const MAX_MEMORY_BYTES: usize = 256 * 1024 * 1024; // 256 MiB

/// A capability-free WASM sandbox.
pub struct Sandbox {
    engine: Engine,
}

impl Sandbox {
    /// Create a sandbox with fuel metering enabled and no host capabilities.
    pub fn new() -> VtaResult<Sandbox> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|e| err(format!("engine: {e}")))?;
        Ok(Sandbox { engine })
    }

    /// Run an exported `func(i32) -> i32` in a fresh, capability-free instance
    /// under a fuel budget. Instantiation fails if the module imports anything
    /// (no ambient authority is granted); the call traps cleanly if it exhausts
    /// its fuel. Compute-only hooks use this; richer hooks extend the host set.
    pub fn run_i32(&self, wasm: &[u8], func: &str, arg: i32, fuel: u64) -> VtaResult<i32> {
        // TODO(security, L10): provider modules are currently unsigned. Before
        // instantiating untrusted modules in production, verify `wasm` against a
        // pinned provider-signing key (reusing `vanta-security`'s minisign
        // verification) so only vetted hooks ever reach `Module::new`.
        let module = Module::new(&self.engine, wasm).map_err(|e| err(format!("compile: {e}")))?;
        // L10: bound guest memory so `memory.grow` cannot exhaust host RAM.
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY_BYTES)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|state| state as &mut dyn wasmtime::ResourceLimiter);
        store
            .set_fuel(fuel)
            .map_err(|e| err(format!("set fuel: {e}")))?;
        // Empty import list: a module requiring any import cannot instantiate.
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|e| err(format!("instantiate (no capabilities granted): {e}")))?;
        let typed = instance
            .get_typed_func::<i32, i32>(&mut store, func)
            .map_err(|e| err(format!("export `{func}`: {e}")))?;
        typed
            .call(&mut store, arg)
            .map_err(|e| err(format!("guest trap: {e}")))
    }
}

fn err(msg: String) -> VtaError {
    VtaError::new(Area::Prov, 1, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A compute-only module: (func (export "double") (param i32) (result i32) ...).
    const DOUBLE: &str = r#"(module
        (func (export "double") (param i32) (result i32)
            local.get 0
            i32.const 2
            i32.mul))"#;

    // A module that loops forever — must trap on fuel exhaustion, not hang.
    const SPIN: &str = r#"(module
        (func (export "spin") (param i32) (result i32)
            (loop (br 0))
            i32.const 0))"#;

    // A module that imports a host function it is not granted — must fail to
    // instantiate (no ambient authority).
    const NEEDS_IMPORT: &str = r#"(module
        (import "env" "secret" (func $secret (result i32)))
        (func (export "go") (param i32) (result i32) (call $secret)))"#;

    #[test]
    fn runs_pure_compute() {
        let wasm = wat::parse_str(DOUBLE).unwrap();
        let sb = Sandbox::new().unwrap();
        assert_eq!(sb.run_i32(&wasm, "double", 21, 1_000_000).unwrap(), 42);
    }

    #[test]
    fn fuel_exhaustion_traps_not_hangs() {
        let wasm = wat::parse_str(SPIN).unwrap();
        let sb = Sandbox::new().unwrap();
        let err = sb.run_i32(&wasm, "spin", 0, 10_000).unwrap_err();
        assert_eq!(err.area, Area::Prov); // trapped (out of fuel), did not hang
    }

    #[test]
    fn imports_are_denied() {
        let wasm = wat::parse_str(NEEDS_IMPORT).unwrap();
        let sb = Sandbox::new().unwrap();
        // No host functions are provided, so instantiation must fail.
        assert!(sb.run_i32(&wasm, "go", 0, 1_000_000).is_err());
    }
}
