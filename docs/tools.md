# Tools

A tool is something the model can ask the runtime to do. Built-in and external tools look the same to the runtime. The
contract is shaped like the Model Context Protocol's tool definition ([ADR-0008](adr/0008-tool-contract.md)), so an MCP
adapter in Phase 4 is just another implementation.

## The contract

```rust
pub struct ToolSpec {
    id: String,                 // "math.evaluate": stable, and the name the model uses
    name: String,               // "Calculator": for people
    description: String,        // written for the model
    input_schema: Value,        // JSON Schema of the arguments
    output_schema: Option<Value>,
    annotations: ToolAnnotations,   // read_only, destructive, idempotent, open_world
    required_permissions: Vec<Permission>,
    requires_approval: bool,
    timeout_ms: u64,
    max_output_bytes: usize,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError>;
}
```

Annotations **describe**; they never grant. They assume the worst by default (destructive, open-world, not idempotent)
until the author says otherwise. `idempotent` matters for recovery: a tool that was running when the process stopped is
run again only if it is idempotent, otherwise a person decides.

`ToolError` is `Retryable` (try again) or `Failed` (cannot succeed as it is).

A tool receives a `ToolContext`: the execution and call ids, the tenant, the grants, a cancellation token and the
workspace folder. It has no other access to the runtime.

## What the runtime does around a call

| Step | Detail |
|---|---|
| Register | `ToolRegistry::register` checks the id, the timeout, and that both schemas are valid JSON Schemas |
| Validate | Arguments are checked against `input_schema` before anything runs. The model gets the problems as an error |
| Authorise | The policy engine decides now. See [permissions](permissions.md) |
| Run | With `timeout_ms`; cancelling the execution stops the call. A tool that stops because of a cancel is reported as cancelled, not failed |
| Check | The result is checked against `output_schema` when there is one |
| Cap | A result over `max_output_bytes` becomes `{"truncated": true, "original_bytes": …, "preview": …}`. It is cut **before** it is stored, so JSON is never cut in the middle |

## The built-in tools

They exist to prove the architecture. Use `zorix tools` to list them.

| Tool | Does | Care |
|---|---|---|
| `clock.now` | Current UTC time | read-only |
| `math.evaluate` | Arithmetic with `+ - * / % ^` and parentheses. A small parser: no `eval`, no variables, bounded length and depth | read-only |
| `text.stats` | Characters, words, lines | read-only |
| `notes.get` | Reads a note (per tenant) | read-only |
| `notes.put` | Saves a note (per tenant, 1000 at most, 4 KiB each) | **asks first** |
| `workspace.list` | Lists a folder of the workspace (200 entries at most) | read-only |
| `workspace.read` | Reads a text file (first 64 KiB) | read-only |
| `workspace.write` | Creates or replaces a file (256 KiB at most) | **asks first** |

### The workspace is a sandbox for paths

The file tools only touch the folder given as `--workspace` (per execution, never global). Two checks apply to every
call:

1. **The grant** must cover `workspace:<relative path>` for `read` or `write`.
2. **The path** must be relative, with forward slashes and no `..`, no drive letter, no backslash, no control
   characters, no names Windows reserves (`CON`, `NUL`…), and not inside the reserved `.zorix` folder. After that the real
   path (following symbolic links) must still be inside the workspace. A link that points outside is not followed, and
   ZORIX will not write *through* a link.

Writes go to a temporary file that is renamed over the target, so a crash never leaves half a file.

This is confinement of **paths**, not a code sandbox. No built-in tool runs code. A tool that runs commands will need
real isolation first; that is planned work, not part of this phase.

## Writing a tool

See [`examples/embed.rs`](../crates/zorix-runtime/examples/embed.rs):

```rust
let mut spec = ToolSpec::new("email.send", "Send email", "Sends an email.", schema);
spec.annotations = ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: true };
registry.register(Arc::new(SendEmail { spec }))?;
```

Guidance:

- Write the schema tightly (`required`, `additionalProperties: false`, `maxLength`). Validation is your first defence.
- Check resource permissions yourself when the resource is only known at call time, as the file tools do.
- Declare `idempotent` honestly. When in doubt, it is not.
- Keep results small and put the useful part first.
- Never put secrets in a result: the model will see it.
