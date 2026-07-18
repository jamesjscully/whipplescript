#!/usr/bin/env python3
"""Token-boundary alignment for whipplescript.

Objective (narrow): do LLM tokenizer breaks land on whip's lexical boundaries?
Specifically: does the brace-LESS block structure (rule/when headers, guards,
indentation) misalign more than the braced regions?

Metric:
  - HARMFUL STRADDLE: one tokenizer token spanning >=2 adjacent NON-whitespace
    whip lexemes (blurs a real boundary). Leading-whitespace absorbed into a
    keyword is NOT harmful and is tracked separately.
  - FRAGMENTATION: a structural whip lexeme (keyword / operator / delimiter)
    split across >1 tokenizer token (excluding a pure leading-ws token).
  - Reported per lexeme CLASS so we can compare braces vs brace-less block markers.
"""
import glob, os, re, sys, collections
import tiktoken

WHIP_KEYWORDS = set("""use workflow output failure class counter table as rule when on case after flow
complete fail decide coerce send read redact write invoke ask human agent tool
key cap reset daily weekly hourly result error input in out via from to of
if then else and or not is empty true false with settle suppose mark evidence why
gauge campaign improve channel message signal ingress import merge stream memory
script checkpoint restore fork park spend resume timeout completes fails times""".split())

# whip block/control markers that are BRACE-LESS (structure via keyword+indent)
BRACELESS_BLOCK = set("rule when on case after flow decide when guard".split())
BRACES = set("{}")
BRACKETS = set("[]")
MULTI_OPS = ["=>", "->", "::", "!=", "==", ">=", "<=", "|>", "<-", "&&", "||"]

# lexer: order matters
TOKEN_RE = re.compile(r"""
  (?P<comment>\#[^\n]*)
 |(?P<string>"(?:[^"\\]|\\.)*")
 |(?P<number>\d+(?:\.\d+)?)
 |(?P<mop>=>|->|::|!=|==|>=|<=|\|>|<-|&&|\|\|)
 |(?P<word>[A-Za-z_][A-Za-z0-9_]*)
 |(?P<punct>[{}\[\](),:.!<>=+\-*/@|])
 |(?P<nl>\n)
 |(?P<ws>[ \t]+)
""", re.VERBOSE)

def lex(src):
    """Yield (start, end, cls, text). cls in comment/string/number/mop/kw/ident/brace/bracket/punct/nl/ws."""
    out = []
    for m in TOKEN_RE.finditer(src):
        k = m.lastgroup; t = m.group()
        if k == "word":
            cls = "kw" if t in WHIP_KEYWORDS else "ident"
        elif k == "punct":
            cls = "brace" if t in BRACES else "bracket" if t in BRACKETS else "punct"
        elif k == "mop":
            cls = "mop"
        else:
            cls = k
        out.append((m.start(), m.end(), cls, t))
    return out

def analyze(path, encs):
    src = open(path, encoding="utf-8").read()
    lexemes = lex(src)
    # char -> lexeme index, and lexeme class per char
    char_lex = [-1] * len(src)
    for i, (s, e, cls, t) in enumerate(lexemes):
        for c in range(s, e):
            char_lex[c] = i
    ws_like = {"ws", "nl"}
    res = {}
    for name, enc in encs.items():
        ids = enc.encode(src)
        # rebuild char spans of tokens
        spans = []
        pos = 0
        for tid in ids:
            piece = enc.decode([tid])
            spans.append((pos, pos + len(piece)))
            pos += len(piece)
        harmful_straddle = 0
        straddle_examples = []
        total_tokens = len(ids)
        # fragmentation: count tokens overlapping each structural lexeme
        frag = collections.defaultdict(lambda: [0, 0])  # cls -> [n_lexemes, n_fragmented]
        lex_tokencount = collections.defaultdict(int)
        for (ts, te) in spans:
            # which non-ws lexemes does this token touch?
            touched = set()
            for c in range(ts, min(te, len(src))):
                li = char_lex[c]
                if li >= 0 and lexemes[li][2] not in ws_like:
                    touched.add(li)
            if len(touched) >= 2:
                harmful_straddle += 1
                if len(straddle_examples) < 12:
                    straddle_examples.append((repr(src[ts:te]),
                                              [lexemes[i][2]+":"+lexemes[i][3] for i in sorted(touched)]))
        # fragmentation per lexeme
        for i, (s, e, cls, t) in enumerate(lexemes):
            if cls in ("kw", "mop", "brace", "bracket", "ident", "punct"):
                # count tokenizer tokens that overlap [s,e)
                n = sum(1 for (ts, te) in spans if ts < e and te > s)
                frag[cls][0] += 1
                if n > 1:
                    frag[cls][1] += 1
        res[name] = dict(total_tokens=total_tokens, harmful_straddle=harmful_straddle,
                         straddle_examples=straddle_examples, frag=dict(frag),
                         n_lexemes=len(lexemes))
    return res

def main():
    files = sorted(glob.glob("examples/*.whip"))
    encs = {"o200k": tiktoken.get_encoding("o200k_base"),
            "cl100k": tiktoken.get_encoding("cl100k_base")}
    agg = {n: dict(total_tokens=0, harmful_straddle=0,
                   frag=collections.defaultdict(lambda: [0, 0]), n_lexemes=0) for n in encs}
    all_straddles = {n: [] for n in encs}
    for path in files:
        r = analyze(path, encs)
        for n in encs:
            agg[n]["total_tokens"] += r[n]["total_tokens"]
            agg[n]["harmful_straddle"] += r[n]["harmful_straddle"]
            agg[n]["n_lexemes"] += r[n]["n_lexemes"]
            for cls, (a, b) in r[n]["frag"].items():
                agg[n]["frag"][cls][0] += a
                agg[n]["frag"][cls][1] += b
            all_straddles[n].extend(r[n]["straddle_examples"])
    print(f"corpus: {len(files)} example files\n")
    for n in encs:
        a = agg[n]
        print(f"===== {n} =====")
        print(f"  total tokens:        {a['total_tokens']}")
        print(f"  total whip lexemes:  {a['n_lexemes']}")
        print(f"  HARMFUL straddles:   {a['harmful_straddle']}  "
              f"({100*a['harmful_straddle']/max(1,a['total_tokens']):.2f}% of tokens)")
        print(f"  fragmentation by lexeme class (fragmented / total, %):")
        for cls in ("kw", "mop", "brace", "bracket", "punct", "ident"):
            if cls in a["frag"]:
                tot, fr = a["frag"][cls][0], a["frag"][cls][1]
                print(f"      {cls:8s}: {fr:5d} / {tot:5d}  ({100*fr/max(1,tot):5.1f}%)")
        print("  sample harmful straddles:")
        seen = set()
        for tok, parts in all_straddles[n]:
            keyp = (tok, tuple(parts))
            if keyp in seen: continue
            seen.add(keyp)
            print(f"      {tok:20s} spans {parts}")
            if len(seen) >= 8: break
        print()

if __name__ == "__main__":
    main()
