# std

The repo-agnostic helper layer of Factory Components — transport, typed
judging, retry, and loop machinery. Knows nothing about any consumer
repo. See `../README.md` for the library contract and consumption model.

- `session-config.luau` — ordered session-config entries applied as
  `setConfig` calls in declared order
- `predicate.luau` — typed boolean judge
- `gh.luau` — GitHub CLI transport (`ptah.exec` + structured outcomes)
- `daemon.luau` — per-repo loop skeleton with error isolation
