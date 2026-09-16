--!strict
-- Factory workflow: the autonomous issue → reviewed PR loop.
--
-- Each iteration claims the oldest eligible issue (issue playbook), resolves
-- which existing openspec change it maps to, and drives that change through
-- the openspec lifecycle (groom → implement → verify, which syncs and
-- archives). When no change applies, an agent edits the repository directly
-- instead. Either way it then commits/pushes/opens a PR and runs the
-- convergent PR review loop on it. The loop stops when the queue holds no
-- eligible issue.
--
-- The issue playbook is agent-free (a claim marker plus earliest-wins
-- read-back); the openspec and pr playbooks own their own convergence and
-- escalation. This script owns only the orchestration between them, so a
-- single issue that fails (a missing human provider, a non-converging
-- review) is logged and the loop moves on — the claim marker stays as the
-- audit trail a human re-queues by hand.
--
-- Ptah scripts take no runtime arguments; the knobs below are hand-edited
-- per environment. The `pi` agent (see `.ptah/config.toml`) must carry the
-- openspec skills and `openspec` must be on PATH for the lifecycle steps,
-- and `gh` must be authenticated for the issue/PR steps.

local libs = require("@ptah_libs")

-- ---------------------------------------------------------------------------
-- Local config
-- ---------------------------------------------------------------------------

-- The repo-configured queue label: a human's assertion that an issue is ready
-- for development. Required by the issue playbook, which never defaults it.
local QUEUE_LABEL = "ai-r4d"

local MODEL = "ollama-cloud/deepseek-v4.1-flash"

-- Bounded retries for the typed-result hand-offs (resolve change, open PR):
-- agents may ignore the submit tool, so a nil result is designed degradation,
-- not a hard failure, until the attempts are exhausted.
local RESOLVE_ATTEMPTS = 3
local PR_ATTEMPTS = 3

local MAX_REVIEW_ITERATIONS = 10

-- Every session is configured in declared order (see Ptah Playbooks' session
-- config contract). All values here are strings, so no explicit entry
-- annotation is needed.
local workConfig = { { id = "model", value = MODEL } }

local work = ptah.agent("pi")
local judge = ptah.agent("pi")
local reporter = ptah.agent("pi")

local issueOps = libs.issue.new({
	queueLabel = QUEUE_LABEL,
})

local opsx = libs.openspec.new({
	agent = work,
	judgeAgent = judge,
	sessionConfig = workConfig,
	judgeSessionConfig = workConfig,
})

local reviewLoop = libs.pr.new({
	agent = work,
	judgeAgent = judge,
	reporterAgent = reporter,
	sessionConfig = workConfig,
	judgeSessionConfig = workConfig,
	reporterSessionConfig = workConfig,
	dryRun = false,
	maxIterations = MAX_REVIEW_ITERATIONS,
})

-- The pickup brief's shape, restated locally: the playbook's exported types
-- ride the generated package shim, which does not expose nested qualified
-- type names, so the structural shape is spelled out here.
type Brief = {
	number: number,
	url: string,
	title: string,
	body: string,
}

-- ---------------------------------------------------------------------------
-- Issue → change or direct edit
-- ---------------------------------------------------------------------------

-- An agent reads the claimed issue and identifies which *existing* openspec
-- change in `openspec/changes/` should be applied to resolve it. No change is
-- created here. An empty submitted name means no change applies: the issue is
-- a direct edit (see `directEdit` below).
local function resolveChange(brief: Brief): string?
	local session = work:session({
		id = ("factory-resolve:%d"):format(brief.number),
		resultSchema = {
			type = "object",
			properties = { change = { type = "string" } },
			required = { "change" },
		},
	})
	libs.std.sessionConfig.apply(session, workConfig)

	local prompt = table.concat({
		("Identify which existing openspec change should be applied to resolve GitHub issue #%d."):format(brief.number),
		"",
		("Title: %s"):format(brief.title),
		("URL: %s"):format(brief.url),
		"",
		"Body:",
		brief.body,
		"",
		"If the issue names an openspec change, use that name. Otherwise list the changes under `openspec/changes/` and pick the one whose scope matches this issue. Do not create a new change and do not implement anything.",
		'Submit that change\'s directory name through this session\'s ptah result tool as { "change": "<kebab-case-name>" }, or an empty string if no change applies and the issue is a direct edit.',
	}, "\n")

	local change: string? = nil
	local resolved = false
	for attempt = 1, RESOLVE_ATTEMPTS do
		local outcome = session:prompt(prompt)
		local result = outcome.result
		if type(result) == "table" and type(result.change) == "string" then
			change = if result.change == "" then nil else result.change
			resolved = true
			break
		end
		ptah.log(("factory: resolver submitted no change name (attempt %d/%d)"):format(attempt, RESOLVE_ATTEMPTS))
	end
	session:close()
	if not resolved then
		error(("factory: could not identify an openspec change for issue #%d"):format(brief.number))
	end
	return change
