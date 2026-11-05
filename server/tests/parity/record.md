# Recording what the original answers

Parity tests compare Folio with the pinned original (see `UPSTREAM.lock`), never with a guess.

1. Add one entry per request to `abs/requests.json`: `name` (the fixture's file name), `path`, and optionally
   `method`, `body`, `auth: false`, and `save` (`{"key": "dotted.path"}` keeps a value from the answer for later
   `{key}` placeholders). The recorder signs in as `root` / `root`, adds the library `{library}` on the test media
   (`abs/make_media.sh`) and scans it first.
2. Push the change in a PR. The `parity-record` workflow runs the original's image, replays every request, and
   uploads the answers and the database schema as the `abs-fixtures` artifact. You can also run it locally with
   Docker: start `ghcr.io/advplyr/audiobookshelf:<version>` with the media mounted at `/media`, then
   `python3 -I abs/record.py http://127.0.0.1:13378 OUT DB_PATH`.
3. Copy the fixtures you need into `fixtures/abs/` and write the test against them. Ids, tokens and times differ
   between runs: compare them by shape (same type, same links between objects), everything else exactly.
4. `{b64:key}` in a path is the saved value base64-encoded for a URL (library page filters such as `authors.<id>`).
   After the last request, the recorder writes every row of the original's database to `fixtures/_db_end.json`: a
   test can load those rows into Folio and replay the read-only requests at the end of `requests.json` on the
   same data (see `tests/parity_abs_library.rs`). The recording is also pushed to the branch `parity/<branch>`
   of a pull request, so `git fetch origin parity/<branch>` gets it.
