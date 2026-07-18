# Tokenizer-alignment research note (whipplescript surface)

**Status: CLOSED 2026-07-17 (Jack signed off — "fine as-is is probably the right
answer"). Verdict: FINE AS-IS — no syntax change justified on tokenization-alignment
grounds.** The optional open-tokenizer spot-check was waived. Opened under the v0.2
milestone, Cluster C (`spec/v0.2-milestone-tracker.md`).

## Objective (narrow, set by Jack 2026-07-17)

Humans are a secondary audience; no preconceived notions about "what models
like." The single question: **do LLM tokenizer breaks land on whipplescript's
lexical boundaries — and specifically, does the brace-LESS block structure
(rule/when headers, guards, indentation) misalign more than the braced regions?**

Not the objective: token *count* (a weak proxy — programs are tiny vs. the
prompt; terseness the model gets wrong is worse than verbosity it gets right).
Aesthetics / human readability: explicitly out.

## What "misalignment" means (metric)

- **Harmful straddle** — one tokenizer token spanning ≥2 adjacent *non-whitespace*
  whip lexemes (blurs a real boundary). Leading whitespace absorbed into a keyword
  is NOT harmful (keyword stays crisp) and is excluded.
- **Fragmentation** — a *structural* lexeme (keyword / operator / brace / bracket /
  punct) split across >1 token. Reported per class so braces vs brace-less block
  markers are directly comparable.
- Identifier fragmentation is tracked but IGNORED for the verdict — it's
  author-chosen-name subword splitting, inherent to every language.

## Correction to the premise

whip is **hybrid, not brace-less**: bodies are brace-delimited
(`class {…}`, `rule … => {…}`, nested records), tables use `[…]`. The genuinely
brace-less regions are only the `rule <name>` / `when <Type> as <b>` headers,
guard expressions, and multi-char operators (`=>` `->` `:` `!`). So the question
narrows to: do *those* regions misalign? (They don't.)

## Result — 61-file corpus, o200k_base + cl100k_base

Fragmentation (fragmented / total):

| class | o200k | cl100k |
|---|---|---|
| braces `{}` | 0.0% (0/1356) | 0.0% |
| brackets `[]` | 0.0% (0/106) | 0.0% |
| punctuation | 0.0% (0/576) | 0.0% |
| multi-char ops | 0.4% (1/232) | 0.4% |
| keywords (brace-less markers) | 1.4% (22/1528) | 1.0% |
| identifiers | 22.4% | 22.3% *(ignored — names)* |

- Total tokens 19,230 (o200k) vs 19,232 (cl100k) — the two tokenizers agree to
  within a handful, so the result is not tokenizer-specific.
- "Harmful straddle" rate 2.78%, **but every straddle is a `.member` field-access
  merge** (`.coord`, `.status`, `.id`) — the benign, universal reading of member
  access, unrelated to braces/blocks. Not worth touching.
- **Braces (0%) and brace-less block markers (1.4%) both align cleanly** — no
  penalty for brace-lessness. Indentation-carried structure blurs zero lexeme
  boundaries.

## Caveats / what would change the verdict

1. OpenAI tokenizers only (o200k, cl100k). Claude's isn't public; no Llama/open
   tokenizer checked yet. The two BPE tokenizers agreeing this tightly strongly
   implies it generalizes — **one open-tokenizer (Llama-3-class) spot-check** is
   the sole optional follow-up to make it airtight.
2. Ground truth = a regex lexer (`scripts/tokenizer-alignment.py`), *exact* for structural
   tokens (braces/keywords/operators are unambiguous); only the ignored identifier
   class is fuzzy.

## Disposition

Cluster-C "tokenizer" lens = **answered: no action.** The remaining half of #3
(LLM-authorability via model priors / does the model emit correct whip) is a
*separate, larger* question and is NOT settled by this note — this note only
closes the token-boundary-alignment question Jack asked.
