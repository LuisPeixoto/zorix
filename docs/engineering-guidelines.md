# Engineering guidelines

These guidelines are mandatory for anyone, human or automated, who works on ZORIX. The goal is open-source software of
excellent quality: sustainable architecture, an excellent experience, high reliability, and the ability to compete with the
best tools on the market.

Work as a complete engineering team: take responsibility for architecture, development, tests, security, UX/UI,
documentation, quality and maintenance.

**The main rule: never put speed of implementation ahead of quality, security, stability or the user's experience.**

[How ZORIX applies them](#how-zorix-applies-them) and the [known gaps](#known-gaps) are at the end. The day-to-day
commands are in [CONTRIBUTING.md](../CONTRIBUTING.md), and the entry point for contributors is [AGENTS.md](../AGENTS.md).

---

## 1. GitHub and the open-source ecosystem

Use the GitHub features that build and maintain a professional open-source project.

- Keep Issues organised for bugs, improvements, new features and technical debt.
- Read existing Issues before implementing something, to avoid duplication.
- Use Pull Requests to organise, review and track changes.
- Keep Discussions for questions, ideas, feedback and community conversation.
- Use GitHub Projects to track the roadmap, tasks and progress.
- Use GitHub Actions for automation, tests, validation and CI/CD.
- Configure automatic checks before merges are allowed.
- Use Releases and Tags to control versions.
- Keep the CHANGELOG up to date.
- Use GitHub Security, Dependabot and vulnerability analysis tools.
- Keep templates for Issues and Pull Requests.
- Keep CONTRIBUTING.md, CODE_OF_CONDUCT.md and SECURITY.md up to date.
- Keep the README complete, current and accessible.
- Use CODEOWNERS where it makes sense.
- Evaluate GitHub Sponsors and other open-source sustainability initiatives.
- Keep the roadmap public and current.

Use the GitHub features whenever they add to the quality, transparency and maintenance of the project.

## 2. Tests and quality assurance

No implementation is complete without adequate validation.

- Run tests before and after any relevant change.
- Make sure existing features keep working.
- Write tests for every new feature.
- Write regression tests for every fixed bug.
- Write unit, integration and end-to-end tests where they apply.
- Test positive, negative, extreme and unexpected scenarios.
- Test empty and invalid input, maximum limits and inconsistent data.
- Test network failures, timeouts, unavailable providers and interruptions.
- Test concurrency, asynchronous operations and race conditions.
- Test persistence, state recovery and data integrity.
- Do not write tests that only confirm a function exists without checking its behaviour.
- Avoid excessive mocks that do not represent real operation.
- Do not ignore failing tests to finish an implementation.
- Do not remove or weaken existing tests just to make the pipeline pass.
- Aim for high coverage, especially in critical modules, without chasing artificial numbers.
- Make tests reproducible: no random or unstable results.

**Cross-platform tests.** ZORIX must work correctly on macOS, Windows and Linux. Any change that can affect
operating-system-specific behaviour must be validated on the matching platforms. The pipeline covers all three whenever it
is technically possible.

## 3. Software quality and product excellence

ZORIX aims to be one of the most complete and reliable tools on the market.

- Implement complete features, not superficial ones.
- Think about the user's experience from the planning stage.
- Consider real usage scenarios.
- Avoid features that are incomplete, inconsistent or that work only in ideal scenarios.
- Do not implement only the happy path.
- Identify limitations and opportunities for improvement while developing.
- Compare features with competing tools when relevant.
- Identify important features that may be missing.
- Prefer robust, scalable and sustainable solutions.
- Do not create unnecessary features just to increase the number of features.
- Look for real differentiators against competing tools.
- Keep features, modules and platforms consistent with each other.

Whenever a feature is implemented, check that it is complete from the functional, technical and user-experience points of
view.

## 4. UX/UI and responsiveness

The interface has the visual and functional quality of a high-level commercial product.

- Every screen is fully responsive.
- Support different resolutions, window sizes and aspect ratios.
- No cut-off, overlapping or off-screen elements.
- No unjustified empty space, misalignment or inconsistent spacing.
- No broken layouts in small windows.
- Behave well on large and small screens.
- Consider dynamic window resizing.
- Tables, forms, menus, dialogs and panels adapt to the available space.
- Use the components and tokens of the design system correctly.
- Avoid duplicated styles and inconsistent visual values.
- Keep colours, typography, icons, borders, shadows and spacing consistent.
- Provide loading, empty, error, success and disabled states.
- Give visual feedback for every relevant action.
- Avoid confusing interactions or ones that need unnecessary technical knowledge.
- Do not use purely decorative elements that harm usability.
- Keep navigation intuitive and predictable.
- Consider accessibility in every component.

Always validate the implemented screens visually, preferably at several resolutions, and fix the problems found before
finishing.

## 5. Architecture and code organisation

Preserve and respect the architecture defined for ZORIX.

- Respect the separation of responsibilities between crates and modules.
- Keep coupling low and cohesion high.
- Avoid circular dependencies.
- Do not duplicate business rules.
- Reuse existing components, services and abstractions.
- Avoid very large files and modules with several responsibilities.
- Avoid complex functions that are hard to test.
- Use abstractions only when they add real value.
- Avoid overengineering.
- Do not mix interface, domain, infrastructure and persistence responsibilities.
- Keep contracts and interfaces well defined.
- Prefer simple, explicit and predictable solutions.
- Keep modules compatible with each other.
- Do not introduce unnecessary dependencies.
- Before creating a new abstraction, check whether something equivalent already exists.

**Mandatory rule: before changing an important part of the project, understand how it relates to the other modules and
which features may be affected.**

## 6. Security and privacy

Security is a fundamental requirement of ZORIX, not an optional improvement.

- Apply the principle of least privilege.
- Do not grant permissions implicitly.
- Validate all input from models, tools, integrations and users.
- Treat responses from models and external tools as untrusted data.
- Protect against prompt injection.
- Make sure tool results cannot widen permissions.
- Do not allow arbitrary command execution without proper authorisation.
- Isolate the execution environment.
- Protect credentials, tokens, passwords and API keys.
- Never record secrets in logs, events or error messages.
- Redact sensitive information.
- Validate permissions before executing operations.
- Bind human approvals to the exact operation that was authorised.
- Prevent approvals from being reused improperly.
- Protect local data and the user's files.
- Evaluate the risks of path traversal, command injection, SSRF and privilege escalation.
- Keep dependencies updated and monitored.
- Run a vulnerability analysis before relevant releases.

Every change involving permissions, code execution, tools, agents or integrations must go through a specific security
analysis.

## 7. Performance and efficiency

ZORIX performs well even in intensive use.

- Avoid blocking operations in the interface.
- Do not run heavy operations on the main thread.
- Avoid unnecessary processing.
- Optimise queries and persistence operations.
- Avoid unnecessary allocations and copies.
- Control memory use.
- Avoid memory and resource leaks.
- Shut down processes and connections correctly.
- Use concurrency safely.
- Set limits on potentially costly operations.
- Avoid redundant calls to AI providers.
- Optimise context and token management.
- Monitor latency, resource use and cost.
- Evaluate performance with large numbers of executions, events, tools and agents.

Do not optimise prematurely, but find and fix real bottlenecks.

## 8. Error handling and resilience

The system deals correctly with foreseeable and unexpected failures.

- Never ignore errors silently.
- Do not use default values to hide failures that should be reported.
- Give clear and useful error messages.
- Keep enough information for diagnosis.
- Avoid exposing sensitive information in errors.
- Set timeouts on external operations.
- Retry with backoff where appropriate.
- Avoid repeating operations that are not idempotent.
- Keep state consistent after failures.
- Recover interrupted executions.
- Implement cancellation.
- Make sure partially completed operations do not leave the system inconsistent.
- Anticipate the unavailability of providers, integrations and external services.

## 9. Documentation

Every relevant change has matching documentation.

- Update the documentation when a feature's behaviour changes.
- Document new features, APIs, commands and settings.
- Document architectural decisions with ADRs where relevant.
- Keep examples working and current.
- Document known limitations.
- Document installation, configuration and usage procedures.
- Keep the development documentation current.
- Update the README when something important changes.
- Do not document behaviour that does not exist.
- Do not leave documentation that contradicts the implementation.
- A new developer can understand the project without outside explanations.

## 10. Compatibility and evolution

ZORIX evolves without needlessly breaking existing projects and data.

- Preserve compatibility whenever possible.
- Assess the impact of changes on public APIs.
- Consider the compatibility of settings and persisted files.
- Write migrations when the database changes.
- Make sure old data can be recovered or migrated.
- Avoid unnecessary breaking changes.
- Document incompatible changes.
- Consider earlier versions of providers and integrations.
- Keep contracts consistent between versions.
- Do not remove features without assessing the impact on users.

## 11. Accessibility and internationalisation

- Prepare the interface for several languages.
- Do not scatter interface text through the code.
- Centralise translations.
- Avoid cut-off text when another language changes the length of a sentence.
- Support keyboard navigation.
- Keep a visible focus.
- Use suitable labels and descriptions.
- Keep enough contrast.
- Do not rely on colour alone to carry information.
- Consider screen readers.
- Keep date, time and number formats consistent.

## 12. Observability and traceability

ZORIX makes it possible to understand what happened in every execution.

- Keep structured logs.
- Record relevant events.
- Keep traceability between executions, agents, tools and providers.
- Record times, consumption and results.
- Keep related operations correlated.
- Use metrics and tracing where they apply.
- Avoid excessive logging.
- Redact sensitive information.
- Make errors diagnosable without reproducing the whole scenario.
- Keep the integrity and the order of events.

## 13. AI quality and harness engineering

ZORIX is a harness-engineering platform, so its execution mechanisms need special care.

- Do not trust model responses blindly.
- Validate tool arguments against their schemas.
- Keep explicit limits on tokens, cost, time, iterations and calls.
- Make sure limits are respected in delegations between agents too.
- Prevent child agents from widening the permissions they receive.
- Keep the context of executions isolated.
- Avoid leaking information between users, projects or agents.
- Make sure human approvals cannot be manipulated by model instructions.
- Test prompt-injection scenarios at several points of the flow.
- Test provider failures and fallback mechanisms.
- Check the consistency of results after executions resume.
- Make sure tools are not run twice when they should not be.
- Test behaviour with different models.
- Make sure switching providers does not break existing contracts.
- Write automated evaluations that measure the quality of agents.
- Evaluate behavioural regressions when instructions, models or tools change.

## 14. Dependencies and technologies

- Prefer mature, maintained libraries with a good reputation.
- Evaluate the licence and the security history before adding a dependency.
- Avoid a dependency when the feature can be built simply and safely with existing resources.
- Do not use outdated versions without a reason.
- Avoid abandoned dependencies.
- Pin versions and keep lockfiles current.
- Evaluate the effect of a new dependency on size, performance and maintenance.
- Do not update large sets of dependencies without evaluating the impact.
- Stay compatible with the minimum versions the project supports.

## 15. Mandatory development process

Every task follows this flow:

1. **Understanding:** analyse the request and understand the expected behaviour.
2. **Investigation:** study the existing implementation, the architecture, the dependencies and related features.
3. **Planning:** identify the affected files, the risks, the impact and the tests needed.
4. **Implementation:** develop while respecting these guidelines.
5. **Validation:** run the tests, lint, formatting, static analysis and build that apply.
6. **Regression:** check that existing features still work.
7. **Review:** review your own code for faults, duplication, vulnerabilities and improvements.
8. **Documentation:** update the affected documents.
9. **Delivery:** present what was done, which validations were run and which limitations remain.

A task is not complete while a mandatory step that applies is pending.

## 16. Autonomy and responsibility

Work proactively, and do not limit yourself strictly to the literal request.

- Identify problems related to the task even when they were not mentioned.
- Suggest relevant technical improvements.
- Identify incomplete features.
- Identify possible regressions.
- Identify inconsistencies between modules.
- Question decisions that could compromise architecture or security.
- Do not assume an existing implementation is correct just because it exists.
- Do not change features outside the scope without need or authorisation.
- Do not hide limitations or failures you find.
- Do not claim tests were run when they were not.
- Do not claim a feature is complete when known issues remain.

Act as the person responsible for the quality of the result, not only as an executor of commands.

---

## 17. Final approval criteria

An implementation is complete only when the criteria below that apply are met:

- [ ] Meets the requested requirement.
- [ ] Respects the existing architecture.
- [ ] Introduces no known regressions.
- [ ] Has adequate tests.
- [ ] Tests and validations ran successfully.
- [ ] Has no compilation or build errors.
- [ ] Has no lint or formatting problems.
- [ ] Introduces no known vulnerabilities.
- [ ] The interface was validated visually, where applicable.
- [ ] Responsiveness was validated, where applicable.
- [ ] Documentation is updated, where applicable.
- [ ] Compatibility is preserved or the changes are documented.
- [ ] Error handling is implemented.
- [ ] Performance was considered.
- [ ] The result was reviewed before delivery.

**The definitive principle: the goal is not simply to make ZORIX work. It is to build exceptional software: reliable,
secure, complete, sustainable, and able to compete worldwide with the best solutions on the market.**

---

## How ZORIX applies them

Where the project already enforces a section, this is where. A guideline with no enforcement here is listed under
[Known gaps](#known-gaps).

| Section | Where it is enforced |
|---|---|
| 1. GitHub | Issue and pull-request templates, [CODEOWNERS](../.github/CODEOWNERS), Dependabot, CI and release workflows, security policy ([SECURITY.md](../SECURITY.md)), [CHANGELOG](../CHANGELOG.md), [roadmap](roadmap.md). Merges are made locally with a merge commit, and `main` cannot be force-pushed or deleted |
| 2. Tests | `cargo test --workspace`, the interface tests and the live scripts in `tools/` ([testing](testing.md)). CI runs the Rust and desktop jobs on Linux, macOS and Windows, plus the Postgres tools |
| 3. Product | The [roadmap](roadmap.md) phases, each with acceptance criteria and a release |
| 4. UX/UI | The design system is the contract ([ADR-0032](adr/0032-design-system-v3-is-the-contract.md), [design-system.md](design-system.md)). `tools/check-design.mjs` and `tools/design-tokens.mjs` fail the build; Light and Dark screenshots are compared with the design |
| 5. Architecture | One job per crate ([architecture](architecture.md)); the contracts live in `zorix-core`; decisions are recorded as [ADRs](adr) |
| 6. Security | Least-privilege grants and approvals ([permissions](permissions.md), [ADR-0006](adr/0006-permissions-approvals.md), [ADR-0030](adr/0030-budgets-permissions-and-approvals-down-the-tree.md)), the [sandbox](sandbox.md), the autonomy rules ([ADR-0039](adr/0039-autonomy-is-decided-by-the-policy.md)), secret redaction, and [SECURITY.md](../SECURITY.md) |
| 7. Performance | Bounded sizes, counts and times on every tool and loop; limits are stated in each guide |
| 8. Errors | Small error enums with human sentences, limits and timeouts on every run, recovery of interrupted executions ([persistence](persistence.md)) |
| 9. Documentation | One guide per feature, `tools/check-docs.mjs` for links and anchors, ADRs, and the changelog |
| 10. Compatibility | Numbered database migrations with upgrade tests, a versioned event contract ([events](events.md)) |
| 11. Accessibility | Keyboard use, visible focus and an accessibility review for every screen. Every interface string is in the catalogue (English now, more languages in Phase 15) and a lint keeps it so; the validation errors that are still plain sentences are in [known gaps](#known-gaps) |
| 12. Observability | The event stream, usage and cost on every run, correlation by execution and parent ([events](events.md)) |
| 13. AI quality | Schema-validated tool arguments, limits narrowed down delegation trees, approvals bound to the exact call, [evaluations](evals.md), simulated providers with injected failures |
| 14. Dependencies | Lockfiles are committed, Dependabot is on, and CI audits the dependencies |
| 15. Process | [AGENTS.md](../AGENTS.md) and [CONTRIBUTING.md](../CONTRIBUTING.md): one branch per piece of work, small commits, the gates, docs with code |
| 16. Autonomy | Limitations are written in the guides and the roadmap, not hidden |
| 17. Approval | The [pull request template](../.github/PULL_REQUEST_TEMPLATE.md) carries the checklist |

Rules that decide how the guidelines are used in practice:

- A feature is designed with its tests, its documentation and its failure cases. It is not finished when the happy path
  works.
- A change to permissions, tools, agents, approvals or integrations gets a security review of its own, written in the
  change's ADR or guide.
- Work on the user's data is done only in a throwaway data folder (`ZORIX_HOME`), never in the user's own.
- If a guideline and a deadline conflict, the guideline wins.

## Known gaps

These guidelines ask for things ZORIX does not have yet. Each one is tracked, and none is hidden.

| Gap | Section | Tracked in |
|---|---|---|
| The end-to-end flows are driven only on Linux; macOS and Windows start the app and check that it loads | 2 | [#27](https://github.com/LuisPeixoto/zorix/issues/27) |
| GitHub Sponsors have not been evaluated | 1 | [#4](https://github.com/LuisPeixoto/zorix/issues/4) |
| About 170 validation errors are English sentences, not codes, so a translated interface shows them in English | 11 | [#26](https://github.com/LuisPeixoto/zorix/issues/26) |
| Long conversations are summarised within one run; the exchanges of earlier runs of a conversation are still dropped when the window is full | 7 | [#49](https://github.com/LuisPeixoto/zorix/issues/49) |
| The webhook settings are in `settings.json`, the rest of the program's settings in the database | 10 | [#50](https://github.com/LuisPeixoto/zorix/issues/50) |
| A failing telemetry export is not shown to the person who turned it on | 12 | [#51](https://github.com/LuisPeixoto/zorix/issues/51) |
