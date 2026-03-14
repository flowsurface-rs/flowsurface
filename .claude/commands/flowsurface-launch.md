Launch the Flowsurface.app bundle on macOS via mise, automatically choosing the right build profile.

## Build profile decision

Read the recent conversation context and choose:

**`mise run run:fast`** (default — use this unless a release signal is present)

- Day-to-day iteration: UI changes, behavior checks, debugging
- No explicit release intent mentioned
- User just wants to see the result quickly
- ~35–40s build, opt-level=2, nearly identical runtime for a GPU-bound app

**`mise run run:app`** (full release — only when explicitly needed)

- User mentions: "release", "final build", "ship", "production", "benchmark", "performance test", "publish"
- Changes are performance-sensitive (rendering hot paths, indicator math, trade processing)
- Preparing a .app bundle to share or distribute

## Execution

1. State which profile you chose and why (one short sentence)
2. Run the chosen mise task in the background: `mise run run:fast` OR `mise run run:app`
3. Both tasks run preflight (SSH tunnel + ClickHouse + data validation) before building
4. Report success/failure when the background task completes
