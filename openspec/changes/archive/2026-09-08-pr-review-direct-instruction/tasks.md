## 1. Component

- [x] 1.1 Rename the `Config` field `reviewInstructionFile: string?` to `reviewInstruction: string?` in `factory-components/components/pr-review-loop/component.luau`; rewrite its doc comment to state the classification requirement (configured text must define what counts as blocking and direct blocking/non-blocking classification) and that a nil value selects the built-in default. Verify: `grep -r reviewInstructionFile factory-components/` returns nothing.
- [x] 1.2 Collapse the two-branch review ask into the single template — `local instruction = config.reviewInstruction or defaultInstruction` (Luau truthiness makes `or` exactly the nil-only fallback of Decision 1; empty string is truthy and stays configured) — wrapped as the default branch's "Use the following review instruction:\n\n{text}\n\n---\n\n…" with the blocking/non-blocking ask appended in both modes. Verify: reading the loop shows one review-ask template and no file branch.

## 2. Documentation

- [x] 2.1 Rewrite `factory-components/components/pr-review-loop/README.md`: contract section speaks "reviewer instruction" (text, not document) with the classification requirement; built-in-default section (nil selects it, configured text replaces it); environment requirements make the document bullet conditional ("any document your reviewer instruction references must exist and be readable by the agent"); the config example shows `reviewInstruction` with a pointer-pattern comment; a pointer-pattern paragraph presents text-referencing-a-document as the recommended long/pinned form and notes the per-iteration inlining trade. Verify: README uses the glossary terms from `CONTEXT.md` (Reviewer instruction; no "instruction document" as a config concept) and `default-instruction.luau`'s header comment already reads consistently.

## 3. Tests

- [x] 3.1 Flip `pr_review_loop_configured_instruction_wins_over_default` in `crates/ptah-cli/tests/factory_components.rs`: configure `reviewInstruction` with test-authored instruction text; assert the text is inlined in the echoed review prompt and the default's uppercase `BLOCKING` directive is absent. Verify: `cargo test --test factory_components pr_review_loop_configured` passes.
- [x] 3.2 Rename the field in the dryRun type-error fixture (`reviewInstructionFile = "x.md"` → `reviewInstruction = "x.md"`) and any other fixtures referencing the old field. Verify: `cargo test --test factory_components pr_review_loop_rejects` (type-error suite) passes.
- [x] 3.3 Run the offline suite end to end. Verify: `nix develop -c cargo test` passes with no `reviewInstructionFile` reference anywhere outside `openspec/changes/archive/`.
