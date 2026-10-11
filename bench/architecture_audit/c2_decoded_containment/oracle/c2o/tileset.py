"""`.3tz` archive reading and explicit tileset traversal (with external tilesets).

Traversal order (an oracle reading; the contract says "traversal order" without
defining it): depth-first, pre-order; a tile's content(s) before its children;
an external tileset is expanded in place at its reference.
"""

import zipfile

from . import spans as S
from .gltf import resolve
from .numbers import JsonNumberError, OracleUndecided, exact, is_number, load_json
from .volumes import parse_volume

TILESET_ALLOWLIST = {"3DTILES_content_gltf_vector"}
ROLE_ORDER = {"own": 0, "content": 1, "externalContent": 2, "ancestor": 3}


class Archive:
    def __init__(self, path, max_member_bytes):
        try:
            self.zip = zipfile.ZipFile(path)
        except (OSError, zipfile.BadZipFile) as err:
            raise OracleUndecided("archive is not a readable zip: %s" % err)
        self.infos = {i.filename: i for i in self.zip.infolist()}
        self.max_member_bytes = max_member_bytes
        self.cache = {}

    def read(self, name):
        if name in self.cache:
            return self.cache[name]
        info = self.infos.get(name)
        if info is None or info.is_dir():
            raise OracleUndecided("archive member %r is missing (C1 is expected to reject)" % name)
        if info.file_size > self.max_member_bytes:
            raise OracleUndecided("member %r exceeds the oracle's member-size limit" % name)
        data = self.zip.read(info)
        self.cache[name] = data
        return data


class LevelVolume:
    def __init__(self, role, level, volume, where):
        self.role = role
        self.level = level
        self.volume = volume
        self.where = where


class Level:
    """One frame of a reference's chain. `to_parent` is the transform of the child
    tile (level-1) that maps frame l-1 into frame l; None at level 0 or when the
    child has no transform."""

    def __init__(self, tile_where):
        self.tile_where = tile_where
        self.volumes = []
        self.to_parent = None  # exact 4x4 or None
        self.to_parent_pairs = None

    def volumes_in_order(self):
        return sorted(self.volumes, key=lambda v: ROLE_ORDER[v.role])


class Reference:
    def __init__(self, index, member, uri, where, levels):
        self.index = index
        self.member = member
        self.uri = uri
        self.where = where
        self.levels = levels


class _Frame:
    """A tile on the current traversal path."""

    def __init__(self, tile, where, volume, transform, external_content):
        self.tile = tile
        self.where = where
        self.volume = volume
        self.transform = transform  # raw 16 numbers or None
        self.external_content = external_content  # (volume, where) or None


def _transform(tile, where):
    t = tile.get("transform")
    if t is None:
        return None
    if not isinstance(t, list) or len(t) != 16 or not all(is_number(v) for v in t):
        raise OracleUndecided("tile transform malformed at %s" % where)
    if not (t[3] == 0 and t[7] == 0 and t[11] == 0 and t[15] == 1):
        raise OracleUndecided("tile transform at %s is not affine; the contract's frame "
                              "chain assumes an affine last row" % where)
    return t


def _contents(tile):
    out = []
    if isinstance(tile.get("content"), dict):
        out.append(tile["content"])
    if isinstance(tile.get("contents"), list):
        out.extend(c for c in tile["contents"] if isinstance(c, dict))
    return out


