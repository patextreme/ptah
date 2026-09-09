--!strict
--
-- Runtime probe for `.ptah/ptah.d.luau`: exercises every member, method,
-- and field the definitions promise, against the mock agent. Wired as a
-- cargo test (tests/types.rs substitutes @MOCK_BIN@ with the mock binary)
-- and analyzed by the `ptah-analyze` nix check. If a member documented in
-- the definitions is removed or renamed in the runtime, this script fails.

-- ptah.agent(name) → agent:session(SessionOptions: id, cwd,
-- mcpServers, resultSchema)
local agent = ptah.agent("demo")
local servers: { McpServer } = {
	{
		type = "stdio",
		name = "ctx",
		command = "/bin/true",
		args = { "--flag" },
		env = { { name = "K", value = "V" } },
	},
	{
		type = "http",
		name = "remote",
		url = "http://127.0.0.1:1/mcp",
		headers = { { name = "H", value = "V" } },
	},
}
local s = agent:session({ id = "rev", cwd = ".", mcpServers = servers })

-- session:label() is a method (not a property) at runtime.
assert(s:label() == "demo/rev", "label: " .. s:label())

-- prompt result: text, stopReason, usage table (mock MOCK_USAGE=5,10,2,3),
-- prompt options: timeoutMs.
local r = s:prompt("review this", { timeoutMs = 5000 })
assert(r.text == "review this", "text: " .. r.text)
assert(r.stopReason == "end_turn", "stopReason: " .. r.stopReason)
assert(r.usage.input == 2, "usage.input: " .. tostring(r.usage.input))
assert(r.usage.output == 3, "usage.output: " .. tostring(r.usage.output))
assert(r.usage.cacheRead == 0, "usage.cacheRead: " .. tostring(r.usage.cacheRead))
assert(r.usage.cacheWrite == 0, "usage.cacheWrite: " .. tostring(r.usage.cacheWrite))
assert(tostring(r) == r.text, "tostring sugar: " .. tostring(r))

-- ptah.agent(AgentSpec: command, args, env) — registry-free.
local inline = ptah.agent({
	command = "@MOCK_BIN@",
	args = {},
	env = { MOCK_CHUNKS = "inline reply" },
})
local si = inline:session()
local ri = si:prompt("ignored")
assert(ri.text == "inline reply", "inline text: " .. ri.text)
si:close()

-- Typed results (SessionOptions.resultSchema / PromptResult.result): a
-- result session whose mock agent submits through the injected MCP
-- bridge; the outcome's `result` field carries the accepted submission
-- end to end.
local result_agent = ptah.agent({
	command = "@MOCK_BIN@",
	args = {},
	env = { MOCK_SUBMIT = '{"verdict":"approve"}' },
})
local sr = result_agent:session({
	resultSchema = {
		type = "object",
		properties = { verdict = { type = "string" } },
		required = { "verdict" },
	},
})
local rr = sr:prompt("review")
assert(rr.result ~= nil, "result submitted")
assert(rr.result.verdict == "approve", "result.verdict: " .. tostring(rr.result.verdict))
assert(rr.stopReason == "end_turn", "result stopReason: " .. rr.stopReason)
sr:close()

