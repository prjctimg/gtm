# Coding

- Never inline-import symbols with `crate::my_crate::SomeSymbol` paths inside expressions or function bodies; that is a code smell. Always resolve symbols via `use` statements at the top of the module.
- Avoid verbose or long variable and function names; rely on language features such as type inference, iterators, pattern matching, and RAII to keep names terse.

# Rules

- Use terse commit message and never add any long explanations to the message. Respect the style used in the codebase.
- Respect the existing code before making any changes and follow existing conventions.
- Keep the code DRY and avoid repetitive patterns when a better terse alternative exists.
- Never add any session related comments to the code and the only documentation should be at the symbol level.
- Keep replies short and straight to the point. Drop all pleasantries and eager replies. Just do the task and give me summaries of the work done or prompt for clarification only.
- Do not run tests or clippy locally; those are handled by CI. Verify changes with `cargo build --workspace` only.
- Do all work on the dev branch exclusively; only tagged commits are pushed to main.
- Always `git pull --rebase` new changes from the remote before beginning any session and before committing.
