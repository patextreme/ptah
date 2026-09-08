--!strict
-- Shim: openspec lifecycle (groom/implement/verify) over the shared
-- library (see factory-components/README.md for the consumption
-- model). The only workflow code this repo owns.
--
-- ptah scripts take no runtime arguments, so the op and change name
-- are hand-edited per use; one call active at a time.

local piAgent = ptah.agent("pi")

-- COMPONENTS
local openspec = require("../../../factory-components/components/openspec/component")
local prReview = require("../../../factory-components/components/pr-review-loop/component")
local opsx = openspec.new({
	agent = piAgent,
	judgeAgent = piAgent,
	model = "zai/glm-5.3",
	judgeModel = "zai/glm-5.3-flash",
})
local prr = prReview.new({
	agent = piAgent,
	judgeAgent = piAgent,
	model = "zai/glm-5.3",
	judgeModel = "zai/glm-5.3-flash",
	dryRun = false,
	maxIterations = 15,
})

-- WORKFLOW
local changeName = "pr-review-direct-instruction"

opsx:groom(changeName)
opsx:implement(changeName)
opsx:verify(changeName)

local session = piAgent:session({
	id = "committer",
	resultSchema = { prUrl = { type = "string" } },
})
session:prompt("Commit the uncommitted changes (except the .ptah directory) and push to remote")
local result = session:prompt("What is the PR url? Pass the url back to the ptah via result submit tool")
prr:review(result.result.prUrl :: string)
