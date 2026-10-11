#!/bin/bash
set -e
cd /tmp/rusty-tiles-r2-foundation
FILES="src/validate.rs src/validate/types.rs src/content_integrity.rs src/content_integrity/payload.rs src/content_integrity/payload/records.rs src/implicit.rs src/implicit/tileset.rs src/metadata.rs src/mesh_archive/source.rs src/mesh_archive/placement.rs src/mesh_archive/source/original_bounds.rs src/mesh_archive/partition.rs src/mesh_archive/model.rs src/point_cloud.rs src/point_cloud/tiles.rs src/vector/pipeline/store.rs src/vector/pipeline/encoding.rs src/vector/model.rs src/terrain.rs src/bbox.rs src/tileset_node.rs src/tileset.rs src/tile.rs src/main.rs src/glb.rs docs/VALIDATION.md docs/CLI.md docs/architecture/public-surface-inventory.md bindings/python/src/lib.rs"
echo "repo=/tmp/rusty-tiles-r2-foundation pinned_commit=$(git rev-parse e83e880); blob sha256 via git show (worktree not read); compared with previous pin 70d184f"
for f in $FILES; do
  a=$(git show e83e880:$f | sha256sum | cut -d' ' -f1)
  b=$(git show 70d184f:$f | sha256sum | cut -d' ' -f1)
  if [ "$a" = "$b" ]; then s=same-as-70d184f; else s=CHANGED-since-70d184f; fi
  echo "$a  $f  $s"
done
