# Security policy

Convert64 is a desktop/CLI image converter. It reads image files you give it and writes Commodore 64 data files. It makes no network connections except the optional C64 Ultimate HTTP push (M3), which only talks to a host you configure.

## Supported versions

Only the latest release and the `main` branch receive fixes. There are no long-term support branches.

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

Use GitHub's private vulnerability reporting instead:

1. Go to <https://github.com/lumi-living/convert64/security/advisories/new>.
2. Describe the problem, how to reproduce it, and which version or commit you tested.
3. Attach a minimal input file if the issue is triggered by malformed image, project or C64 data.

You will get an acknowledgement within 7 days. Once a fix is ready we publish an advisory, credit you unless you prefer otherwise, and release a patched version.

## What counts

Things we consider security issues:

- Crashes, memory-safety bugs or unbounded allocation triggered by crafted input files (PNG/JPG/TIFF, `.convert64` projects, `.kla`/`.art`/`.spd`/`.ctm` imports).
- Path traversal or arbitrary file writes through project files or CLI arguments.
- Anything in the Tauri shell that lets web content reach the file system beyond what the user chose.
- Supply-chain problems in our build or release process.

Things that are not security issues: producing illegal VIC-II data (that is a bug, open a normal issue), or results that look wrong.
