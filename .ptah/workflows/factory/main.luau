--!strict
-- Factory workflow: the autonomous issue → reviewed PR loop, config only.
--
-- The whole orchestration — claiming the oldest eligible issue, provisioning
-- its `issue-<n>` worktree off `origin/<base>`, resolving the openspec change
-- or editing directly, driving groom → implement → verify, opening the
-- issue-linked PR (body carrying `Closes #<n>`), the convergent review-fix
-- loop, and the teardown — is the ptah-libs factory playbook
-- (`playbooks/factory/`; its README has the outcome taxonomy, the laws, and
-- the environment requirements). This shim is local config and nothing else.
--
-- Environment: the `pi` agent (see `.ptah/config.toml`) must resolve and
-- carry the openspec skills; `openspec`, `gh`, and `git` must be on PATH
-- (`gh` authenticated); `.ptah/worktree/` must stay git-ignored — it is the
-- worktrees' default home.

local libs = require("@ptah_libs")

-- ---------------------------------------------------------------------------
-- Local config
-- ---------------------------------------------------------------------------

-- The repo-configured queue label: a human's assertion that an issue is ready
-- for development. Required by the playbook, which never defaults it.
local QUEUE_LABEL = "ai-r4d"

-- The base every issue's worktree branches from and every PR targets
-- (required, no default).
local BASE = "main"

local MODEL = "ollama-cloud/deepseek-v4.1-flash"

-- Every session is configured in declared order (see Ptah Playbooks' session
-- config contract). All values here are strings, so no explicit entry
-- annotation is needed.
local workConfig = { { id = "model", value = MODEL } }

local factory = libs.factory.new({
	agent = ptah.agent("pi"),
	judgeAgent = ptah.agent("pi"),
	reporterAgent = ptah.agent("pi"),
	sessionConfig = workConfig,
	judgeSessionConfig = workConfig,
	reporterSessionConfig = workConfig,
	queueLabel = QUEUE_LABEL,
	base = BASE,
	-- Library defaults, kept visible as hand-edited knobs.
	maxReviewIterations = 10,
	resolveChangeAttempts = 3,
	openPullRequestAttempts = 3,
})

-- One issue end to end is `factory:issueToPR()` (or `factory:issueToPR(n)`
-- for a specific number); the drain loops it in scan mode until the queue
-- holds no eligible issue. A failed issue is logged and the loop moves on —
-- the claim marker stays as the audit trail a human re-queues by hand.
factory:drain()
