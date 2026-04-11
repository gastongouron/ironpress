# AGENTS.md

## Code Organization Rules

- Keep code organized into small, thematic, reusable functions.
- Keep code organized into small, thematic files with a single clear responsibility.
- Do not allow files to grow beyond 500 lines.
- If a change would push a file beyond 500 lines, split the code into smaller modules first.
- Prefer reusable abstractions over copy-pasted logic and boilerplate.
- When several code paths share behavior, extract a named helper or module instead of duplicating the implementation.
- Apply these rules both to new code and to refactors of touched code.
