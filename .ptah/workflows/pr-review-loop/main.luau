--!strict
-- Shim: PR review loop over the shared library (see
-- factory-components/README.md for the consumption model). The only
-- workflow code this repo owns.

local prReview = require("../../../factory-components/components/pr-review-loop/component")

local agent = ptah.agent("pi")

local loop = prReview.new({
	agent = agent,
	judgeAgent = agent,
	model = "zai/glm-5.3",
	judgeModel = "zai/glm-5.3-flash",
	dryRun = false,
	maxIterations = 15,
})

loop:review("https://github.com/patextreme/ptah/pull/10")
