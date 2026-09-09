--!strict
--
-- Environment knobs: `os.getenv(name)` reads a snapshot of ptah's
-- environment taken once when the run starts. It is the only env-read
-- surface — read-only, not enumerable. Unset variables read as `nil`,
-- so the `or` fallback is the idiom; an explicitly empty value reads
-- as `""`, which keeps "configured empty" distinguishable from "not
-- set". (PTAH_EXAMPLE_MODEL and PTAH_EXAMPLE_VERBOSE are set for the
-- test run; PTAH_EXAMPLE_REVIEWER deliberately is not.)
--
-- Demonstrates:
--   * the nil-default pattern: `os.getenv("X") or "fallback"`
--   * presence flags: compare against `nil`, not truthiness of a value
--   * unset vs set-to-empty staying distinguishable

-- A knob with a default: set for the run (haiku), unset would fall
-- back to sonnet.
local model: string = os.getenv("PTAH_EXAMPLE_MODEL") or "sonnet"

-- Presence flags: the value is irrelevant, only set-vs-unset matters.
local verbose: boolean = os.getenv("PTAH_EXAMPLE_VERBOSE") ~= nil

-- An unset knob reads as nil — the explicit-branch spelling of the
-- same fallback pattern.
local reviewer: string = os.getenv("PTAH_EXAMPLE_REVIEWER") or "second-opinion"

-- An explicitly empty value is "", not nil — callers can mean "off".
local extra: string = os.getenv("PTAH_EXAMPLE_EXTRA") or ""

assert(model == "haiku", "model: " .. model)
assert(verbose, "verbose flag must read as set")
assert(reviewer == "second-opinion", "reviewer: " .. reviewer)
assert(extra == "", "extra: " .. extra)

ptah.log(("knobs: model=%s verbose=%s reviewer=%s"):format(model, tostring(verbose), reviewer))
ptah.exit(0)
