# Plan: Improve Review Pipeline Accuracy

## Overview

The AI code reviewer produces speculative and factually incorrect findings.
This plan addresses 4 root causes through 4 independent chantiers that merge
into a single integration branch `improve/review-pipeline-accuracy`.

## Branch Strategy

```
main
 └── improve/review-pipeline-accuracy  (integration)
      ├── fix/prompt-factual-guards     (#42) — Chantier 1
      ├── fix/chunk-by-file             (#43) — Chantier 2
      ├── feat/round2-verification      (#44) — Chantier 3
      └── feat/context-enrichment       (#45) — Chantier 4
```

Chantiers 1 & 2 are independent. Chantier 3 depends on 2 (file-aware chunking).
Chantier 4 depends on 3 (source.rs module).

Merge order: 1 → 2 → 3 → 4 → PR to main.

---

## Chantier 1: Prompt Factual Guards (#42)

**Files**: `crates/code-review-cli/src/review.rs`
**Branch**: `fix/prompt-factual-guards`

### Steps

1. Edit `SINGLE_ROUND_SYSTEM_PROMPT` and `SUMMARIZE_SYSTEM_PROMPT`:
   - Add factual verification requirement
   - Add anti-speculation instruction
   - Add "fewer findings is better than inflated findings"

2. Edit `SINGLE_ROUND_USER_TEMPLATE` and `SUMMARIZE_USER_TEMPLATE`:
   - Remove pre-filled `| 1 |` example row
   - Simplify YAML frontmatter (single total count)
   - Add rule: empty table is valid
   - Add rule: no assumptions about external APIs

3. Update all affected test assertions in `mod tests`

4. Run `cargo test && cargo clippy`

---

## Chantier 2: File-Aware Chunking (#43)

**Files**: `crates/code-review-cli/src/review.rs`
**Branch**: `fix/chunk-by-file`

### Steps

1. Add `split_diff_by_file(diff: &str) -> Vec<(String, String)>`:
   - Parse on `\n\n# File:` headers
   - Return Vec<(filename, file_diff)>

2. Add `group_files_into_chunks(files, max_words) -> Vec<Vec<...>>`:
   - Group files without splitting any file
   - Single file > max_words stays alone

3. Update `multi_round_review()` to use file-aware chunking

4. Add tests:
   - Multi-file split on boundaries
   - Large single file stays whole
   - Small files grouped efficiently

5. Run `cargo test && cargo clippy`

---

## Chantier 3: Round 2 Verification (#44)

**Files**: `crates/code-review-cli/src/source.rs` (new), `review.rs`
**Branch**: `feat/round2-verification`
**Depends on**: Chantier 2 (file-aware chunking for file list)

### Steps

1. Create `src/source.rs`:
   - `extract_modified_files(diff) -> Vec<String>`
   - `read_source_files(files) -> Vec<(String, String)>`
   - Skip files > 1000 lines, truncate with note

2. Add `mod source;` to `main.rs`

3. In `review.rs`:
   - Add `VERIFY_SYSTEM_PROMPT` (skeptical verifier role)
   - Add `VERIFY_USER_TEMPLATE` (CONFIRMED/REJECTED/DOWNGRADED classification)
   - Add `verify_findings()` function
   - Update `multi_round_review()`: Round 2 calls verify_findings()
   - Fallback to summarize_review() on failure

4. Lower summarize temperature from 0.3 to 0.1

5. Add tests for source.rs functions + verify prompt content

6. Run `cargo test && cargo clippy`

---

## Chantier 4: Context Enrichment (#45)

**Files**: `source.rs`, `review.rs`
**Branch**: `feat/context-enrichment`
**Depends on**: Chantier 3 (source.rs module)

### Steps

1. In `source.rs`:
   - Add `extract_function_context(source, diff, margin) -> String`
   - Large files: extract modified functions ± 50 lines
   - Small files (< 500 lines): include full content

2. Add context budget management:
   - `MAX_CONTEXT_CHARS` env var (default 100_000)
   - Include files smallest-first until budget reached
   - Log included/excluded files

3. Update `single_round_review()`:
   - Append source files section after diff
   - Add prompt instruction: source is for verification context only

4. Update `multi_round_review()` chunk prompts:
   - Include source for files in each chunk

5. Add tests for budget logic and function extraction

6. Run `cargo test && cargo clippy`

---

## Final Merge & PR

1. Merge branches into `improve/review-pipeline-accuracy` in order: 1 → 2 → 3 → 4
2. Run full test suite
3. Create PR to main with summary of all 4 issues