end

-- No openspec change applies: an agent implements the issue directly in the
-- repository. The PR review loop that follows is the convergence step.
local function directEdit(brief: Brief): nil
	local session = work:session({ id = ("factory-direct:%d"):format(brief.number) })
	libs.std.sessionConfig.apply(session, workConfig)

	local prompt = table.concat({
		("Implement GitHub issue #%d directly in this repository. No openspec change applies to it."):format(
			brief.number
		),
		"",
		("Title: %s"):format(brief.title),
		("URL: %s"):format(brief.url),
		"",
		"Body:",
		brief.body,
		"",
		"Make the edits the issue asks for, following the repository's conventions, and run the relevant format and test commands before finishing. Do not create an openspec change.",
	}, "\n")
	session:prompt(prompt)
	session:close()
	return nil
end

-- ---------------------------------------------------------------------------
-- Commit, push, open the PR
-- ---------------------------------------------------------------------------

-- The committer owns the git mechanics (branch, commit, push, `gh pr create`)
-- and returns the PR URL as a typed result. The branch follows this repo's
-- convention: `issue-<n>`.
local function openPullRequest(brief: Brief, change: string?): string
	local session = work:session({
		id = ("factory-pr:%d"):format(brief.number),
		resultSchema = {
			type = "object",
			properties = { prUrl = { type = "string" } },
			required = { "prUrl" },
		},
	})
	libs.std.sessionConfig.apply(session, workConfig)

	local summary = if change ~= nil
		then ("The openspec change `%s` for GitHub issue #%d is implemented, verified, synced, and archived."):format(
			change,
			brief.number
		)
		else ("The changes for GitHub issue #%d are implemented."):format(brief.number)
	local first = table.concat({
		summary,
		("Commit the uncommitted changes with a Conventional Commits message that references issue #%d, create or reuse the branch `issue-%d`, push it to the remote, and open a pull request against main using `gh`."):format(
			brief.number,
			brief.number
		),
		'Submit the pull request URL through this session\'s ptah result tool as { "prUrl": "https://github.com/..." }.',
	}, "\n")
	local retry =
		'Submit the pull request URL now through this session\'s ptah result tool as { "prUrl": "https://github.com/..." }. If the pull request does not exist yet, create it first.'

	local prUrl: string? = nil
	for attempt = 1, PR_ATTEMPTS do
		local outcome = session:prompt(if attempt == 1 then first else retry)
		local result = outcome.result
		if type(result) == "table" and type(result.prUrl) == "string" and result.prUrl ~= "" then
			prUrl = result.prUrl
			break
		end
		ptah.log(("factory: committer submitted no PR URL (attempt %d/%d)"):format(attempt, PR_ATTEMPTS))
	end
	session:close()
	if prUrl == nil then
		error(("factory: could not open a pull request for issue #%d"):format(brief.number))
	end
	return prUrl
end

-- ---------------------------------------------------------------------------
-- One issue, end to end
-- ---------------------------------------------------------------------------

local function runIssue(brief: Brief): nil
	local change = resolveChange(brief)
	if change ~= nil then
		ptah.log(("factory: issue #%d resolved to openspec change `%s`"):format(brief.number, change))
		-- skip gromming as spec is assumed to be ready before marking as r4d
		opsx:implement(change)
		opsx:verify(change)
		ptah.log(("factory: change `%s` implemented, verified, synced, and archived"):format(change))
	else
		ptah.log(("factory: issue #%d has no openspec change; implementing directly"):format(brief.number))
		directEdit(brief)
	end

	local prUrl = openPullRequest(brief, change)
	ptah.log(("factory: issue #%d pull request: %s"):format(brief.number, prUrl))

	local prReview = reviewLoop:review(prUrl)
	ptah.log(("factory: pr review loop %s: %s"):format(prReview.status, prReview.verdict))
	return nil
end

-- ---------------------------------------------------------------------------
-- The loop
-- ---------------------------------------------------------------------------

while true do
	local picked = issueOps:pickUp()
	if picked.status == "no-eligible-issue" then
		ptah.log(("factory: no eligible issue left (scanned %d); stopping"):format(picked.scanned))
		break
	end

	local brief = picked.brief
	ptah.log(("factory: claimed issue #%d — %s"):format(brief.number, brief.title))

	-- Per-issue error boundary: a failed issue is logged and left claimed
	-- (the marker is the audit trail), and the loop advances to the next one.
	local ok, failure = pcall(runIssue, brief)
	if not ok then
		ptah.log(("factory: issue #%d failed: %s"):format(brief.number, tostring(failure)))
	end
end
