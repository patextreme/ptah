--!strict
-- Session config (stdlib): an ordered array of config entries applied
-- to a session as `setConfig` calls in declared order — after session
-- creation, before the session's first prompt. The array is the
-- consumer's `setConfig` call sequence expressed as data; the order is
-- load-bearing for agents with dependent options (a `model` set can
-- re-derive other options), which is exactly the order a table cannot
-- carry.
--
-- One shared `apply` — the components and the typed judge call it so
-- application semantics cannot drift. No validation beyond the types:
-- nil/empty applies nothing, duplicate ids apply verbatim in order
-- (last wins, exactly as repeated runtime `setConfig` calls), and an
-- agent-rejected entry fails through the existing `setConfig` error
-- path — a catchable error carrying the option id and the agent's
-- message — with entries applied before the rejection left applied.

export type Entry = {
	id: string,
	value: string | boolean,
}

local M = {}

-- Apply `entries` to `session` in array order; nil/empty applies
-- nothing. Raises through `session:setConfig` when the agent rejects
-- an entry.
function M.apply(session: Session, entries: { Entry }?)
	if entries == nil then
		return
	end
	for _, entry in entries do
		session:setConfig(entry.id, entry.value)
	end
end

return M
