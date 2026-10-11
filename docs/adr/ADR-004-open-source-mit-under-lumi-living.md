# ADR-004: Open source, MIT, under the Lumi Living GitHub org

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

Scene goodwill, contributions (fixture images, format corrections), and the ability to study and port code from MIT-licensed references (retropixels, png2c64) argue for open source from the first commit. Lumi Living Oy already hosts several projects, so a new org would add bureaucracy.

## Decision

Public repository `lumi-living/convert64`, MIT licence, copyright Lumi Living Oy. Contributions are accepted under the same licence (inbound = outbound, stated in CONTRIBUTING.md). The `convert64` GitHub org name and `convert64.com/.app/.dev` were free on 2026-10-11; a domain is only needed if the browser build is published publicly.

## Consequences
- Easier: credits and attribution to reference tools are simple; contributors can test on real hardware we do not have.
- Harder: fixture images must have redistributable rights; the repo is visible while rough.
- Revisit when: the project outgrows the org (transfer with redirects is painless).
