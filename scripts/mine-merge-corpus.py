#!/usr/bin/env python3
"""Mine real three-way text-merge cases from git history.

For every 2-parent merge commit in the given repos, find prose files BOTH
sides changed since their merge base and emit one JSONL case per file:

    {"repo", "commit", "file", "base", "ours", "theirs", "result"}

`result` is the HUMAN-ADMITTED merged body (the merge commit's tree) — the
reference the text-merge engine is scored against by
`examples/text_merge_eval.rs` (spec/text-merge-spec.md §10: zero bad
merges is the hard gate; spurious conflicts are the cost to minimize).

Usage: mine-merge-corpus.py OUT.jsonl REPO [REPO...]
"""

import json
import subprocess
import sys

PROSE_EXTENSIONS = (".md", ".markdown", ".txt", ".rst")
MAX_BYTES = 512 * 1024


def git(repo, *args, binary=False):
    out = subprocess.run(
        ["git", "-C", repo, *args], capture_output=True, check=False
    )
    if out.returncode != 0:
        return None
    return out.stdout if binary else out.stdout.decode("utf-8", "replace")


def blob(repo, rev, path):
    raw = git(repo, "show", f"{rev}:{path}", binary=True)
    if raw is None or len(raw) > MAX_BYTES or b"\x00" in raw:
        return None
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        return None


def changed(repo, base, tip):
    names = git(repo, "diff", "--name-only", "--diff-filter=M", base, tip)
    if names is None:
        return set()
    return {
        name
        for name in names.splitlines()
        if name.lower().endswith(PROSE_EXTENSIONS)
    }


def mine(repo, sink):
    merges = git(repo, "rev-list", "--merges", "--all")
    if not merges:
        return 0
    cases = 0
    for commit in merges.split():
        parents = (git(repo, "rev-list", "--parents", "-n", "1", commit) or "").split()
        if len(parents) != 3:  # commit + exactly two parents
            continue
        _, ours_rev, theirs_rev = parents
        base_rev = (git(repo, "merge-base", ours_rev, theirs_rev) or "").strip()
        if not base_rev:
            continue
        for path in sorted(changed(repo, base_rev, ours_rev) & changed(repo, base_rev, theirs_rev)):
            bodies = [blob(repo, rev, path) for rev in (base_rev, ours_rev, theirs_rev, commit)]
            if any(body is None for body in bodies):
                continue
            base, ours, theirs, result = bodies
            # Modify/modify only, with real divergence on both sides.
            if base == ours or base == theirs or ours == theirs:
                continue
            sink.write(
                json.dumps(
                    {
                        "repo": repo.rstrip("/").rsplit("/", 1)[-1],
                        "commit": commit[:12],
                        "file": path,
                        "base": base,
                        "ours": ours,
                        "theirs": theirs,
                        "result": result,
                    }
                )
                + "\n"
            )
            cases += 1
    return cases


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    out_path, repos = sys.argv[1], sys.argv[2:]
    total = 0
    with open(out_path, "w", encoding="utf-8") as sink:
        for repo in repos:
            count = mine(repo, sink)
            print(f"{repo}: {count} cases")
            total += count
    print(f"total: {total} cases -> {out_path}")


if __name__ == "__main__":
    main()
