--!strict
-- PR review workflow: the convergent review-fix loop over one pull request —
-- discovery review, typed judging, ledgered fix rounds, and a PR review report
-- posted on every terminal outcome. The whole orchestration is the ptah-libs
-- `pr` playbook (`:reviewFixLoop`; its README has the pass shape, the ledger,
-- the report contract, and the environment requirements). This shim is Local
-- config and the run-time input read, nothing else.
--
-- The pull request is selected at run time through the `PR_URL` environment
-- variable — ptah scripts take no runtime arguments:
--
--   PR_URL=https://github.com/<owner>/<repo>/pull/<n> \
--     ptah run .ptah/workflows/factory/pr-review/main.luau
--
-- Environment: the `pi` agent (see `.ptah/config.toml`) must carry the review
-- skills; `gh` must be authenticated (the loop reads/writes the ledger and
-- report comments and pushes fixes to the PR branch); the work session runs in
-- ptah's invocation directory, so run this from the PR's checkout (or a
-- worktree of it). One operation per PR at a time — the ledger comment is
-- edited in place.

local libs = require("@ptah_libs")

-- ---------------------------------------------------------------------------
-- Run input (ptah has no runtime arguments; PR_URL selects the PR)
-- ---------------------------------------------------------------------------

local prUrl: string = os.getenv("PR_URL")
	or error(
		"PR_URL is not set — run with PR_URL=https://github.com/<owner>/<repo>/pull/<n> ptah run .ptah/workflows/factory/pr-review/main.luau"
	)

-- ---------------------------------------------------------------------------
-- Local config
-- ---------------------------------------------------------------------------

local MODEL = "ollama-cloud/deepseek-v4.1-flash"

-- Every session is configured in declared order (see Ptah Playbooks' session
-- config contract). All values here are strings, so no explicit entry
-- annotation is needed.
local workConfig = { { id = "model", value = MODEL } }

local ops = libs.pr.new({
	agent = ptah.agent("pi"),
	judgeAgent = ptah.agent("pi"),
	reporterAgent = ptah.agent("pi"),
	sessionConfig = workConfig,
	judgeSessionConfig = workConfig,
	reporterSessionConfig = workConfig,
	-- Library defaults, kept visible as hand-edited knobs.
	maxIterations = 10,
})

local outcome = ops:reviewFixLoop(prUrl)

ptah.log("pr-review: " .. outcome.status)
ptah.log(outcome.report)
