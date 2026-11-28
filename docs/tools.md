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
   characters, no names Windows reserves (`CON`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, `CONIN$`, `CONOUT$`, the superscript forms, with or without an extension or spaces before it), none of the characters Windows does not allow in names (`< > " | ? *`), no name written like a short Windows name (`PROGRA~1`, which on some volumes is another file's name, `.zorix` included), and not inside the reserved `.zorix` folder (in any case: `.ZORIX` is the same folder on macOS and Windows). Nothing is **written** inside a `.git` folder, at any depth: git would run or obey it (reading there is allowed with a grant). After that the real
   path (following symbolic links) must still be inside the workspace. A link that points outside is not followed, and
   ZORIX will not write *through* a link. **A link inside the workspace is held to the place it leads to**: that place needs
   its own grant (`docs/shortcut` pointing at `private/notes.txt` is read only if `workspace:private/notes.txt` is granted
   too), and it cannot be a reserved one (a link to `.zorix` is refused).

Writes go to a temporary file that is renamed over the target, so a crash never leaves half a file. The temporary file is
created new (anything already at its name, a link included, makes the write pick another name) and its name carries a number
nobody can guess, so two files with the same stem (`a.txt` and `a.md`) never share one. On Windows, where the rename is not instant
for others, a read that meets it asks again for a few milliseconds before it says there is a problem.

**A limit, stated plainly:** the real path is checked and then the file is opened, as two steps. Somebody who can change the workspace folder *at that very moment* (another program of
the same user, replacing a folder with a link between the check and the open) could redirect one call. Closing that gap needs `openat` and `O_NOFOLLOW` all the way down, which is not
portable to Windows; the workspace is a folder you gave the run, so this is documented rather than hidden. Do not point a run at a folder another program is writing to.

This is confinement of **paths**, not a code sandbox. The file tools run no code. Code runs only in the [sandbox](sandbox.md)
(`code.run`, JavaScript and Python in WebAssembly); no tool runs a command on the host, and one that did would need real isolation first.

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
