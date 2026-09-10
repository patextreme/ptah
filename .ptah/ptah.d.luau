--!strict
-- @meta
--
-- Type definitions for ptah scripts: the injected `ptah` global plus the
-- sandboxed environment the runtime provides. Load them into your editor's
-- luau-lsp (`ptah types > ptah.d.luau` — see the README "Editor setup"
-- section). This file is the single source of truth; the binary embeds it
-- verbatim. Written in plain `declare name: type` syntax (no `declare
-- class`, which newer Luau parsers reject); method-style fields take an
-- explicit `self` so colon calls (`s:prompt(…)`) type-check.

-- ---------------------------------------------------------------------------
-- Data tables
-- ---------------------------------------------------------------------------

-- Token counts reported for a completed turn (zero when unreported).
export type Usage = {
	input: number,
	cacheRead: number,
	cacheWrite: number,
	output: number,
}

-- The result of `session:prompt(text)`. `tostring(r)` returns `r.text` at
-- runtime via a metatable; definitions cannot express that sugar, so use
-- `r.text` explicitly when the type checker wants a string. `result` is
-- the turn's typed submission (see `SessionOptions.resultSchema`): a Luau
-- value converted from the accepted JSON, `nil` when the session declared
-- no contract or the turn had no accepted submission.
export type PromptResult = {
	text: string,
	stopReason: string,
	usage: Usage,
	result: any?,
}

export type EnvVariable = {
	name: string,
	value: string,
}

-- stdio MCP server (`mcpServers` entry, `{ type = "stdio", … }`).
export type McpServerStdio = {
	type: "stdio",
	name: string,
	command: string,
	args: { string }?,
	env: { EnvVariable }?,
}

-- http MCP server (`mcpServers` entry, `{ type = "http", … }`).
export type McpServerHttp = {
	type: "http",
	name: string,
	url: string,
	headers: { EnvVariable }?,
}

export type McpServer = McpServerStdio | McpServerHttp

-- Options for `agent:session(…)`. All fields optional; `id` defaults to
-- `s1, s2, …` per agent, `cwd` to the invocation directory. `resultSchema`
-- is a JSON Schema expressed as a plain Luau table: when set, the session
-- gets a typed-result contract — prompts carry a submit-tool instruction
-- and the outcome's `result` field carries the accepted submission.
-- (Session config is applied with `setConfig` calls after `session()`
-- returns — there is no `config` field; a `config` key is rejected at
-- runtime before any agent subprocess spawns.)
export type SessionOptions = {
	id: string?,
	cwd: string?,
	mcpServers: { McpServer }?,
	resultSchema: { [string]: any }?,
}

-- Options for `session:prompt(text, …)`.
export type PromptOptions = {
	timeoutMs: number?,
}

-- One choice of a select config option (`options` entry).
export type ConfigOptionChoice = {
	id: string,
	name: string,
	description: string?,
}

-- One entry of `session:configOptions()`. `type` discriminates the shape:
-- select options carry a string `currentValue` (the selected choice id)
-- and an `options` list of choices; boolean options carry a boolean
-- `currentValue` and no `options`. `category` is a UX hint only (nil when
-- the agent omits it) and must not be relied on.
export type ConfigOption = {
	id: string,
	name: string,
	type: "select" | "boolean",
	currentValue: string | boolean,
	category: string?,
	options: { ConfigOptionChoice }?,
}

-- Inline agent spec for `ptah.agent({ … })` (skips the registry).
export type AgentSpec = {
	command: string,
	args: { string }?,
	env: { [string]: string }?,
}

-- Options for `ptah.parallel(items, fn, …)`.
export type ParallelOptions = {
	concurrency: number?,
}

-- Options for `ptah.exec(cmd, …)`: `timeoutMs` absent or nil means no
-- budget (bounded only by outer cancellation).
export type ExecOptions = {
	timeoutMs: number?,
}

-- Result of `ptah.exec(cmd)`. Any exit code is data — `exitCode` is a
-- number even when the command failed (a signal death maps to the
-- shell's `128 + signal` convention); only could-not-run and timeout
-- raise.
export type ExecResult = {
	exitCode: number,
	stdout: string,
	stderr: string,
}

-- Options for `ptah.ask(…)`: the question to show (required) plus
-- optional context rendered as one indented line under the prompt.
-- v1 has no other options (no timeout: an ask blocks until answered,
-- aborted, or the run is cancelled).
export type AskOptions = {
	prompt: string,
	details: string?,
}

-- Result of `ptah.ask(opts)`: a discriminated union — narrow on
-- `action` (bind the result to a local first). `respond` carries the
-- answer as received, unprocessed (one line); `abort` is a normal
-- result the workflow handles like any other value, not an error, and
-- carries no `text`.
export type AskResult = {
	action: "respond",
	text: string,
} | {
	action: "abort",
}

-- Pure JSON module (`ptah.json`): turns captured command output into
-- script data and back. `parse` raises on malformed input; JSON `null`
-- decodes to nil. `stringify` rejects non-string table keys (arrays use
-- consecutive 1..n indexes).
export type Json = {
	parse: (s: string) -> any,
	stringify: (value: any, opts: { indent: number? }?) -> string,
}

-- Per-item outcome of `ptah.parallel` / `ptah.join`: narrow on
-- `entry.ok` (bind the entry to a local first) to reach `value` or `error`.
export type Outcome<T> = {
	ok: true,
	value: T,
} | {
	ok: false,
	error: string,
}

-- Handle returned by `ptah.spawn(fn)`; `task:await()` yields the task's
-- first return value or re-raises its error at the await site.
export type Task<T> = {
	await: (self: Task<T>) -> T,
}

-- Agent factory from `ptah.agent(name_or_spec)`.
export type Agent = {
	session: (self: Agent, opts: SessionOptions?) -> Session,
}

-- One live agent session (own subprocess). `label` is a method, not a
-- property: call it as `session:label()`.
export type Session = {
	prompt: (self: Session, text: string, opts: PromptOptions?) -> PromptResult,
	cancel: (self: Session) -> (),
	label: (self: Session) -> string,
	close: (self: Session) -> (),
	configOptions: (self: Session) -> { ConfigOption },
	setConfig: (self: Session, id: string, value: string | boolean) -> (),
}

-- ---------------------------------------------------------------------------
-- The `ptah` global
-- ---------------------------------------------------------------------------

declare ptah: {
	agent: (spec: string | AgentSpec) -> Agent,
	spawn: <T>(f: () -> T) -> Task<T>,
	join: <T>(tasks: { Task<T> }) -> { Outcome<T> },
	parallel: <T, U>(items: { T }, fn: (item: T) -> U, opts: ParallelOptions?) -> { Outcome<U> },
	exec: (cmd: string, opts: ExecOptions?) -> ExecResult,
	ask: (opts: AskOptions) -> AskResult,
	json: Json,
	sleep: (ms: number) -> (),
	log: (msg: string) -> (),
	exit: (code: number?) -> (),
	version: string,
}

-- ---------------------------------------------------------------------------
-- Sandbox shadows: the runtime curates the stdlib; these declarations mirror
-- exactly what survives, so editor-approved code cannot reach a global that
-- would fail at runtime.
-- ---------------------------------------------------------------------------

declare os: {
	time: (t: {
		year: number,
		month: number,
		day: number,
		hour: number,
		min: number,
		sec: number,
		isdst: boolean?,
	}?) -> number,
	clock: () -> number,
	getenv: (name: string) -> string?,
}

declare coroutine: {
	yield: (...any) -> ...any,
}

declare loadstring: nil
declare collectgarbage: nil
