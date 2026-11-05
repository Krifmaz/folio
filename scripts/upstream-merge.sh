#!/usr/bin/env bash
# Merge a pinned upstream app into this repo as a light fork (see UPSTREAM.md).
#
# Upstreams (name: repo, upstream directory -> our directory):
#   abs-web  advplyr/audiobookshelf      client/ -> abs-web/   (the web frontend)
#   abs-app  advplyr/audiobookshelf-app  (root)  -> abs-app/   (the mobile app)
# Reference pins (recorded in UPSTREAM.lock only, never imported; the backends are ported to Rust):
#   abs-server  advplyr/audiobookshelf   (server/ is the reference for the port)
#   readarr     Readarr/Readarr          (src/ is the reference for the port)
#
# Mechanism, per upstream NAME:
#   1. Shallow-fetch the requested upstream commit into a cache repo (.upstream/NAME.git, git-ignored).
#   2. Build a snapshot commit there whose tree holds ONLY our directory, with the previous snapshot
#      as parent, and fetch it into this repo as branch `upstream/NAME`.
#   3. Merge `upstream/NAME` into the current branch and record the commit in UPSTREAM.lock.
#
# Exit status: 0 merged (or already up to date), 1 merge conflicts (merge left in progress, files
# listed), 2 usage or precondition error (nothing changed).
#
# Usage: scripts/upstream-merge.sh NAME [--ref REF] [--no-merge] [--abort-on-conflict]
set -euo pipefail

die() {
    printf 'upstream-merge: %s\n' "$1" >&2
    exit 2
}
usage() {
    sed -n '2,18p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 2
}

[[ $# -ge 1 ]] || usage
name="$1"
shift
case "$name" in
    abs-web) repo_url="https://github.com/advplyr/audiobookshelf" src="client" dst="abs-web" ref="v2.37.2" ;;
    abs-app) repo_url="https://github.com/advplyr/audiobookshelf-app" src="." dst="abs-app" ref="v0.14.2-beta" ;;
    abs-server) repo_url="https://github.com/advplyr/audiobookshelf" src="server" dst="" ref="v2.37.2" ;;
    readarr) repo_url="https://github.com/Readarr/Readarr" src="src" dst="" ref="develop" ;;
    -h | --help) usage ;;
    *) die "unknown upstream: $name (abs-web, abs-app, abs-server or readarr)" ;;
esac
do_merge=1
abort_on_conflict=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --ref) [[ $# -ge 2 ]] || usage; ref="$2"; shift 2 ;;
        --no-merge) do_merge=0; shift ;;
        --abort-on-conflict) abort_on_conflict=1; shift ;;
        -h | --help) usage ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
done
[[ "$ref" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]{0,200}$ && "$ref" != *..* ]] || die "invalid --ref: $ref"
branch="upstream/$name"
lock_file="UPSTREAM.lock"

top="$(git rev-parse --show-toplevel 2>/dev/null)" || die "not inside a git repository"
cd -- "$top"
git rev-parse --verify --quiet HEAD >/dev/null || die "repository has no commits"
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || die "working tree has uncommitted changes; commit or stash them first"
! git rev-parse --verify --quiet MERGE_HEAD >/dev/null || die "a merge is already in progress; finish or abort it first"

cache="$top/.upstream/$name.git"
[[ -d "$cache" ]] || { mkdir -p -- "$top/.upstream"; git init --quiet --bare -- "$cache"; }
ucache() { git --git-dir="$cache" "$@"; }

echo "Fetching $ref from $repo_url ..."
ucache fetch --quiet --depth 1 --no-tags -- "$repo_url" "$ref" || die "fetch failed for $ref from $repo_url"
upstream_sha="$(ucache rev-parse --verify 'FETCH_HEAD^{commit}')"
upstream_date="$(ucache log -1 --format=%cI "$upstream_sha")"
echo "Upstream commit: $upstream_sha ($upstream_date)"

