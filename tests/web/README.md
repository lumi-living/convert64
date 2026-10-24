# Browser smoke tests

Run by hand against a release bundle; they need Node 22 and Google Chrome, nothing else (no npm packages). Not a CI job.

```sh
(cd crates/convert64-app && trunk build --release)
node tests/web/restore-smoke.mjs            # OPFS; SHOTS=dir keeps screenshots, CHROME=path picks Chrome
STORAGE=idb node tests/web/restore-smoke.mjs  # the same with OPFS writes taken away: IndexedDB
```

- `restore-smoke.mjs` (#241): open the `title` fixture, nudge it, wait for the autosave, reload (the unsaved-changes guard must ask), Restore, check the nudge survived, then Save and check that a real download arrived holding the nudged project. Exit code 0 when every check passes.
- `pages-smoke.mjs` (#9): the GitHub Pages build under its project sub-path. Build with `(cd crates/convert64-app && trunk build --release --public-url /convert64/)`, then `node tests/web/pages-smoke.mjs [dist] [picture] [screenshot.png]`. It serves the bundle at `/convert64/` with Python's `http.server` (needs `python3`; `PORT` picks the port), opens it, drops the `title` fixture, and checks that the app starts, the picture opens, nothing throws and every request but the favicon succeeds, `solve-worker.js` and the worker's module fetches included. Rebuild without `--public-url` before running `restore-smoke.mjs`, which serves the bundle at `/`.
