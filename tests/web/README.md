# Browser smoke tests

Run by hand against a release bundle; they need Node 22 and Google Chrome, nothing else (no npm packages). Not a CI job.

```sh
(cd crates/convert64-app && trunk build --release)
node tests/web/restore-smoke.mjs            # OPFS; SHOTS=dir keeps screenshots, CHROME=path picks Chrome
STORAGE=idb node tests/web/restore-smoke.mjs  # the same with OPFS writes taken away: IndexedDB
RUNS=3 node tests/web/picker-timing.mjs      # the method picker's thumbnail timing, three opens
```

- `restore-smoke.mjs` (#241): open the `title` fixture, nudge it, wait for the autosave, reload (the unsaved-changes guard must ask), Restore, check the nudge survived, then Save and check that a real download arrived holding the nudged project. Exit code 0 when every check passes.
- `picker-timing.mjs` (#275): open the `title` fixture, open the method picker on its recipe and print the app's two timing lines: when every visible card first has its thumbnail and when all 90 are rendered (#260), with the time from the click to each and the load average. `RUNS=n` reopens it n times, changing the strength in between so the cache does not serve it. Exit code 0 when every open logged both lines.
