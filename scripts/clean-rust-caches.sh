#!/usr/bin/env bash
# clean-rust-caches.sh — reclaim per-user disk quota from stale Rust build
# artifacts across the developer's repos. This machine enforces a per-user disk
# QUOTA (Errno 122) separate from physical free space; Rust `target/` dirs (and
# especially incremental-compilation caches) are the recurring culprit that has
# blocked release cuts.
#
# Safe by default:
#   - incremental caches (`target/*/incremental`) are ALWAYS removed — they only
#     speed up rebuilds, cargo regenerates them, and they are the biggest chunk;
#   - a WHOLE `target/` dir is removed only when its repo is DORMANT
#     (unmodified > AGE_DAYS) — a dormant repo has no in-progress build to break;
#   - the cargo registry download cache + git checkouts are cleared (re-fetched);
#   - orphaned git worktrees are pruned, and leftover agent workflow worktrees
#     (`.claude/worktrees/*`) idle > AGE_DAYS are removed.
#
# It never removes a whole `target/` that changed within FRESH_HOURS (active work)
# and never touches source, git history, or the checked-out tree.
#
# Usage:
#   clean-rust-caches.sh [--dry-run] [--aggressive]
#     --aggressive  also drop incremental caches of ACTIVELY-built repos (the
#                   "I'm blocked on quota right now" manual escape hatch). The
#                   routine/cron path leaves actively-built repos untouched so it
#                   can never clobber an in-progress compile.
# Tunables (env):
#   WHIP_CLEAN_ROOTS         space-separated repo roots      (default "$HOME/code")
#   WHIP_CLEAN_TARGET_AGE_DAYS  dormant whole-target removal (default 21)
#   WHIP_CLEAN_FRESH_HOURS   "active" window, left alone     (default 6)
set -uo pipefail

DRY_RUN=0
AGGRESSIVE=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --aggressive) AGGRESSIVE=1 ;;
  esac
done

read -r -a ROOTS <<<"${WHIP_CLEAN_ROOTS:-$HOME/code}"
AGE_DAYS="${WHIP_CLEAN_TARGET_AGE_DAYS:-21}"
FRESH_HOURS="${WHIP_CLEAN_FRESH_HOURS:-6}"

log() { echo "[clean-rust-caches $(date -u +%H:%M:%SZ)] $*"; }
kb_of() { du -sk "$1" 2>/dev/null | awk '{print $1}'; }
reclaimed_kb=0
rm_reporting() { # $1 = path
  local path="$1" size
  [[ -e "$path" ]] || return 0
  size=$(kb_of "$path")
  if [[ $DRY_RUN == 1 ]]; then
    log "would remove ${size:-0}KB: $path"
  else
    rm -rf -- "$path" && reclaimed_kb=$((reclaimed_kb + ${size:-0})) &&
      log "removed ${size:-0}KB: $path"
  fi
}

# Most-recent-modification age of a dir, in whole minutes (0 if empty/missing).
dir_age_minutes() {
  local dir="$1" newest
  newest=$(find "$dir" -type f -printf '%T@\n' 2>/dev/null | sort -rn | head -1)
  [[ -z "$newest" ]] && { echo 999999999; return; }
  echo $(( ( $(date +%s) - ${newest%.*} ) / 60 ))
}

log "roots=${ROOTS[*]} age_days=$AGE_DAYS fresh_hours=$FRESH_HOURS dry_run=$DRY_RUN"

# 1) target/ dirs across the repo roots (depth-limited so we don't descend into
#    vendored/nested targets).
while IFS= read -r target; do
  [[ -d "$target" ]] || continue
  age_min=$(dir_age_minutes "$target")
  if (( age_min < FRESH_HOURS * 60 )); then
    # Actively-built repo: the routine path leaves it completely alone so an
    # in-progress compile is never clobbered. `--aggressive` still drops its
    # incremental caches (safe — cargo regenerates), for a manual quota rescue.
    if (( AGGRESSIVE == 1 )); then
      for inc in "$target"/*/incremental; do rm_reporting "$inc"; done
    else
      log "skip (active <${FRESH_HOURS}h, use --aggressive to reclaim): $target"
    fi
  elif (( age_min > AGE_DAYS * 24 * 60 )); then
    rm_reporting "$target"            # dormant repo: reclaim the whole target
  else
    for inc in "$target"/*/incremental; do rm_reporting "$inc"; done
  fi
done < <(for root in "${ROOTS[@]}"; do
  find "$root" -maxdepth 3 -type d -name target 2>/dev/null
done)

# 2) cargo registry download cache + git checkouts (re-fetched on next build).
rm_reporting "$HOME/.cargo/registry/cache"
rm_reporting "$HOME/.cargo/git/checkouts"

# 3) orphaned + stale git worktrees.
for root in "${ROOTS[@]}"; do
  while IFS= read -r gitdir; do
    repo="${gitdir%/.git}"
    if [[ $DRY_RUN == 1 ]]; then
      log "would: git -C $repo worktree prune"
    else
      git -C "$repo" worktree prune 2>/dev/null && log "pruned worktrees in $repo"
    fi
  done < <(find "$root" -maxdepth 2 -name .git -type d 2>/dev/null)
done
# Leftover agent workflow worktrees that outlived their run.
while IFS= read -r wt; do
  (( $(dir_age_minutes "$wt") > AGE_DAYS * 24 * 60 )) && rm_reporting "$wt"
done < <(find "${ROOTS[@]}" -maxdepth 4 -type d -path '*/.claude/worktrees/*' -prune 2>/dev/null)

log "done — reclaimed ~$(( reclaimed_kb / 1024 ))MB this run"