class Traversal:
    def __init__(self, archive, max_tiles):
        self.archive = archive
        self.max_tiles = max_tiles
        self.tiles = 0
        self.references = []
        self.exclusions = set()
        self.exclusion_sites = []
        self.caveats = []
        self.c1_caveats = []

    def exclude(self, reason, where):
        self.exclusions.add(reason)
        self.exclusion_sites.append({"reason": reason, "where": where})

    def _check_extensions(self, obj, where):
        ext = obj.get("extensions")
        if isinstance(ext, dict):
            for name in ext:
                if name not in TILESET_ALLOWLIST:
                    self.exclude("tilesetExtension", "%s.extensions.%s" % (where, name))

    def run(self, root_member="tileset.json"):
        self._tileset(root_member, [], [root_member])
        return self

    def _tileset(self, member, stack, documents):
        try:
            doc = load_json(self.archive.read(member))
        except JsonNumberError as err:
            raise OracleUndecided("tileset number overflow in %s (C1 InvalidInput): %s" % (member, err))
        except (ValueError, UnicodeDecodeError) as err:
            raise OracleUndecided("tileset JSON unreadable %s: %s" % (member, err))
        if not isinstance(doc, dict) or not isinstance(doc.get("root"), dict):
            raise OracleUndecided("tileset %s has no root tile" % member)
        for key in ("extensionsUsed", "extensionsRequired"):
            names = doc.get(key, [])
            if not isinstance(names, list):
                raise OracleUndecided("%s is not an array in %s" % (key, member))
            for name in names:
                if name not in TILESET_ALLOWLIST:
                    self.exclude("tilesetExtension", "%s:%s:%s" % (member, key, name))
        self._check_extensions(doc, member)
        asset = doc.get("asset")
        if isinstance(asset, dict) and "gltfUpAxis" in asset and asset["gltfUpAxis"] != "Y":
            self.exclude("upAxisOverride", "%s:asset.gltfUpAxis" % member)
        self._tile(doc["root"], member, "%s:root" % member, stack, documents)

    def _tile(self, root, member, root_where, stack, documents):
        # Iterative pre-order walk: (tile, where, depth in `path`)
        path = list(stack)
        work = [(root, root_where, len(stack))]
        while work:
            tile, where, depth = work.pop()
            del path[depth:]
            self.tiles += 1
            if self.tiles > self.max_tiles:
                raise OracleUndecided("tile count exceeds the oracle's tile limit")
            if not isinstance(tile, dict) or "boundingVolume" not in tile:
                raise OracleUndecided("tile without boundingVolume at %s" % where)
            self._check_extensions(tile, where)
            if isinstance(tile.get("boundingVolume"), dict):
                self._check_extensions(tile["boundingVolume"], where + ".boundingVolume")
            volume = parse_volume(tile["boundingVolume"], where)
            if "metadata" in tile:
                self.exclude("tileMetadata", where + ".metadata")
            frame = _Frame(tile, where, volume, _transform(tile, where), None)
            path.append(frame)
            implicit = "implicitTiling" in tile
            if implicit:
                self.exclude("implicitTiling", where)
                self.caveats.append("implicit tile at %s: its subtree and template payloads are "
                                    "not expanded, so their records are not admission-checked by "
                                    "the oracle" % where)
            for ci, content in enumerate(_contents(tile)):
                cwhere = "%s.content[%d]" % (where, ci)
                self._check_extensions(content, cwhere)
                if "metadata" in content:
                    self.exclude("tileMetadata", cwhere + ".metadata")
                cvol = None
                if "boundingVolume" in content:
                    if isinstance(content["boundingVolume"], dict):
                        self._check_extensions(content["boundingVolume"], cwhere + ".boundingVolume")
                    cvol = parse_volume(content["boundingVolume"], cwhere)
                if implicit:
                    continue  # template URI, not a reference
                uri = content.get("uri")
                if not isinstance(uri, str):
                    raise OracleUndecided("content without uri at %s" % cwhere)
                target = resolve(member, uri)
                if target.endswith(".json"):
                    if target in documents:
                        raise OracleUndecided("external tileset cycle at %s" % cwhere)
                    outer = list(path)
                    outer[-1] = _Frame(tile, where, volume, frame.transform,
                                       (cvol, cwhere) if cvol is not None else None)
                    self._tileset(target, outer, documents + [target])
                else:
                    self._reference(target, uri, cwhere, path, cvol)
            children = tile.get("children", [])
            if not implicit and isinstance(children, list):
                for ci in reversed(range(len(children))):
                    work.append((children[ci], "%s.children[%d]" % (where, ci), len(path)))

    def _reference(self, target, uri, where, path, content_volume):
        levels = []
        for distance, frame in enumerate(reversed(path)):
            level = Level(frame.where)
            if distance == 0:
                level.volumes.append(LevelVolume("own", 0, frame.volume, frame.where))
                if content_volume is not None:
                    level.volumes.append(LevelVolume("content", 0, content_volume, where))
            else:
                if frame.external_content is not None:
                    vol, cwhere = frame.external_content
                    level.volumes.append(LevelVolume("externalContent", distance, vol, cwhere))
                level.volumes.append(LevelVolume("ancestor", distance, frame.volume, frame.where))
                child = path[len(path) - distance]
                if child.transform is not None:
                    level.to_parent = [[exact(child.transform[4 * c + r]) for c in range(4)]
                                       for r in range(4)]
                    level.to_parent_pairs = [S.pair_of(exact(v)) for v in child.transform]
            levels.append(level)
        for level in levels:
            for lv in level.volumes:
                reason = lv.volume.exclusion()
                if reason:
                    self.exclude(reason, lv.where)
        self.references.append(Reference(len(self.references), target, uri, where, levels))
