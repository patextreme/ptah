--!strict
-- Shim: PR review loop over Ptah Playbooks (the shared library,
-- consumed as the `ptah_libs` package; see
-- https://github.com/patextreme/ptah-libs for the consumption model).
-- The only workflow code this repo owns.

local libs = require("@ptah_libs")

local agent = ptah.agent("pi")

local loop = libs.prReviewLoop.new({
	agent = agent,
	judgeAgent = agent,
	sessionConfig = { { id = "model", value = "zai/glm-5.3" } },
	judgeSessionConfig = { { id = "model", value = "zai/glm-5.3-flash" } },
	dryRun = false,
	maxIterations = 15,
})

loop:review("https://github.com/patextreme/ptah/pull/10")
