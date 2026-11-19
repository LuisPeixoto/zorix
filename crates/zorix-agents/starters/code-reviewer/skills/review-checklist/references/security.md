# Patterns worth a second look

- String building next to `SELECT`, `INSERT`, `exec`, `system`, `eval` or a file path: use parameters, argument lists and path checks.
- `verify=False`, disabled certificate checks, `http://` for anything private.
- Comparing secrets with `==`; hashing passwords with a fast hash.
- Logging request bodies, tokens or whole exception objects that hold them.
- `pickle.loads`, `yaml.load` without a safe loader, unbounded `json` or archive extraction from untrusted input.
- A new endpoint or command without an authorisation check next to its siblings.
