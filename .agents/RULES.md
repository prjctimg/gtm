# Coding

- Never inline-import symbols with `crate::my_crate::SomeSymbol` paths inside expressions or function bodies; that is a code smell. Always resolve symbols via `use` statements at the top of the module.
- Avoid verbose or long variable and function names; rely on language features such as type inference, iterators, pattern matching, and RAII to keep names terse.
- Identifiers may have at most three underscore-separated segments (hard limit, e.g. `resolve_track_meta`); prefer composing types/structs over extending a name. External crate and standard library symbols are exempt.

# Rules

- Use terse commit message and never add any long explanations to the message. Respect the style used in the codebase.
- Respect the existing code before making any changes and follow existing conventions.
- Keep the code DRY and avoid repetitive patterns when a better terse alternative exists.
- Never add any session related comments to the code and the only documentation should be at the symbol level.
- Keep replies short and straight to the point. Drop all pleasantries and eager replies. Just do the task and give me summaries of the work done or prompt for clarification only.
- Do not run tests, clippy, or full `cargo build --workspace` locally; those are handled by CI. Use `cargo check` for quick local verification only.
- Do all work on the dev branch exclusively; only tagged commits are pushed to main.
- Always `git pull --rebase` new changes from the remote before beginning any session and before committing.
- Always use the question tool for user clarification instead of dumping all questions on the screen.

# Debugging

- Always read the log files before theorising about a bug, and before asking the user to describe a symptom. `$XDG_DATA_HOME/gtm/gtmd.log` (default `~/.local/share/gtm/gtmd.log`) for the daemon, `gtm.log` beside it for the client. Both are written at `info` by default; `-v` adds `debug`. Provider rejections, mixer events and IPC errors are recorded there and nowhere else.
- A bug report of "it does not work" is not a diagnosis. Reproduce it, grep the log, and report the lines that localise the fault. Never propose a fix from static reading alone when a log line would settle it.
- Confirm which binary the user is actually running before assuming a fix reached them: a released tag can be hundreds of commits behind `dev`, and a commit message containing `[skip ci]` suppresses every workflow for the whole push.
