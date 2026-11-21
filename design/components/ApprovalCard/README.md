# ApprovalCard

The moment ZORIX stops and asks a person: the tool, the exact arguments, why, and a fingerprint of the call.

- The answer is bound to this call by hash and can be given once. It survives closing the app.
- Shows what the change would do: a SQL statement highlighted, a file diff, a message preview.
- Actions: Approve (primary), Refuse (secondary), and "Always allow for this run" as a ghost button when policy permits.
- An approval from a sub-run or a workflow step names where it came from ("asked by researcher, 2 levels down").
