--!strict
-- PR review loop component: run a review→fix→push convergence against a
-- pull request, generalized from identus-ws's pr-review-loop — repo
-- specifics (reviewer instruction document, dry-run gating) arrive as
-- config, with the built-in default instruction standing in when no
-- document is configured, and the PR URL is the sole repository
-- context. The loop is this component's own policy
-- over std.predicate (following the shared conventions in ../README.md):
-- escalation probes blocking findings with a subagent before asking,
-- each fix is followed by a commit-and-push prompt, and the converged
-- session posts the verdict comment.

local predicate = require("../../std/predicate")
local defaultInstruction = require("./default-instruction")

export type Config = {
	-- Work agent handle reviewing and fixing the PR.
	agent: Agent,
	-- Judge agent handle for the typed predicates.
	judgeAgent: Agent,
	-- Optional per-session model config ids for work and judge sessions.
	model: string?,
	judgeModel: string?,
	-- Optional repository-relative path of a reviewer instruction
	-- document. A configured document must define what counts as a
	-- blocking issue for this repository and instruct the reviewer to
	-- classify findings as blocking or non-blocking — the instruction
	-- contract (see the README): the judge predicates and fix prompts
	-- below speak that fixed vocabulary. When omitted, reviews run
	-- against the built-in default instruction; a configured document
	-- wins.
	reviewInstructionFile: string?,
	-- When true the commit/push step is skipped: the loop converges on
	-- reviews and fixes but never pushes to the PR branch.
	dryRun: boolean?,
	-- Convergence cap (default 15).
	maxIterations: number?,
}

export type Instance = {
	-- Run the review loop against one pull request (the PR URL is
	-- per-call data, not config).
	review: (self: Instance, prUrl: string) -> string,
}

local DEFAULT_MAX_ITERATIONS = 15

local M = {}

function M.new(config: Config): Instance
	local maxIterations = if config.maxIterations ~= nil then config.maxIterations else DEFAULT_MAX_ITERATIONS
	local dryRun = config.dryRun == true
	local work: Agent = config.agent

	local instance: Instance = {
		review = function(_self: Instance, prUrl: string): string
			local push: string? = if dryRun
				then nil
				else "Commit the changes and push them to the PR branch so the next review sees them. Also comment on the PR with the review feedback."
			if dryRun then
				ptah.log(`pr-review-loop: dry run — fixes are not pushed to {prUrl}`)
			end

			for iteration = 0, maxIterations - 1 do
				local header = `[pr-review iteration {iteration + 1} of {maxIterations}]\n`
				local session: Session = work:session({ id = `pr-review:{iteration}` })
				if config.model ~= nil then
					session:setConfig("model", config.model)
				end

				-- The instruction branch: a configured document wins over the
				-- built-in default. The classification ask stays in both
				-- branches — enforcement for instruction documents that
				-- under-specify the output format (redundant with a compliant
				-- document by design).
				local reviewAsk: string
				if config.reviewInstructionFile ~= nil then
					reviewAsk =
						`Use the review instruction at {config.reviewInstructionFile} to review PR {prUrl}. Based on the review feedback, determine the blocking issues and the non-blocking issues.`
				else
					reviewAsk =
						`Use the following review instruction:\n\n{defaultInstruction}\n\n---\n\nUsing this instruction, review PR {prUrl}. Based on the review feedback, determine the blocking issues and the non-blocking issues.`
				end
				local outcome = session:prompt(header .. reviewAsk)
				if
					predicate("The review verdict does not contain blocking issues", outcome.text, {
						agent = config.judgeAgent,
						model = config.judgeModel,
						sessionId = `pr-review-judge:{iteration}`,
					})
				then
					-- The converged session posts the verdict comment.
					session:prompt(header .. "Please comment on the PR with the review feedback along with the verdict")
					session:close()
					return outcome.text
				end

				-- Escalation probes the blocking findings with a subagent
				-- before asking for a human; if a human is needed the
				-- loop fails without issuing the fix.
				local probe = session:prompt(
					header
						.. "For each of the blocking issues, validate with a subagent whether the issue is real. For the blocking issues that remain, do you know how to resolve them, or do you need human input?"
				)
				local needsHuman = predicate("Human input is required to resolve the blocking issues", probe.text, {
					agent = config.judgeAgent,
					model = config.judgeModel,
					sessionId = `pr-review-human:{iteration}`,
				})
				if needsHuman then
					session:close()
					error(
						`pr-review: human input is required to resolve the findings (iteration {iteration + 1} of {maxIterations})`
					)
				end

				local fix = session:prompt(header .. "Go ahead and resolve the blocking issues")
				ptah.log(`pr-review iteration {iteration + 1}: findings judged fixable — fix result: {fix.text}`)
				if push ~= nil then
					session:prompt(header .. push)
				end
				session:close()
			end
			return error(`pr-review: did not converge within {maxIterations} iterations`)
		end,
	}
	return instance
end

return M
