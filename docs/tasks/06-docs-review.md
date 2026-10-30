# Task 06: slop and accuracy review of every document

Status: open; best done after tasks 01 to 05 have landed, and again before 1.0.

## Goal

Read every Markdown file and every doc comment in the repository as a stranger would and fix what
fails the rules below. Report each fix as "file: what was wrong: what it says now".

## The rules

1. **A fact lives once.** The same explanation in two places: keep the better one, link the other.
2. **No placeholders.** Any `TODO`, `TBD`, `XXX`, all-caps token or "coming soon".
3. **Dates belong in the change log and in evidence.** A date in a code comment or in a README
   sentence that is not reporting a measurement moves to `CHANGELOG.md` or goes.
4. **Every number has a source.** A count, a size, a percentage or a "none seen" names the command,
   file or fixture that produced it, or goes.
5. **No hedges.** "Should", "probably", "may work", "might": either a tested statement or a line under
   what the project does not do.
6. **Comments say why.** Delete comments that restate the code; shorten the ones that carry a reason
   or a piece of evidence.
7. **Public text names no private thing.** No hostnames, container names assumed, paths on someone's
   machine, people.
8. **Claims match the code.** Every route, flag, environment variable, file name and command in a
   document exists and does what the document says. Check each one.
9. **Plain words.** No marketing adjectives, no exclamation marks, no emoji, one idea per sentence.

## Accepted when

- A pull request with the fixes and a list in its description, one line per fix.
- `grep -rn -E "TODO|TBD|XXX|FIXME|coming soon" --include=*.md --include=*.rs .` prints nothing
  outside this file.
- Every command in `README.md` and `CONTRIBUTING.md` was run by someone and did what it says.
