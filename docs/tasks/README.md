# Tasks for contributors, human or AI

Each file here is a self-contained brief: goal, the files to read, the interface to meet, how it is
accepted. They are written so that an assistant without repository access can work from the brief
plus the files it names, pasted or attached.

## How a task flows

1. **Pick a brief** that is not marked taken in its header. Say so in an issue or pull request title.
2. **Work from the brief and the files it names.** Do not widen the scope; if the brief is wrong,
   say what is wrong instead of guessing around it.
3. **Deliver a patch** against `main`: new and changed files in full, with the tests the brief asks
   for. For an AI assistant: the person driving it applies the patch on a branch and runs the checks
   in [CONTRIBUTING.md](../../CONTRIBUTING.md). Nothing goes to `main` without those checks passing.
4. **Report what was and was not tested**, in the pull request, in plain words. Fixture, or live.

## Rules that apply to every task

- Rust, pinned toolchain, `cargo fmt`, `clippy --all-targets -- -D warnings` with the workspace's
  pedantic set, tests that name the behaviour they prove.
- Match the prototype's output exactly where the brief says "byte for byte" or "same shape": the
  prototype is in `reference/python/` and is the specification.
- No new dependency without naming it, its licence and why the standard library will not do.
- Nothing private: no hostnames, addresses, tokens, unscrubbed captures.
- Plain writing. No placeholders, no dates in code comments, no hedges in docs.

## The briefs

| Task | What | Needs first | Size |
|---|---|---|---|
| [01](01-chat-formats.md) | Chat parsing and emote readers in `breakless-formats` | done | |
| [02](02-upstream-watch.md) | The upstream watcher as `breakless-tools upstream` | done | |
| [03](03-container-and-release.md) | Dockerfile to `scratch`, release workflow to GHCR | done; the workflow awaits its first tag | |
| [04](04-fake-twitch.md) | A fake Twitch for the service's tests | nothing | 1 to 2 days |
| [05](05-youtube-probe.md) | The YouTube live measurement probe | nothing | 1 to 2 days, plus runs on a home line |
| [06](06-docs-review.md) | Slop and accuracy review of every document | the others landing | half a day |

The maintainer's own work, not up for grabs: the service crate (sessions, exits, routes, chat rooms)
and the cutover.
