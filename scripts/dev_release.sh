#!/usr/bin/env bash
# Publish a dev build as the GitHub pre-release `dev` at BUILD_SHA without a window in which
# `releases/download/dev/...` returns 404 (dev-build.yml, #339/#346).
#
#   scripts/dev_release.sh NOTES DIST
#
# Needs GH_TOKEN, GH_REPO, BUILD_SHA and GITHUB_RUN_ID; uses gh and curl. DIST holds the new
# assets (four CLI archives, four wheels, SHA256SUMS).
#
# With a published `dev` release:
#   1. move the tag: PATCH git/refs/tags/dev to BUILD_SHA (force). The release keeps
#      serving the previous build's assets meanwhile.
#   2. upload each new asset under a staging name, then give it its real name (deleting a
#      same-named old asset just before; only a rebuild of the same version, and
#      SHA256SUMS, have one, so only those names are missing for one API call).
#   3. update the release's notes, then delete the old assets that are not in the new set.
# If GitHub refuses the tag move (for example a GITHUB_TOKEN restriction on refs), it falls
# back to staging a draft and replacing the release, which leaves a few seconds of 404.
# Without a release, it creates one the same way. Either way it then proves the result
# through the API: the tag ref resolves to BUILD_SHA, and `dev` is a published pre-release
# whose assets are exactly the new set.
set -euo pipefail
if [ "$#" -ne 2 ]; then
  echo "usage: $0 NOTES DIST" >&2
  exit 2
fi
notes=$1 dist=$2
api="repos/$GH_REPO"
title='Development build'
staging="staging-$GITHUB_RUN_ID-"

upload() { # release-id file name
  curl --fail --silent --show-error --retry 3 -X POST -o /dev/null \
    -H "Authorization: Bearer $GH_TOKEN" -H 'Content-Type: application/octet-stream' \
    --data-binary @"$2" "https://uploads.github.com/$api/releases/$1/assets?name=$3"
}

# The new assets, SHA256SUMS last: the installer reads it first and then the archive it
# names, so it is renamed into place only after every file it lists.
new_assets() {
  (cd "$dist" && printf '%s\n' ./*.tgz ./*.zip ./*.whl) | sed 's|^\./||' | LC_ALL=C sort
  echo SHA256SUMS
}

replace_with_draft() { # Stage a complete draft, then swap it in as `dev`.
  local draft previous
  draft=$(gh api -X POST "$api/releases" -f tag_name="dev-staging-$GITHUB_RUN_ID" \
    -f target_commitish="$BUILD_SHA" -f name="$title" -F body=@"$notes" \
    -F draft=true -F prerelease=true --jq .id)
  while read -r name; do upload "$draft" "$dist/$name" "$name"; done < <(new_assets)
  test "$(gh api "$api/releases/$draft" --jq '.assets | length')" -eq "$(new_assets | wc -l)"
  # releases/tags/ returns only a published release, never the draft.
  if previous=$(gh api "$api/releases/tags/dev" --jq .id 2>/dev/null); then
    gh api -X DELETE "$api/releases/$previous"
  fi
  if gh api "$api/git/ref/tags/dev" > /dev/null 2>&1; then
    gh api -X DELETE "$api/git/refs/tags/dev"
  fi
  gh api -X PATCH "$api/releases/$draft" -f tag_name=dev -f target_commitish="$BUILD_SHA" \
    -F draft=false -F prerelease=true --jq .html_url
}

update_in_place() { # release-id
  local release=$1 id name staged old wanted
  old=$(gh api --paginate "$api/releases/$release/assets" --jq '.[] | "\(.id)\t\(.name)"')
  wanted=$(new_assets)
  while read -r name; do
    upload "$release" "$dist/$name" "$staging$name"
  done < <(new_assets)
  while read -r name; do
    staged=$(gh api --paginate "$api/releases/$release/assets" \
      --jq ".[] | select(.name == \"$staging$name\") | .id")
    id=$(printf '%s\n' "$old" | awk -F'\t' -v n="$name" '$2 == n {print $1}')
    if [ -n "$id" ]; then gh api -X DELETE "$api/releases/assets/$id"; fi
    gh api -X PATCH "$api/releases/assets/$staged" -f name="$name" > /dev/null
  done < <(new_assets)
  gh api -X PATCH "$api/releases/$release" -f name="$title" -F body=@"$notes" \
    -F prerelease=true > /dev/null
  while IFS=$'\t' read -r id name; do
    # Not `new_assets | grep -q`: with pipefail, grep's early exit would fail the pipeline.
    if [ -n "$id" ] && ! grep -Fxq -- "$name" <<< "$wanted"; then
      gh api -X DELETE "$api/releases/assets/$id"
    fi
  done <<< "$old"
}

# Staging drafts left by a failed earlier run.
for id in $(gh api --paginate "$api/releases" --jq ".[] | select(.draft and .name == \"$title\" and (.tag_name | startswith(\"dev-staging-\"))) | .id"); do
  gh api -X DELETE "$api/releases/$id"
done

if release=$(gh api "$api/releases/tags/dev" --jq .id 2>/dev/null); then
  if gh api -X PATCH "$api/git/refs/tags/dev" -f sha="$BUILD_SHA" -F force=true > /dev/null; then
    echo "Moved tag dev to $BUILD_SHA; updating release $release in place."
    update_in_place "$release"
  else
    echo "::warning::GitHub refused to move tag dev; replacing the release instead (downloads return 404 for a few seconds)."
    replace_with_draft
  fi
else
  echo "No published dev release; creating it."
  replace_with_draft
fi

# Proof, from the same API: the tag, the release and its exact assets.
ref=$(gh api "$api/git/ref/tags/dev" --jq '.object.type + " " + .object.sha')
echo "git/ref/tags/dev: $ref"
test "$ref" = "commit $BUILD_SHA"
state=$(gh api "$api/releases/tags/dev" --jq '"draft=\(.draft) prerelease=\(.prerelease) tag=\(.tag_name)"')
echo "releases/tags/dev: $state"
test "$state" = "draft=false prerelease=true tag=dev"
diff <(gh api "$api/releases/tags/dev" --jq '.assets[].name' | LC_ALL=C sort) <(new_assets | LC_ALL=C sort)
echo "Release dev serves $(new_assets | wc -l) assets of $BUILD_SHA."
