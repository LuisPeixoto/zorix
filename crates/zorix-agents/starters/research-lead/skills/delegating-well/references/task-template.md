# A task that stands alone

```
Goal: Find the boiling point of ethanol at sea level, in °C.
Form: One sentence with the number, then the source's title and address.
Look at: A chemistry reference or a standards body; not forums.
Leave out: Mixtures and other pressures.
```

The same in the arguments of `agents.delegate`:

```json
{"tasks": [
  {"agent": "researcher",
   "task": "Find the boiling point of ethanol at sea level, in °C. Answer in one sentence with the number, then the source's title and address. Prefer a chemistry reference or a standards body.",
   "expected_output": {"type": "object", "required": ["celsius", "source"]}}
]}
```
