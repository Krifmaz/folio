# Upstreams

Folio is a port of [Audiobookshelf](https://github.com/advplyr/audiobookshelf) and
[Readarr](https://github.com/Readarr/Readarr) (both GPL-3.0), with Audiobookshelf's
[mobile app](https://github.com/advplyr/audiobookshelf-app). [UPSTREAM.lock](UPSTREAM.lock) records the exact
commit of each one. Every upstream is credited in the [README](README.md) and keeps its license notices.

| Name | Upstream | How Folio uses it |
| --- | --- | --- |
| `abs-web` | `client/` of advplyr/audiobookshelf | **Light fork** in `abs-web/`: Folio's web app, rethemed to the Encore look, talking to Folio's Audiobookshelf-compatible API. |
| `abs-app` | advplyr/audiobookshelf-app | **Light fork** in `abs-app/`: Folio's mobile app, rethemed the same way. |
| `abs-server` | `server/` of advplyr/audiobookshelf | **Reference** for the Rust port of the backend. Not imported. |
| `readarr` | `src/` of Readarr/Readarr | **Reference** for the Rust port of the wanted lists, profiles and import. Not imported. |

## Light forks

`scripts/upstream-merge.sh abs-web` (or `abs-app`) keeps a branch `upstream/<name>` whose commits are
*snapshots* of only that upstream directory, placed at our directory:

```text
advplyr/audiobookshelf ── A ─── B ─── C      (never fetched in full)
                          ┆     ┆     ┆      snapshot = client/ placed at abs-web/
upstream/abs-web ──────── a ─── b ─── c      (one commit per import)
                           ╲     ╲     ╲
main ───────────────────── M1 ── M2 ── M3    (normal merges)
```

1. The upstream commit is fetched shallowly into `.upstream/<name>.git` (git-ignored).
2. A snapshot commit holding only our directory is built there, with the previous snapshot as its parent, and
   fetched in as `upstream/<name>`.
3. It is merged into the current branch with a three-way merge, so only upstream's changes since the last import
   land on top of ours, and `UPSTREAM.lock` is updated.

Keep local changes in the forks small (theme, branding, the API address, a few hooks) so upstream fixes merge
cleanly. **A PR that imports or updates a fork is merged with a merge commit, never squashed**: the snapshot must
stay an ancestor of `main`, or the next import has no merge base.

```sh
git switch -c upstream-sync main
scripts/upstream-merge.sh abs-web                 # the pinned tag
scripts/upstream-merge.sh abs-web --ref v2.38.0   # or a newer tag, branch or commit
```

Exit status: `0` merged or already up to date; `1` conflicts (the merge is left in progress, files listed);
`2` usage or precondition error (nothing changed).

## Reference pins

`scripts/upstream-merge.sh abs-server` (or `readarr`) records the upstream commit in `UPSTREAM.lock` and commits
it, without importing anything. The port follows that commit: [docs/PARITY.md](docs/PARITY.md) links each row to
its source there. After a bump, run `scripts/parity-inventory.py` on fresh checkouts and update PARITY.md.
