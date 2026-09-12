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

The shared workflow library is [Ptah Playbooks](https://github.com/patextreme/ptah-libs),
consumed as the `ptah_libs` package (declared in `.ptah/pesde.toml`, installed
under `.ptah/luau_packages/`). Workflow entrypoints reach it through the
package alias:

```lua
local libs = require("@ptah_libs")
```
