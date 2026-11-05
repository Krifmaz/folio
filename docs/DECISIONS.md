# Decisions

Non-obvious calls, one line each, newest last.

- 2026-11-05 · Made from the kit template at v0.2.2 (the newest kit tag) with the Android app; the template's desktop app is left out, since the goal asks for the web and Android only.
- 2026-11-05 · Releases start at 0.0.1 (`first-version` in release.yml) so every merge ships a normal release before the goal's v0.1.0; the PR that gets a requested audiobook and ebook verified and filed end to end adds `docs/releases/0.1.0.md`, which makes that merge v0.1.0.
- 2026-11-05 · License GPL-3.0-only for Folio's own code (the kit stays MIT, which GPL allows); the original projects are credited in the README.
- 2026-11-05 · The first commit (goals and README) went to main directly, since a pull request needs a branch to merge into and the repository was empty.
- 2026-11-05 · Merged branches are deleted by the kit's cleanup-branches workflow (copied in), as the kit does, because this environment's proxy refuses ref deletes.
