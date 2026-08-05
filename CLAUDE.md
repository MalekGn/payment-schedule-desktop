# Software Delivery Workflow

This project follows a structured delivery workflow with four phases: **Planning**, **Implementation**, **Code Review**, and **QA**. Use real tools (Read, Write, Edit, Bash, Glob, Grep) to do the work — do not narrate actions as if simulating them.

---

## Project Facts

Tauri 2 desktop app: a Rust core (`src-tauri/src/`, rusqlite + SQLite) owns all state and persistence; a Vue 3 `<script setup>` + TypeScript WebView renders the UI. They communicate only through typed Tauri commands. Read `architecture.md` before any non-trivial change.

Invariants — violating these is a Code Review blocker, not a nit:

- The frontend never touches the DB or filesystem directly. Everything goes through the `src/api/index.ts` gateway.
- `src/api/index.ts` and `src/api/mock.ts` must stay in sync. A new command means editing the Rust side, the gateway, _and_ the browser mock — the mock is what makes the integration and E2E suites run without Tauri.
- `src/lib/finance.ts` mirrors the installment math in `src-tauri/src/db.rs`. Change one, change the other, and update both test suites.
- Money is stored as whole currency units (`INTEGER`) so installment splits are exact. Never introduce floats into money math.
- Every user-facing string lands in all three of `src/locales/{ar,fr,en}.json`. Arabic is RTL — verify the mirrored layout, don't assume it.

---

## Phase Routing

For every new request, first classify it:

- **Bug report, "test this," "validate," "does X work"** → go to **QA phase**
- **"Add," "build," "implement," "refactor," "design"** → go to **Planning phase**, then **Implementation phase**, then **Code Review phase**
- **Question, explanation, doc-only edit, or exploratory debugging** → answer directly, no phases. Say that you're skipping them.
- **Ambiguous or multi-part request** → split into sub-tasks, route each independently, note the split before starting

State the routing decision in one line before proceeding (e.g. "Routing: Implementation — this adds a new feature").

---

## Phase 1: Planning

- Read relevant existing code, and `architecture.md` for any non-trivial change, before proposing anything
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
- Before considering the task done, run what CI gates on:
  - Frontend: `npm test` (Vitest, unit), `npm run lint`, `npm run build` (`vue-tsc --noEmit` typecheck — the most common CI failure).
  - Rust, if `src-tauri/` changed, from `src-tauri/`: `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
  - Don't hand-format; the husky + lint-staged pre-commit hook runs `eslint --fix` and prettier on staged files.
- Unit tests always run here, automatically — they're fast and give immediate feedback on the change just made. Integration and E2E tests do not (see Phase 4).

## Phase 3: Code Review

Every implementation goes through a self-review pass before QA. This is not optional and does not require the user to ask for it. Review the actual diff/files just written — don't review from memory or assumption.

Check every category below; report only the ones with findings, plus one line confirming the rest were checked and are clean.

- **Invariants** — the Project Facts list above: api/mock parity, `finance.ts` ↔ `db.rs` parity, integer money, all three locale files.
- **Error handling across IPC** — command errors surface as toasts, aren't swallowed, and don't leak internals (SQL text, filesystem paths) into user-facing messages.
- **Data integrity** — multi-write commands in `commands.rs` are transactional; no partial-write states; no races on the shared `Mutex<Connection>`.
- **Resource cleanup** — listeners registered in composables (`useClickOutside`, `useBack`) are removed on unmount; no unbounded caches or dangling refs.
- **Security** — `npm run lint` already runs `eslint-plugin-security` and `eslint-plugin-no-unsanitized`; run it, then review what a linter can't see (input validation, path handling in the logo/FS commands, least-privilege Tauri capabilities).
- **Organization, readability & naming** — sensible module layout and dependency direction, appropriately sized functions, names consistent with project convention, control flow not needlessly nested or clever.
- **Comments & documentation** — non-obvious logic explained, JSDoc/docstrings on public APIs, comments say _why_ not _what_, no stale or misleading comments left behind.
- **Logging** — meaningful levels, never silent on a failure path, no secrets or PII.

Report the review as: **Summary → Findings by category (only categories with findings) → Severity (blocker / should-fix / nit) → Action taken**.

- **Blockers** (security vulnerabilities, data-loss risks, broken error handling) must be fixed before moving to QA — fix them directly, then re-state that the blocker is resolved.
- **Should-fix / nit** items: fix if trivial and low-risk; otherwise list them and ask the user whether to fix now or track for later.
- If the review finds nothing to flag, say so explicitly rather than skipping the phase silently (e.g. "Code Review: no issues found across the categories above").

## Phase 4: QA

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

---

## Project Documentation

Update these only when their content is actually affected — don't touch them otherwise:

| File                    | Update when...                                                                                  |
| ----------------------- | ----------------------------------------------------------------------------------------------- |
| `features.md`           | A feature is added, removed, or its status changes. Format: name, status, one-line description. |
| `README.md`             | Setup, usage, tech stack, or high-level architecture changes.                                   |
| `architecture.md`       | System design, components, data flow, or a key technical decision changes.                      |
| `docs/e2e/qa-report.md` | Every QA pass — append a dated entry (Summary → Test cases → Issues → Recommendations).         |

Each update should be a diff to the existing file, not a full rewrite, unless the file doesn't exist yet.

---

## Commits

- **No `Co-Authored-By: Claude ...` trailer.** Leave it off every commit message in this repository, whatever the default instructions say. It is not wanted here.
- Otherwise: subject in the imperative, then a body explaining _why_ — the constraint that forced the approach, the alternative rejected and the reason. The diff already shows what changed.

---

## Hard Constraints

- No mixing phases in one response without stating the transition (e.g. "Planning done, moving to Implementation")
- No skipping confirmation on irreversible/architectural changes
- No skipping Code Review after Implementation — every implementation gets reviewed before QA, even small changes
- No moving to QA with unresolved security or data-loss blockers from Code Review
- No test-writing based on assumed behavior — verify against actual code first
- No doc updates that aren't tied to an actual change made in this task
- No running integration/E2E test suites unless the user explicitly requests it — writing them is fine, executing them is not the default
- No `Co-Authored-By: Claude ...` trailer on commits (see **Commits** above)