# UPSTREAM.lock keeps one [section] per upstream; only this one is rewritten.
write_lock() {
    local tmp
    tmp="$(mktemp)"
    {
        echo "# Managed by scripts/upstream-merge.sh; do not edit by hand."
        if [[ -f "$lock_file" ]]; then
            awk -v n="[$name]" '
                /^# Managed/ { next }
                /^\[/ { skip = ($0 == n) }
                !skip' "$lock_file" | sed '/./,$!d'
            echo
        fi
        echo "[$name]"
        echo "repo = \"$repo_url\""
        echo "ref = \"$ref\""
        echo "commit = \"$upstream_sha\""
        echo "commit_date = \"$upstream_date\""
        echo "from = \"$src\""
        [[ -n "$dst" ]] && echo "into = \"$dst\""
    } >"$tmp"
    mv -- "$tmp" "$lock_file"
    git add -- "$lock_file"
}

if [[ -z "$dst" ]]; then
    write_lock
    git commit --quiet -m "Pin upstream $name at ${upstream_sha:0:12} (reference for the port)" -- "$lock_file"
    echo "Pinned $name at ${upstream_sha:0:12} in $lock_file."
    exit 0
fi

prev=""
prev_upstream=""
# The last snapshot: the local branch, else the newest one in this branch's history (a fresh clone, or the
# branch deleted after its merge).
if ! git rev-parse --verify --quiet "refs/heads/$branch" >/dev/null; then
    last="$(git log -1 --format=%H --grep="^Upstream snapshot of $dst at " HEAD 2>/dev/null || true)"
    [[ -n "$last" ]] && git branch --quiet "$branch" "$last"
fi
if git rev-parse --verify --quiet "refs/heads/$branch" >/dev/null; then
    prev="$(git rev-parse "refs/heads/$branch")"
    prev_upstream="$(git log -1 --format=%B "$prev" | sed -n 's/^Upstream-Commit: //p' | head -n1)"
    if [[ "$prev_upstream" == "$upstream_sha" ]] && [[ $do_merge -eq 1 ]] && git merge-base --is-ancestor "$prev" HEAD; then
        echo "Already merged; nothing to do."
        exit 0
    fi
    [[ "$prev_upstream" == "$upstream_sha" ]] || ucache fetch --quiet --no-tags -- "$top" "refs/heads/$branch:refs/folio/prev"
fi

if [[ -z "$prev" || "$prev_upstream" != "$upstream_sha" ]]; then
    if [[ "$src" == "." ]]; then
        sub="$(ucache rev-parse "$upstream_sha^{tree}")"
    else
        sub="$(ucache rev-parse --verify --quiet "$upstream_sha:$src")" || die "upstream has no $src/ at $upstream_sha"
    fi
    tree="$(printf '040000 tree %s\t%s\n' "$sub" "$dst" | ucache mktree)"
    msg="$(printf 'Upstream snapshot of %s at %s\n\nUpstream-Repo: %s\nUpstream-Commit: %s\nUpstream-Date: %s\n' \
        "$dst" "${upstream_sha:0:12}" "$repo_url" "$upstream_sha" "$upstream_date")"
    parent_args=()
    [[ -n "$prev" ]] && parent_args=(-p "$prev")
    snapshot="$(GIT_AUTHOR_NAME="Folio upstream import" GIT_AUTHOR_EMAIL="upstream@folio.invalid" \
        GIT_COMMITTER_NAME="Folio upstream import" GIT_COMMITTER_EMAIL="upstream@folio.invalid" \
        ucache commit-tree "$tree" "${parent_args[@]}" -m "$msg")"
    ucache update-ref "refs/folio/snapshot" "$snapshot"
    git fetch --quiet --no-tags -- "$cache" "refs/folio/snapshot:refs/heads/$branch"
    echo "Updated $branch -> ${snapshot:0:12}"
fi

if [[ $do_merge -eq 0 ]]; then
    echo "--no-merge: branch $branch updated; merge it with: git merge $branch"
    exit 0
fi

merge_args=(--no-ff --no-commit)
if ! git merge-base HEAD "refs/heads/$branch" >/dev/null 2>&1; then
    echo "First import: merging unrelated history."
    merge_args+=(--allow-unrelated-histories)
fi
set +e
git merge "${merge_args[@]}" -m "Merge upstream $name ${upstream_sha:0:12}" "refs/heads/$branch" >/dev/null 2>&1
merge_status=$?
set -e

mapfile -t conflicts < <(git diff --name-only --diff-filter=U)
if [[ $merge_status -ne 0 && ${#conflicts[@]} -eq 0 ]]; then
    git merge --abort >/dev/null 2>&1 || true
    die "git merge failed without conflicts (see 'git merge $branch')"
fi
if [[ ${#conflicts[@]} -gt 0 ]]; then
    echo
    echo "CONFLICTS merging upstream $name ${upstream_sha:0:12}:"
    printf '      %s\n' "${conflicts[@]}"
    if [[ $abort_on_conflict -eq 1 ]]; then
        git merge --abort
        echo "Merge aborted (--abort-on-conflict). $branch still points at the new snapshot."
    else
        write_lock
        echo
        echo "Resolve the files above, then: git add <files> && git commit"
        echo "Or abandon with: git merge --abort"
    fi
    exit 1
fi
write_lock
git commit --quiet --no-edit
echo "Merged upstream $name ${upstream_sha:0:12} into $(git rev-parse --abbrev-ref HEAD) ($(git rev-parse --short HEAD))."