-- Per-session config (Session.configOptions / Session.setConfig): the
-- mock advertises a select `model` option plus a boolean `fast` option
-- and echoes the current model in each reply (MOCK_CONFIG_ECHO).
-- Configure with setConfig calls immediately after session creation,
-- before the first prompt (there is no constructor `config` option —
-- a `config` key is rejected pre-spawn; both value kinds: strings are
-- select choice ids, booleans are boolean options).
local cfg_agent = ptah.agent("demo-config")
local sc = cfg_agent:session()
sc:setConfig("model", "haiku")
local cfg = sc:configOptions()
assert(#cfg == 2, "configOptions count: " .. tostring(#cfg))
local model = cfg[1]
assert(model.id == "model", "model id: " .. model.id)
assert(model.name == "Model", "model name: " .. model.name)
assert(model.type == "select", "model type: " .. model.type)
assert(model.currentValue == "haiku", "model value: " .. tostring(model.currentValue))
assert(model.category == "model", "model category: " .. tostring(model.category))
local choices = model.options
assert(choices ~= nil and #choices == 2, "model choices")
assert(choices[1].id == "opus" and choices[1].name == "Opus", "choice 1")
assert(choices[2].id == "haiku" and choices[2].name == "Haiku", "choice 2")
local fast = cfg[2]
assert(fast.id == "fast", "fast id: " .. fast.id)
assert(fast.type == "boolean", "fast type: " .. fast.type)
assert(fast.currentValue == false, "fast value: " .. tostring(fast.currentValue))
assert(fast.options == nil, "boolean options must be nil")
assert(fast.category == nil, "absent category must be nil")

local rc = sc:prompt("go")
assert(rc.text == "haiku", "config echo: " .. rc.text)
sc:setConfig("model", "opus")
sc:setConfig("fast", true)
local cfg_after = sc:configOptions()
assert(cfg_after[1].currentValue == "opus", "model after: " .. tostring(cfg_after[1].currentValue))
assert(cfg_after[2].currentValue == true, "fast after: " .. tostring(cfg_after[2].currentValue))
sc:close()

-- session:cancel() on a hanging turn → stopReason == "cancelled".
local hang = ptah.agent("demo-hang")
local sh = hang:session()
local hanging = ptah.spawn(function()
	return sh:prompt("take your time").stopReason
end)
ptah.sleep(100)
sh:cancel()
local cancelled = hanging:await()
assert(cancelled == "cancelled", "cancelled: " .. tostring(cancelled))
sh:close()

-- ptah.spawn → Task:await().
local task = ptah.spawn(function()
	ptah.sleep(10)
	return 41 + 1
end)
local awaited = task:await()
assert(awaited == 42, "await: " .. tostring(awaited))

-- ptah.join: ok and error entries, in order.
local ok_task = ptah.spawn(function()
	return "joined"
end)
local bad_task = ptah.spawn(function()
	error("join boom", 0)
end)
local joined = ptah.join({ ok_task, bad_task })
local e1 = joined[1]
if e1.ok then
	assert(e1.value == "joined", "join value: " .. e1.value)
else
	error("join[1] should be ok: " .. e1.error)
end
local e2 = joined[2]
if e2.ok then
	error("join[2] should have failed: " .. tostring(e2.value))
else
	assert(e2.error:find("join boom", 1, true) ~= nil, "join error: " .. e2.error)
end

-- ptah.parallel with concurrency: ok and error entries, in item order.
local outcomes = ptah.parallel({ "alpha", "beta" }, function(item)
	if item == "beta" then
		error("bad item", 0)
	end
	return item:upper()
end, { concurrency = 2 })
assert(#outcomes == 2, "parallel count: " .. tostring(#outcomes))
local m1 = outcomes[1]
if m1.ok then
	assert(m1.value == "ALPHA", "parallel value: " .. m1.value)
else
	error("parallel[1] should be ok: " .. m1.error)
end
local m2 = outcomes[2]
if m2.ok then
	error("parallel[2] should have failed: " .. tostring(m2.value))
else
	assert(m2.error:find("bad item", 1, true) ~= nil, "parallel error: " .. m2.error)
end

-- ptah.exec + ptah.json (shell-exec capability): run a shell
-- command through the injected process runner, decode its captured
-- stdout as JSON, encode back with indentation. Nonzero exit is data,
-- not an error.
local ex = ptah.exec("printf '[{\"n\":1}]'")
assert(ex.exitCode == 0, "exec exit: " .. tostring(ex.exitCode))
assert(ex.stdout == '[{"n":1}]', "exec stdout: " .. ex.stdout)
local items = ptah.json.parse(ex.stdout)
assert(items[1].n == 1, "json parse: " .. tostring(items[1].n))
local encoded = ptah.json.stringify(items, { indent = 2 })
assert(encoded == '[\n  {\n    "n": 1\n  }\n]', "json stringify: " .. encoded)
local nonzero = ptah.exec("sh -c 'exit 4'")
assert(nonzero.exitCode == 4, "nonzero exit is data: " .. tostring(nonzero.exitCode))

-- os.getenv (environment reads): the startup snapshot — set, unset,
-- and set-to-empty are distinguishable (PTAH_ENV_* set by tests/types.rs).
assert(os.getenv("PTAH_ENV_PROBE") == "x", "env probe: " .. tostring(os.getenv("PTAH_ENV_PROBE")))
assert(os.getenv("PTAH_ENV_ABSENT") == nil, "env absent must be nil")
assert(os.getenv("PTAH_ENV_EMPTY") == "", "env empty must be the empty string")

-- ptah.log, ptah.version.
assert(type(ptah.version) == "string" and #ptah.version > 0, "version")
ptah.log("probe complete: " .. ptah.version)

s:close()
ptah.exit(0)
