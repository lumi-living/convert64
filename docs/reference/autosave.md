# Autosave and recovery files

The app saves the open project after every committed change, so a reload, a closed tab or a crash loses at most about a second of work, and offers it back on the next start (#241, usability finding C3). This page is the layout of what it writes. The code is `crates/convert64-app/src/autosave/` (`Autosave`, the `Storage` trait and its folder, memory, OPFS and IndexedDB stores).

## Where

| Build | Storage | Location |
|---|---|---|
| Native | one file per entry, written to `<name>.tmp` and renamed into place | `eframe::storage_dir("Convert64")/autosave/`: `~/Library/Application Support/Convert64/autosave/` on macOS, `~/.local/share/convert64/autosave/` on Linux, `%APPDATA%\Convert64\data\autosave\` on Windows. `CONVERT64_AUTOSAVE_DIR` overrides it. |
| Browser | the origin private file system (OPFS), one file per entry; a writable stream commits on `close`, so a failed write leaves the old file | folder `convert64-autosave` under `navigator.storage.getDirectory()` |
| Browser without OPFS (no `FileSystemFileHandle.createWritable`, or `getDirectory` refused, as in some private windows) | IndexedDB, one object store, one transaction per batch | database `convert64-autosave`, store `entries`, keyed by entry name |

`--screenshot` runs keep their autosave in memory and never touch these.

## Entries

The namespace is flat; anything else in it is ignored.

| Entry | Holds |
|---|---|
| `session-<id>.json` | one session's record (below) |
| `blob-<key>` | one source's bytes, exactly as loaded (the PNG, or another format the app decoded), or a PNG the app encoded for pixels a command made |

A **content key** is `<length in hex>-<FNV-1a 64><multiply–xorshift 64>` over the bytes, 128 bits of hash (`autosave::content_key`). It only has to tell one user's own sources apart, so it is not cryptographic. A source is written once, whatever its size: later autosaves of the same project write only the record.

## The record

```json
{
  "format": 1,
  "id": 1760793180123,
  "name": "title.png",
  "path": "/Users/me/art/title.convert64",
  "layers": 3,
  "modified_ms": 1760793301456.0,
  "project_key": "4f2-…",
  "saved_key": "4f2-…",
  "sources": ["e8f11-…", "57264-…"],
  "linked": { "e8f11-…": "/Users/me/art/title.png" },
  "project": "{ \"schema_version\": 3, … }"
}
```

| Field | Means |
|---|---|
| `format` | 1. A record of another format is skipped. |
| `id` | The session: the wall-clock ms when the document was opened. A restored session keeps its id. |
| `name`, `layers`, `modified_ms` | What the restore bar says: "Restore your last session (title.png, 3 layers, 2 min ago)?". |
| `path` | Native: the project file Save writes to after a restore. |
| `project` | The [project file](project-file.md) as JSON, schema version 3, with every layer's own source linked as `{"path": "blobs/<key>"}` (a layer sharing pixels stays `{"layer": n}`), masks embedded as in a saved project, and a `.vpl` palette written as its 16 colours. |
| `project_key` | The content key of `project`. |
| `saved_key` | `project_key` as it was at the last Save, or when an unchanged file was opened; absent if the session was never saved. |
| `sources` | The blob keys `project` links. |
| `linked` | Native: for a source that was linked from a file, that file. |

**Unsaved work** is a record whose `project_key` is not its `saved_key`. Undoing back to the saved state makes them equal again, so nothing is offered.

## When

- A change counts once it is committed: one command, or a drag on its release; never per frame.
- The autosave runs 1 s after the last committed change, and at most 5 s after the first one not yet saved (`DEBOUNCE_S`, `MAX_WAIT_S`). A Save rewrites the record at once, with the new `saved_key`.
- One batch at a time. A batch writes the new blobs first, then the record, then deletes; an interrupted batch never leaves a record without its blobs.

## Keeping and collecting

After each write the storage keeps the session just written, the newest other session and, while the restore bar shows it, the session on offer. Every other record is deleted, then every blob no kept record links. A blob left over from an interrupted batch is collected the next time anything is written.

## Restore

On start the newest record with unsaved work (and not the open session's own) is offered in a bar under the toolbar. **Restore** opens it as unsaved work, the title marked `*`, and the session goes on in the same record. Natively a source linked from a file that still holds the same bytes is linked again, so Save writes the project as it was; any other source is embedded. **Discard** deletes the record and its blobs. On native quit with unsaved changes the app asks; **Don't save** deletes the session's record, so it is not offered next time.

## Failure

A failed write (a full storage: `QuotaExceededError` in the browser, `StorageFull` natively; or any other error) shows one line in the status bar in the warning colour: "Autosave failed: storage is full. Save to keep your work." Work goes on. Nothing retries in a loop: the next committed change tries again, and a successful write clears the warning. A storage that cannot be read at all turns the autosave off for the session, with the same kind of warning.
