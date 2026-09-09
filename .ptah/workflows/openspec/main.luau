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
local opsx = openspec.new({
	agent = piAgent,
	judgeAgent = piAgent,
	model = "zai/glm-5.3",
	judgeModel = "zai/glm-5.3-flash",
})

-- WORKFLOW
local function processChange(changeName: string)
	opsx:groom(changeName)
	-- opsx:implement(changeName, "task group 1")
	opsx:implement(changeName)
	opsx:verify(changeName)

	piAgent
		:session({
			id = "committer",
			resultSchema = { prUrl = { type = "string" } },
		})
		:prompt("Commit the uncommitted changes (except the .ptah directory) and push to remote")
end

opsx:verify("implement-task-scope")
piAgent
	:session({
		id = "committer",
		resultSchema = { prUrl = { type = "string" } },
	})
	:prompt("Commit the uncommitted changes (except the .ptah directory) and push to remote")

processChange("add-env-reads")
