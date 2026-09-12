# ptah

A Luau-based CLI that drives ACP-speaking AI agents headlessly. This glossary
covers the ptah context: the CLI, the workflow scripts, and Ptah Playbooks
(the shared workflow library).

## Language

**Ptah Playbooks**:
The shared Luau workflow library (`ptah-libs`, github.com/patextreme/ptah-libs):
repo-agnostic stdlib helpers and composable playbooks, consumed as the
`ptah_libs` package. Its vocabulary lives in that repository's `CONTEXT.md`.
_Avoid_: Factory Components (the retired name), shared-workflow, bricks, lego

**Package**:
A versioned distribution of Ptah Playbooks material (playbooks and/or
stdlib modules) installed by `ptah package` commands into a project. A
package carries playbooks; the CLI noun is package, never playbook.
_Avoid_: playbook (that names the capability inside), dependency, bundle

**Package alias**:
The `@name` a workflow uses to require an installed package. Synced by
`ptah package` commands into the project-root `.luaurc`, read by every
consumer (runtime, `ptah check`, editors) with standard Luau semantics.
_Avoid_: import, module name, package name (that is `scope/name` on the registry)

**Mount point**:
The location in a consumer repo where a workflow library is made available
(an installed package, or a source mount via symlink, submodule, or
vendored copy). Library code only requires within its own tree, so the
mount point is the consumer's free choice.
_Avoid_: vendor dir (that is one mounting mechanism, not the concept)

**Source definitions**:
The hand-maintained `.ptah/ptah.d.luau` in the ptah repo: the single source
of truth for script-API types, embedded into the binary at build time.
_Avoid_: the defs file, types file

**Embedded definitions**:
The definitions a ptah binary carries (compiled from the source definitions)
and emits via `ptah types` and `ptah init`, prefixed with a version header
identifying the emitting binary.
_Avoid_: bundled types

**Project definitions**:
A consumer project's `.ptah/ptah.d.luau`: a derived copy of the embedded
definitions. `ptah init` scaffolds the registry skeleton but syncs the
project definitions; it is never hand-edited.
_Avoid_: local defs, user definitions

**ptah's environment**:
The environment variables ptah inherits from its parent process, captured
once at startup. The read-only source behind `${VAR}` interpolation, agent
and shell-step inheritance, and script reads.
_Avoid_: the env, process env, environment config (that names the agent registry)
