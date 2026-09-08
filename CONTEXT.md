# ptah

A Luau-based CLI that drives ACP-speaking AI agents headlessly. This glossary
covers the ptah context: the CLI, the workflow scripts, and the shared
workflow library.

## Language

**Factory Components**:
The shared workflow library maintained in this repo, consumable by any
repository. Units of it are stdlib helpers and Components.
_Avoid_: shared-workflow, bricks, lego, software factory

**stdlib**:
The repo-agnostic helper layer of Factory Components — transport, typed
judging, retry, and loop machinery. Knows nothing about any consumer repo.
_Avoid_: utils, lib, common

**Component**:
A reusable workflow capability that consumers compose and configure rather
than fork — e.g. an openspec lifecycle or a PR review loop.
_Avoid_: template, plugin, module (module means any Luau file)

**Convergence loop**:
The core workflow pattern: prompt an agent, judge the result with a typed
predicate, and repeat with fixes until the predicate holds or the loop
escalates to a human.
_Avoid_: review loop, retry loop (those name specific uses of the pattern)

**Shim**:
The thin consumer-owned entry script that mounts Factory Components and hands
it Local config. The only workflow code a consumer repo owns.
_Avoid_: wrapper, bootstrap

**Local config**:
The data-only configuration table a consumer repo passes into a Component or
stdlib call. Functions are not configuration.
_Avoid_: settings, options file

**Task scope**:
The per-call description of which tasks an implement run is responsible
for. Completion — and the convergence loop's acceptance — is judged
against the scope, not against the whole change.
_Avoid_: filter (the component cannot see the tasks), instruction (a
scope redefines completion; an instruction does not)

**Reviewer instruction**:
The text that tells the work agent how to review — a configured instruction
in Local config, or the component's built-in default. A long or repo-pinned
one points at a versioned document rather than inlining text.
_Avoid_: instruction document, review instruction, prompt

**Mount point**:
The location in a consumer repo where the Factory Components tree is made
available (symlink, submodule, vendored copy). Library code only requires
within its own tree, so the mount point is the consumer's free choice.
_Avoid_: vendor dir (that is one mounting mechanism, not the concept)

**ptah's environment**:
The environment variables ptah inherits from its parent process, captured
once at startup. The read-only source behind `${VAR}` interpolation, agent
and shell-step inheritance, and script reads.
_Avoid_: the env, process env, environment config (that names the agent registry)
