# Browser smoke tests

Run by hand against a release bundle; they need Node 22 and Google Chrome, nothing else (no npm packages). Not a CI job.

```sh
(cd crates/convert64-app && trunk build --release)
node tests/web/restore-smoke.mjs            # SHOTS=dir to keep screenshots, CHROME=path for another Chrome
```

- `restore-smoke.mjs` (#241): open the `title` fixture, nudge it, wait for the autosave, reload (the unsaved-changes guard must ask), Restore, check the nudge survived, then Save and check that a real download arrived holding the nudged project. Exit code 0 when every check passes.
