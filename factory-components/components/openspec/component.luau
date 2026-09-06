--!strict
-- openspec component: groom, implement, and verify a named openspec
-- change. Convergence is this component's own loop over std.predicate
-- — prompt, judge, escalate to a human or fix, repeat, bounded by the
-- iteration cap — following the shared loop conventions documented in
-- ../README.md. See README.md in this directory for the declared
-- environment requirements.

local predicate = require("../../std/predicate")

export type Config = {
	-- Work agent handle driving the openspec skills.
	agent: Agent,
	-- Judge agent handle for the typed predicates (can be the same
	-- handle with a different model).
	judgeAgent: Agent,
	-- Optional per-session model config ids for work and judge sessions.
	model: string?,
	judgeModel: string?,
	-- Convergence cap shared by every operation (default 10).
	maxIterations: number?,
}

export type Instance = {
	-- Converge the change's proposal through review.
	groom: (self: Instance, change: string) -> string,
	-- Drive task execution to completion.
	implement: (self: Instance, change: string) -> string,
	-- Converge verification, then sync and archive the change in the
	-- same operation.
	verify: (self: Instance, change: string) -> string,
}

local DEFAULT_MAX_ITERATIONS = 10

local M = {}

function M.new(config: Config): Instance
	local maxIterations =
		if config.maxIterations ~= nil then config.maxIterations else DEFAULT_MAX_ITERATIONS
	local work: Agent = config.agent

	-- Prompt, judge the output, escalate or fix, repeat — bounded.
	-- The probe/human/resolve texts are per-operation data: each
	-- operation speaks its own terminal vocabulary (findings for
	-- groom/verify, the apply skill's pause states for implement) over
	-- the same skeleton.
	local function drive(
		sessionId: string,
		prompt: string,
		opts: {
			accepted: string,
			probePrompt: string,
			humanPredicate: string,
			resolvePrompt: string,
		}
	): string
		for iteration = 0, maxIterations - 1 do
			local header = `[{sessionId} iteration {iteration + 1} of {maxIterations}]\n`
			local session: Session = work:session({ id = `{sessionId}:{iteration}` })
			if config.model ~= nil then
				session:setConfig("model", config.model)
			end

			local outcome = session:prompt(header .. prompt)
			if predicate(opts.accepted, outcome.text, {
				agent = config.judgeAgent,
				model = config.judgeModel,
				sessionId = `{sessionId}-judge:{iteration}`,
			}) then
				session:close()
				return outcome.text
			end

			-- A judge-rejected pass probes for human input before any
			-- fix: if a human is needed the operation fails here.
			local probe = session:prompt(header .. opts.probePrompt)
			local needsHuman = predicate(opts.humanPredicate, probe.text, {
				agent = config.judgeAgent,
				model = config.judgeModel,
				sessionId = `{sessionId}-human:{iteration}`,
			})
			if needsHuman then
				session:close()
				local excerpt = string.gsub(string.sub(probe.text, 1, 200), "[\r\n]", " ")
				error(
					`{sessionId}: human input is required (iteration {iteration + 1} of {maxIterations}): {excerpt}`
				)
			end

			session:prompt(header .. opts.resolvePrompt)
			session:close()
		end
		return error(`{sessionId}: did not converge within {maxIterations} iterations`)
	end

	local instance: Instance = {
		groom = function(_self: Instance, change: string): string
			ptah.log(`openspec: reviewing change {change}`)
			local text = drive(
				"openspec-groom",
				`Review the openspec change named {change} using the openspec-review skill`,
				{
					accepted = "The review does not contain any blocker findings",
					probePrompt = "For the blocking findings, do you know how to resolve them, or do you need human input?",
					humanPredicate = "Human input is required to resolve the blocking findings",
					resolvePrompt = "Go ahead and resolve the blocking findings",
				}
			)
			ptah.log(`openspec: change {change} is ready to implement`)
			return text
		end,
		implement = function(_self: Instance, change: string): string
			ptah.log(`openspec: implementing change {change}`)
			local text = drive(
				"openspec-implement",
				`Implement the pending tasks of the openspec change named {change} using the openspec-apply-change skill. End each pass either with all tasks implemented or paused with a stated reason, as the skill defines those states.`,
				{
					accepted = "All tasks of the change are implemented",
					probePrompt = "For the pause: can you resolve it yourself (for example by updating the change's artifacts per the apply skill), or does it require human input to continue?",
					humanPredicate = "The pause requires human input to continue",
					resolvePrompt = "Go ahead and resolve the pause yourself, then continue implementing the remaining tasks",
				}
			)
			ptah.log(`openspec: change {change} is implemented`)
			return text
		end,
		verify = function(_self: Instance, change: string): string
			ptah.log(`openspec: verifying change {change}`)
			local text = drive(
				"openspec-verify",
				`Verify the implementation of the openspec change named {change} using the openspec-verify-change skill`,
				{
					accepted = "The verification does not contain critical findings or warnings",
					probePrompt = "For the critical findings and warnings, do you know how to resolve them, or do you need human input?",
					humanPredicate = "Human input is required to resolve the critical findings and warnings",
					resolvePrompt = "Go ahead and resolve the critical findings and warnings",
				}
			)
			-- The sync-and-archive step is part of the same operation: it
			-- runs once the verification loop has converged.
			local archiver: Session = work:session({ id = "openspec-archiver" })
			if config.model ~= nil then
				archiver:setConfig("model", config.model)
			end
			archiver:prompt(`Please sync and archive the change {change}`)
			archiver:close()
			ptah.log(`openspec: change {change} synced and archived`)
			return text
		end,
	}
	return instance
end

return M
