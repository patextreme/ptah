# Ptah workflows

Each workflow lives in its own directory under `.ptah/workflows/`.
The entrypoint is always named `main.luau`; workflow-specific instructions,
modules, and assets stay beside it.

```text
.ptah/workflows/<name>/main.luau
.ptah/workflows/<name>/instruction.md
.ptah/workflows/<name>/assets/…
```

Run or check a workflow by passing its entrypoint to ptah:

```sh
ptah check .ptah/workflows/<name>/main.luau
ptah run .ptah/workflows/<name>/main.luau
```

The shared Factory Components library is kept at `factory-components/` in this
repository. Workflow entrypoints reach it with a relative require such as
`require("../../../factory-components/components/<component>/component")`.
