--!strict
-- Ask a human mid-run and feed the answer into an agent turn.
--
-- Demonstrates: ptah.ask — the calling coroutine suspends while a human
-- answers; everything else (sessions, in-flight turns, other tasks)
-- keeps running. Answers arrive on stdin, so non-interactive runs pick
-- the provider explicitly: `PTAH_ASK=stdin ptah run examples/ask.luau`.

local answer = ptah.ask({
	prompt = "What should the reviewer focus on?",
	details = "The answer becomes the review prompt's focus line.",
})

local focus: string
if answer.action == "respond" then
	focus = answer.text
else
	-- abort: the human declined to steer — fall back to a default.
	focus = "anything worth flagging"
end

local agent = ptah.agent("demo")
local session = agent:session({ id = "reviewer" })
local r = session:prompt("Review this change, focusing on: " .. focus)
ptah.log("reviewer replied: " .. r.text)
session:close()
