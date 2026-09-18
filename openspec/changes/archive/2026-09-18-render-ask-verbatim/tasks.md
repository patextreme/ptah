## 1. Renderer: verbatim ask prose

- [x] 1.1 Rework ask rendering in `crates/ptah-render/src/lib.rs`: replace
      `ask_prompt_line`/`ask_details_line` (`prompt_preview`-based, one
      collapsed/truncated line each) with a multi-line builder that emits
      the prompt's first line on the label line (`{label}: {first}`), each
      further prompt line as a 2-space-indented line, each details line as
      a 2-space-indented line, leading and trailing blank lines trimmed,
      interior blank lines rendered empty (no indent padding), and a
      blank-after-trimming prompt rendering the label line with no text
      after its colon — no collapse, no `truncate_visible` on any ask
      line. Update `ask_requested` to emit the lines through the existing
      timestamped `ask_line` path with the `> ` cue last. Verify the
      renderer unit tests below pass.
- [x] 1.2 Update/replace the renderer unit tests that pinned the old shape:
      `ask_prompt_and_details_truncate_under_the_shared_budget` (becomes a
      renders-in-full/no-`…` test at >120 chars) and
      `ask_line_bodies_carry_attribution_and_action` (multi-line prompt no
      longer collapses). Add tests for: multi-line prompt (first line on
      label, continuations indented), details as indented block, interior
      blank line rendering empty, leading-blank-line trimming, a fully
      blank prompt rendering the bare label line, trailing-newline
      trimming, cue emitted after the last prose line. Verify with
      `cargo test -p ptah-render`.

## 2. Integration and gates

- [x] 2.1 Confirm no other surface pinned the old shape: run the full suite
      (`cargo test`) — the ask e2e tests (`crates/ptah-cli/tests/ask.rs`)
      and the examples suite use short prompts and assert on substrings
      (`ask 1 main.luau: Continue?`), which remain true verbatim; update
      any expectation that breaks. Verify the suite is green.
- [x] 2.2 Exercise the real binary against a long multi-line ask once by
      hand or via a scratch script under `.work/`: run
      `PTAH_ASK=stdin ptah run <script>` with a >120-char multi-line
      prompt plus multi-line details and confirm full prose, indent shape,
      empty interior blank lines, and the `> ` cue placement. Verify
      `cargo build` and `stylua --check` stay clean (no `.luau` touched in
      the repo — skip if none edited).
