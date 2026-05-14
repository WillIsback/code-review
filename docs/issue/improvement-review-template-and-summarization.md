---
tag: improvement
date: 2026-05-14
title: Code review template lacks clarity and final-pass summarization
---

## Problem

The current review pipeline in `crates/code-review-cli/src/review.rs` splits large PR diffs into
~2 000-word chunks, sends each to the LLM, then concatenates all results with `---` separators.
This produces output that:

- Repeats the same section headers ("Issues / Improvements / Positives") for every chunk
- Grows linearly with PR size — a 10-chunk PR yields 10 repetitive blocks
- Has no final synthesis: identical issues found across chunks appear multiple times
- Provides no ordering by severity or risk

The per-chunk system prompt (`crates/code-review-cli/src/review.rs:41-51`) is minimal and yields
unstructured free-text that is hard to scan quickly.

The vLLM client (`crates/toolkit-core/src/vllm.rs`) hardcodes `enable_thinking: false` with no
way to invoke a reasoning model, making a high-quality summarization pass impossible.

## Proposed solution

1. **Add `chat_complete_with_reasoning`** to `crates/toolkit-core/src/vllm.rs` — same signature as
   `chat_complete` but sets `enable_thinking: true`. Discards `reasoning_content`; returns `content`.

2. **Revise per-chunk prompt** to produce raw bullet lists (no markdown sections), keeping chunk
   calls fast and cheap.

3. **Add `summarize_review`** function in `crates/code-review-cli/src/review.rs` that feeds all
   chunk outputs into a single reasoning LLM call and instructs it to produce:

   ### Code Quality Issues
   Table sorted by pertinence (Critical → High → Medium → Low)

   ### Security Issues
   Table sorted by risk level (Critical → High → Medium → Low)

4. **`review_diff`** returns the summarized output instead of the raw concatenation.

## Acceptance criteria

- Single, concise review comment posted on the PR regardless of diff size
- Two-section structured output (quality + security), each sorted by severity
- Existing `chat_complete` call sites unchanged
- All existing tests pass; new unit tests cover `summarize_review` and the reasoning variant
