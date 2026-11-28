#!/bin/sh
# What's unreleased, counted once per pull request, for the release workflows (cliff.toml has
# the commit rules). Works whether pull requests were squash-merged or merged with a merge commit.
#
#   scripts/changes.sh version [<head>]              the next stable version (x.y.z); nothing if
#                                                    there's nothing to release
#   scripts/changes.sh notes <tag> [<head> [<from>]] the release notes for <tag> (git-cliff)
#   scripts/changes.sh cut [<except-tag>]            the dev commit the newest release was cut from
#
# The changes are the commits on <head>'s first-parent line since <from>: one per pull request
# (a squash commit, or a merge commit read as its pull request's title), never the commits
# inside a merged branch. <from> defaults to the dev commit the newest release was cut from,
# which its tag records (`cut: <sha>`), so a squash-merged release pull request (whose commits
# aren't ancestors of its tag) isn't counted again. <head> defaults to HEAD.
set -eu

# Stable tags, newest version first.
stable_tags() {
  git tag --list 'v[0-9]*' | grep -v -- - | sort -rV || true
}

# The dev commit the newest release (not hotfix) was cut from, from its annotated tag. Tags from
# before tags recorded it count from where they meet <head>.
last_cut() {
  for t in $(stable_tags); do
    [ "$t" = "${1:-}" ] && continue
    if [ "$(git cat-file -t "$t")" = tag ]; then
      msg=$(git cat-file -p "$t" | sed '1,/^$/d')
      case "$msg" in *"kind: hotfix"*) continue ;; esac
      cut=$(printf '%s\n' "$msg" | sed -n 's/^cut: \([0-9a-f]*\)$/\1/p')
      if [ -n "$cut" ]; then
        echo "$cut"
        return
      fi
    fi
    git merge-base "$t" "$head" || true
    return
  done
}

# A commit's message, as the pull request's title for a pull request's merge commit.
message() {
  m=$(git log -1 --format=%B "$1")
  case "$m" in
    "Merge pull request #"*) printf '%s\n' "$m" | sed '1,/^$/d' ;;
    *) printf '%s\n' "$m" ;;
  esac
}

# One commit per pull request since $from.
changes() {
  if [ -n "$from" ]; then git rev-list --first-parent "$from..$head"; else git rev-list --first-parent "$head"; fi
}

cmd=${1:?usage: scripts/changes.sh version|notes|cut ...}
shift
case "$cmd" in
  cut)
    head=HEAD
    last_cut "${1:-}"
    ;;

  version)
    head=${1:-HEAD}
    from=$(last_cut)
    bump=0  # 1 patch, 2 minor, 3 major
    for c in $(changes); do
      m=$(message "$c")
      subject=$(printf '%s\n' "$m" | head -n 1)
      if printf '%s\n' "$subject" | grep -Eq '^[a-z]+(\([^)]*\))?!:' || printf '%s\n' "$m" | grep -q '^BREAKING CHANGE'; then
        level=3
      elif printf '%s\n' "$subject" | grep -Eq '^feat(\([^)]*\))?:'; then
        level=2
      elif printf '%s\n' "$subject" | grep -Eq '^(fix|perf)(\([^)]*\))?:'; then
        level=1
      else
        level=0
      fi
      [ "$level" -gt "$bump" ] && bump=$level
    done
    [ "$bump" -eq 0 ] && exit 0
    last=$(stable_tags | head -n 1)
    last=${last:-v0.0.0}
    # Same rules as cliff.toml's [bump]: a feature raises the minor version and a breaking change
    # the major one, even below 1.0.0.
    echo "${last#v}" | awk -F. -v b="$bump" '{
      if (b == 3) printf "%d.0.0\n", $1 + 1
      else if (b == 2) printf "%d.%d.0\n", $1, $2 + 1
      else printf "%d.%d.%d\n", $1, $2, $3 + 1 }'
    ;;

  notes)
    tag=${1:?usage: scripts/changes.sh notes <tag> [<head> [<from>]]}
    head=${2:-HEAD}
    from=${3:-$(last_cut "$tag")}
    # Everything git-cliff would walk that isn't one of the changes goes in .cliffignore.
    top=$(git rev-parse --show-toplevel)
    trap 'rm -f "$top/.cliffignore"' EXIT
    keep=$(changes)
    if [ -n "$from" ]; then all=$(git rev-list "$from..$head"); else all=$(git rev-list "$head"); fi
    printf '%s\n' "$all" | grep -vxF -e "${keep:-none}" > "$top/.cliffignore" || true
    if [ -n "$from" ]; then range="$from..$head"; else range=$head; fi
    # No tag counts as a release boundary here (the range is the boundary): with one, git-cliff
    # reaches back past the start of the range to the previous tag.
    git cliff "$range" --tag-pattern '^$' --unreleased --tag "$tag" --strip all 2>/dev/null
    ;;

  *)
    echo "usage: scripts/changes.sh version|notes|cut ..." >&2
    exit 2
    ;;
esac
