#!/bin/sh
# What's unreleased, for the release workflows: the next version and the release notes, from
# Conventional Commit messages. Works whether pull requests were squash-merged or merged.
#
#   scripts/changes.sh version [<head>]              the next stable version (x.y.z); nothing if
#                                                    there's nothing to release
#   scripts/changes.sh notes <tag> [<head> [<from>]] the release notes (Markdown)
#   scripts/changes.sh cut [<except-tag>]            the dev commit the newest release was cut from
#
# The changes are the commits on <head>'s first-parent line since <from>: one per pull request
# (a squash commit, or a merge commit, whose message is the pull request's title), never the
# commits inside a merged branch. A patch/* pull request is the exception: it carries several
# changes, so it counts as each of its commits (from the GitHub API, which keeps them even when
# it was squashed; the organization's rules make every commit on patch/** a Conventional Commit).
# <from> defaults to the dev commit the newest release was cut from, which its tag records
# (`cut: <sha>`), so a squash-merged release pull request (whose commits aren't ancestors of its
# tag) isn't counted again. <head> defaults to HEAD.
#
#   feat → minor, fix / perf → patch, `!` or a `BREAKING CHANGE:` footer → major (even below
#   1.0.0); other types (chore, ci, docs, refactor, test, …) release nothing and stay out of the
#   notes.
#
# The API needs GITHUB_REPOSITORY and an authenticated gh (GH_TOKEN), as in the workflows.
# Without them a patch/ pull request counts once, by its title.
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

# A commit's message; for an old-style "Merge pull request #n from …" commit, the pull request's
# title under it.
message() {
  m=$(git log -1 --format=%B "$1")
  case "$m" in
    "Merge pull request #"*) printf '%s\n' "$m" | sed '1,/^$/d' ;;
    *) printf '%s\n' "$m" ;;
  esac
}

# One commit per pull request since $from, oldest first.
changes() {
  if [ -n "$from" ]; then git rev-list --first-parent --reverse "$from..$head"; else git rev-list --first-parent --reverse "$head"; fi
}

api() {
  [ -n "${GITHUB_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1
}

# One line per change, oldest first: "<subject><TAB><1 if a BREAKING CHANGE footer>".
entries() {
  for c in $(changes); do
    pr=
    if api; then
      pr=$(gh api "repos/$GITHUB_REPOSITORY/commits/$c/pulls" \
        --jq '[.[] | select(.head.ref | startswith("patch/"))][0].number // empty' 2>/dev/null || true)
    fi
    if [ -n "$pr" ]; then
      gh api --paginate "repos/$GITHUB_REPOSITORY/pulls/$pr/commits" \
        --jq '.[].commit.message | [(split("\n")[0]), (if test("(?m)^BREAKING CHANGE") then "1" else "0" end)] | @tsv'
    else
      m=$(message "$c")
      breaking=0
      printf '%s\n' "$m" | grep -q '^BREAKING CHANGE' && breaking=1
      printf '%s\t%s\n' "$(printf '%s\n' "$m" | head -n 1)" "$breaking"
    fi
  done
}

# Reads entries; prints the bump level (version) or the notes (notes).
render() {
  awk -F '\t' -v mode="$1" '
    function upfirst(s) { return toupper(substr(s, 1, 1)) substr(s, 2) }
    {
      if (!match($1, /^[a-z]+/)) next
      type = substr($1, 1, RLENGTH); rest = substr($1, RLENGTH + 1); scope = ""; brk = ($2 == "1")
      if (substr(rest, 1, 1) == "(") {
        i = index(rest, ")"); if (!i) next
        scope = substr(rest, 2, i - 2); rest = substr(rest, i + 1)
      }
      if (substr(rest, 1, 1) == "!") { brk = 1; rest = substr(rest, 2) }
      if (substr(rest, 1, 2) != ": ") next
      level = brk ? 3 : type == "feat" ? 2 : (type == "fix" || type == "perf") ? 1 : 0
      if (level > max) max = level
      g = (brk || type == "feat") ? 1 : type == "fix" ? 2 : type == "perf" ? 3 : type == "revert" ? 4 : 0
      if (!g) next
      gsub(/, */, ", ", scope)
      items[g, ++n[g]] = "- " (scope != "" ? "**" scope "**: " : "") (brk ? "**breaking:** " : "") upfirst(substr(rest, 3))
    }
    END {
      if (mode == "version") { print max + 0; exit }
      split("Features Fixes Performance Reverts", name, " ")
      for (g = 1; g <= 4; g++) if (n[g]) {
        printf "\n### %s\n\n", name[g]
        for (i = 1; i <= n[g]; i++) print items[g, i]
      }
    }'
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
    bump=$(entries | render version)
    [ "$bump" -eq 0 ] && exit 0
    last=$(stable_tags | head -n 1)
    last=${last:-v0.0.0}
    echo "${last#v}" | awk -F. -v b="$bump" '{
      if (b == 3) printf "%d.0.0\n", $1 + 1
      else if (b == 2) printf "%d.%d.0\n", $1, $2 + 1
      else printf "%d.%d.%d\n", $1, $2, $3 + 1 }'
    ;;

  notes)
    tag=${1:?usage: scripts/changes.sh notes <tag> [<head> [<from>]]}
    head=${2:-HEAD}
    from=${3:-$(last_cut "$tag")}
    entries | render notes
    ;;

  *)
    echo "usage: scripts/changes.sh version|notes|cut ..." >&2
    exit 2
    ;;
esac
