# Scope Type System

The scope system controls **what the AI can touch**. Every tool parameter that references the filesystem is typed, and the scope constrains what values are legal at runtime.

## Capability Model

A scope is a set of capabilities granted to a profile:

```rust
pub struct Capabilities {
    pub file_scope: FileScope,
    pub shell: bool,
    pub network: bool,
}
```

### File Scope Hierarchy

```
Selection  ⊂  File  ⊂  Project  ⊂  Any
```

| Level | Can access | Use case |
|-------|-----------|----------|
| `Selection` | Only the active visual selection | Cheap models doing inline edits |
| `File` | The current buffer (by path) | Single-file edits, diagnostics |
| `Project` | Any file under the project root | Cross-file analysis, refactoring |
| `Any` | Any absolute path on disk | Unrestricted (dangerous, use sparingly) |

Subtyping: `Selection` is a subtype of `File` — anything that can operate on a selection can also operate on a file, but not vice versa. A tool that requires `File` scope works when granted `Project` or `Any`, but not when granted only `Selection`.

### Orthogonal Capabilities

`Shell` and `Network` are independent of the file hierarchy:

- **Shell**: Can execute commands via the system shell. Required by tools like `cargo_test`, `cargo_clippy`, or any user-defined tool that calls `vim.fn.system()`.
- **Network**: Can make HTTP requests. Required by tools that call external APIs (rare — most AI communication goes through the provider layer, not through tools).

These must be explicitly granted. A profile with `{ files = "project" }` cannot shell out even if it has a tool that needs it.

## Runtime Enforcement

When a tool is invoked, the runtime validates **every parameter** against the active scope before the handler runs:

```rust
impl Capabilities {
    pub fn validate_path(
        &self,
        path: &Path,
        ctx: &ScopeContext,
    ) -> Result<()> {
        match self.file_scope {
            FileScope::Selection => {
                bail!("file access not permitted in selection scope")
            }
            FileScope::File => {
                if path != ctx.current_file {
                    bail!("path {:?} is outside file scope (current: {:?})",
                          path, ctx.current_file)
                }
            }
            FileScope::Project => {
                if !path.starts_with(&ctx.project_root) {
                    bail!("path {:?} is outside project root {:?}",
                          path, ctx.project_root)
                }
            }
            FileScope::Any => {}
        }
        Ok(())
    }

    pub fn check_shell(&self) -> Result<()> {
        if !self.shell { bail!("shell access not permitted") }
        Ok(())
    }

    pub fn check_network(&self) -> Result<()> {
        if !self.network { bail!("network access not permitted") }
        Ok(())
    }
}
```

### ScopeContext

The runtime provides context for validation:

```rust
pub struct ScopeContext {
    pub current_file: PathBuf,      // The active buffer's file path
    pub project_root: PathBuf,      // Detected project root (git root, Cargo.toml, etc.)
    pub selection: Option<Range>,   // Active selection, if any
}
```

## Scope Composition

Scopes compose via intersection. When a context ceiling meets a profile scope, the effective scope is the **minimum** of the two:

```
effective_scope = min(context_ceiling, profile_scope)
```

Example:
- Profile `opus` grants `{ files = "project", shell = true }`
- Context `query` has ceiling `{ files = "project", shell = false, mutations = false }`
- Effective scope: `{ files = "project", shell = false }` + all mutation tools stripped

This is why contexts are safe to point at powerful profiles — the ceiling always wins.

## Tool Scope Declaration

Each tool declares its **minimum required scope** — the least privilege it needs:

```rust
pub struct ToolDefinition {
    pub required_scope: RequiredScope,
    // ...
}

pub struct RequiredScope {
    pub file_scope: FileScope,  // Minimum file access needed
    pub shell: bool,            // Needs shell access?
    pub network: bool,          // Needs network access?
}
```

At profile registration, the runtime checks that every tool's required scope fits within the profile's granted scope. Tools that don't fit are excluded with a warning (not an error — you might add tools speculatively).

## Lua Configuration

```lua
-- In vim.ai.setup()
profiles = {
  sonnet = {
    -- Scope as a table with explicit capabilities
    scope = {
      files = "project",    -- "selection" | "file" | "project" | "any"
      shell = true,         -- default: false
      network = false,      -- default: false
    },

    -- Shorthand: just the file scope (shell=false, network=false)
    -- scope = "project",
  },
}
```

## Design Rationale

**Why not just per-tool permissions?** Permissions control *approval policy* (auto/confirm/deny). Scopes control *capability boundaries*. A tool with `permission = "auto"` still can't escape its scope. These are orthogonal: you might auto-approve `read_file` within project scope but deny it for `any` scope. The scope is the hard boundary; the permission is the UX policy.

**Why intersection for composition?** Union would be unsafe — a restrictive context could be widened by a permissive profile. Intersection guarantees monotonic restriction: adding constraints never increases access.

**Why declare scope on tools?** So the runtime can pre-filter tools before sending them to the model. A model shouldn't see tools it can't use — that wastes context tokens and confuses tool selection.
