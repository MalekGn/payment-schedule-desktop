# Software Delivery Workflow

This project follows a structured delivery workflow with three phases: **Planning**, **Implementation**, and **QA**. Use real tools (Read, Write, Edit, Bash, Glob, Grep) to do the work — do not narrate actions as if simulating them.

---

## Phase Routing

For every new request, first classify it:

- **Bug report, "test this," "validate," "does X work"** → go to **QA phase**
- **"Add," "build," "implement," "refactor," "design"** → go to **Planning phase**, then **Implementation phase**
- **Ambiguous or multi-part request** → split into sub-tasks, route each independently, note the split before starting

State the routing decision in one line before proceeding (e.g. "Routing: Implementation — this adds a new feature").

---

## Phase 1: Planning

- Read relevant existing code and `architecture.md` (if present) before proposing anything
- Summarize the requirement in 2-3 sentences
- List any real ambiguities that would change the implementation approach — ask about these; don't ask about things with a reasonable default
- Propose the approach in a few bullet points (components touched, new files, key decisions)
- **Stop and confirm with the user only if** the change: introduces a new dependency, alters the architecture, deletes/renames existing files or public APIs, or is otherwise hard to reverse. Otherwise proceed straight to implementation.

## Phase 2: Implementation

- Write production-quality code directly to the relevant files using the appropriate tools
- Follow existing project conventions (language, style, folder structure) — check for them before assuming a stack
- Include unit tests alongside the feature (co-located per project convention, or in the existing test directory)
- Document new/changed APIs inline (docstrings/JSDoc) and in an OpenAPI/Swagger file if the project already has one
- Update project docs (see below) if the change affects them
- Run the test suite / linter if one exists in the project before considering the task done

## Phase 3: QA

- Base test scenarios strictly on the actual implemented behavior — read the code, don't assume
- Write integration and/or end-to-end tests as appropriate to the change
- **Do not execute** integration/E2E tests automatically. Write them and stop there, unless the user explicitly asks to run them (e.g. "run the tests," "validate end-to-end," "verify this is ready to ship").
- When execution is skipped, say so explicitly (e.g. "Tests written but not run — say the word if you want me to execute them.")
- Provide reproducible steps for any bug found
- Flag edge cases and risks explicitly, even ones not covered by tests
- Report findings as: Summary → Test cases run (or "not run — awaiting confirmation") → Issues found → Recommendations
- **Generate or update a QA report** at `docs/e2e/qa-report.md` on every QA pass. Use the same Summary → Test cases → Issues → Recommendations structure, date each entry, and append a new dated section rather than discarding prior history. This is the durable record of what was tested and what remains open; the terminal summary is a mirror of it, not a replacement.

Integration/E2E test layout for this project:
- **Unit tests** live co-located in `src/**` and run with `npm test` (Vitest).
- **Integration tests** live in `tests/integration/**` and run only via `npm run test:integration` (separate `vitest.integration.config.ts`) — kept out of the default unit run so they stay opt-in per the constraint below.
- **End-to-end tests** live in `tests/e2e/` (`run.mjs`, Playwright) and run via `npm run test:e2e`; failure screenshots land in `tests/e2e/artifacts/`. The QA report itself stays under `docs/e2e/qa-report.md` (it is documentation, not test code).

**Unit tests are the exception** — always run these automatically as part of Implementation (Phase 2), since they're fast and give immediate feedback on the change just made.

---

## Project Documentation

Update these only when their content is actually affected — don't touch them otherwise:

| File | Update when... |
|---|---|
| `features.md` | A feature is added, removed, or its status changes. Format: name, status, one-line description. |
| `README.md` | Setup, usage, tech stack, or high-level architecture changes. |
| `architecture.md` | System design, components, data flow, or a key technical decision changes. |
| `docs/e2e/qa-report.md` | Every QA pass — append a dated entry (Summary → Test cases → Issues → Recommendations). Created if absent. |

Each update should be a diff to the existing file, not a full rewrite, unless the file doesn't exist yet.

---

## Hard Constraints

- No mixing phases in one response without stating the transition (e.g. "Planning done, moving to Implementation")
- No skipping confirmation on irreversible/architectural changes
- No test-writing based on assumed behavior — verify against actual code first
- No doc updates that aren't tied to an actual change made in this task
- No running integration/E2E test suites unless the user explicitly requests it — writing them is fine, executing them is not the default
