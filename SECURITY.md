# Security policy

ZORIX runs AI models that can ask to use tools on your computer, so security is part of the design, not an add-on.

## Supported versions

Only the latest release receives security fixes while the project is below 1.0.

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private reporting instead:

<https://github.com/LuisPeixoto/zorix/security/advisories/new>

Include, where possible: a description and impact, steps to reproduce or a proof of concept, and the affected versions
and operating systems. You will get an acknowledgement as soon as the maintainer can review it, and credit in the
release notes unless you prefer to stay anonymous.

## What is in scope

- A model request (including text injected through tool output or files) that gets around a permission, a policy or a
  required approval
- An approval that can be forged, replayed, or applied to a different tool call than the one shown to the user
- A built-in tool reaching files outside its workspace root
- Secrets (API keys, tokens) appearing in logs, events, traces or the database
- Data from one tenant or execution becoming visible to another
- Memory safety or parsing flaws in the persistence layer

## Design notes

These notes grow with each feature that handles data, files, network or permissions.

- **Deny by default.** An execution only has the permissions it was created with. Tools are checked when they are
  *run*, not only when they are offered to the model.
- **The model is untrusted.** Its output cannot change grants, policy or approvals. Tool results are added to the
  context labelled as untrusted data.
- **Approvals are bound to one call.** An approval names the runtime-issued id and the hash of the tool and its exact
  arguments. A changed argument needs a new approval.
- **Bounded everything.** Iterations, tool calls, tokens, wall-clock time, tool run time and tool output size all have
  limits (see `docs/runtime.md`).
- **Local data.** The database lives in the data folder (`ZORIX_HOME`, or the platform default). ZORIX has no
  telemetry and makes no network connection of its own in this version.
