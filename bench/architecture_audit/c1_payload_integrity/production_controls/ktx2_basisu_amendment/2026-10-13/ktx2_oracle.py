"""Independent KHR_texture_basisu / KTX2 container oracle (standard library only).

Written from the pinned texts, not from rusty-tiles source:
- KHR_texture_basisu README, KhronosGroup/glTF 1bde276f (primary/KHR_texture_basisu.README.md);
- KTX File Format Specification 2.0, KhronosGroup/KTX-Specification v2.0.4 30cb4530
  (sections cited per rule; the 106,109-byte text is pinned by hash in primary/pins.json);
- its BasisLZ global-data appendix (primary/basislz-gdata.adoc).

`ktx2_violations` lists every rule a container breaks, as (category, rule). Parsing stops
at a rule whose failure leaves later fields unlocatable ("fatal"). The container is the
only thing read: no level is inflated, transcoded or decoded.
"""
import struct

IDENTIFIER = bytes([0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A])
EXT = 'KHR_texture_basisu'
INVALID, UNSUPPORTED = 'invalid_input', 'unsupported'


def u32(raw, at):
    return struct.unpack_from('<I', raw, at)[0]


def u64(raw, at):
    return struct.unpack_from('<Q', raw, at)[0]


def align(value, to):
    return -(-value // to) * to


def ktx2_violations(raw, material):
    found = []

    def bad(rule, category=INVALID):
        found.append((category, rule))

    # KTX 2.0.4 "identifier".
    if raw[:12] != IDENTIFIER:
        return [(INVALID, 'identifier')]
    if len(raw) < 80:
        return [(INVALID, 'header-truncated')]
    vk, type_size, width, height, depth, layers, faces, level_count, scheme = struct.unpack_from('<9I', raw, 12)
    dfd_offset, dfd_length, kvd_offset, kvd_length = struct.unpack_from('<4I', raw, 48)
    sgd_offset, sgd_length = struct.unpack_from('<QQ', raw, 64)
    # "Texture Type" table, and "pixelHeight must not be 0 for block-compressed formats,
    # including BasisLZ/ETC1S and UASTC".
    if width == 0 or height == 0 or faces not in (1, 6) or (faces == 6 and (height != width or depth)):
        return [(INVALID, 'texture-type')]
    # Extension: "the KTX v2 image MUST be of 2D type" for material maps. Other valid
    # types are outside the admitted profile when no material uses the texture.
    if depth or layers or faces != 1:
        return [(INVALID if material else UNSUPPORTED, 'not-2d')]
    # "BasisLZ ... vkFormat must be VK_FORMAT_UNDEFINED"; UASTC is indicated by
    # VK_FORMAT_UNDEFINED; "When format is VK_FORMAT_UNDEFINED, typeSize must equal 1".
    if vk != 0 or type_size != 1:
        bad('vkformat')
    # Extension: "pixelWidth and pixelHeight MUST be multiples of 4".
    if width % 4 or height % 4:
        bad('multiple-of-4')
    # "levelCount = 0 is allowed, except for block-compressed formats"; level_p must not
    # exceed M = floor(log2(max(width, height, depth))).
    if level_count == 0 or level_count - 1 > max(width, height).bit_length() - 1:
        return found + [(INVALID, 'level-count')]
    # Extension: ETC1S uses scheme 1, UASTC scheme 0 or 2; nothing else is allowed.
    if scheme not in (0, 1, 2):
        bad('scheme')
    levels = level_count
    index_end = 80 + 24 * levels
    if len(raw) < index_end:
        return found + [(INVALID, 'index-truncated')]
    # "Basic Structure": the DFD directly follows the level index; dfdTotalSize equals
    # dfdByteLength; the first descriptor block is the Khronos basic block, version 2.
    if dfd_offset != index_end:
        bad('dfd-position')
    if dfd_offset + dfd_length > len(raw) or dfd_length < 28:
        return found + [(INVALID, 'dfd-range')]
    dfd = raw[dfd_offset:dfd_offset + dfd_length]
    if u32(dfd, 0) != dfd_length or dfd_length % 4:
        bad('dfd-size')
    block_size = u32(dfd, 8) >> 16
    if u32(dfd, 4) != 0 or u32(dfd, 8) & 0xFFFF != 2 or block_size < 24 or (block_size - 24) % 16 or 4 + block_size > dfd_length:
        return found + [(INVALID, 'dfd-block')]
    samples = (block_size - 24) // 16
    channels = tuple(dfd[4 + 24 + 16 * i + 3] & 0x0F for i in range(samples))
    model = dfd[12]
    if model == 163:  # KHR_DF_MODEL_ETC1S
        if scheme != 1:
            bad('model-scheme')
        # Extension: RGB, RGB+AAA, RRR, RRR+GGG.
        if channels not in ((0,), (0, 15), (3,), (3, 4)):
            bad('channels')
    elif model == 166:  # KHR_DF_MODEL_UASTC
        if scheme == 1:
            bad('model-scheme')
        # Extension: single UASTC_RGB, UASTC_RGBA, UASTC_RRR or UASTC_RG channel.
        if channels not in ((0,), (3,), (4,), (6,)):
            bad('channels')
    else:
        bad('color-model')
    # ETC1S and UASTC are 4x4 blocks (texelBlockDimension stores size - 1).
    if dfd[16:20] != bytes([3, 3, 0, 0]):
        bad('texel-block')

    # "Key/Value Data": follows the DFD; sum(ceil(len/4)*4) + 4n == kvdByteLength;
    # keyAndValueByteLength >= 2 with a NUL-terminated key. kvdByteOffset is 0 when
    # kvdByteLength is 0. Extension: KTXswizzle rgba, KTXorientation rd (or omitted).
    end = dfd_offset + dfd_length
    if kvd_length == 0:
        if kvd_offset != 0:
            bad('kvd-position')
    else:
        if kvd_offset != end:
            bad('kvd-position')
        if kvd_offset + kvd_length > len(raw):
            return found + [(INVALID, 'kvd-range')]
        kvd = raw[kvd_offset:kvd_offset + kvd_length]
        at = 0
        while at < kvd_length:
            if at + 4 > kvd_length:
                bad('kvd-pairs')
                break
            n = u32(kvd, at)
            pair = kvd[at + 4:at + 4 + n]
            if n < 2 or len(pair) != n or b'\0' not in pair or pair[0] == 0:
                bad('kvd-pairs')
                break
            key, value = pair.split(b'\0', 1)
            if value.endswith(b'\0'):
                value = value[:-1]
            if (key == b'KTXswizzle' and value != b'rgba') or (key == b'KTXorientation' and value != b'rd'):
                bad('kvd-metadata')
            at += 4 + align(n, 4)
        else:
            if at != kvd_length:
                bad('kvd-pairs')
        end = kvd_offset + kvd_length

    # "supercompressionGlobalData": only schemes with a Global Data Format (BasisLZ)
    # have it; it starts at the next 8-byte boundary; sgdByteOffset is 0 without it.
    if sgd_length == 0:
        if sgd_offset != 0 or scheme == 1:
            bad('global-data')
    else:
        if scheme != 1 or sgd_offset != align(end, 8):
            bad('global-data')
        if sgd_offset + sgd_length > len(raw):
            return found + [(INVALID, 'global-data-range')]
        end = sgd_offset + sgd_length

    # "Mip Level Array" and "Level Index": smallest level first; mipPadding aligns
    # level starts to lcm(texel block size, 4) = 16 only for scheme 0; the image data
    # ends at the end of the file. uncompressedByteLength is the UASTC block bytes
    # (byteLength too for scheme 0) and 0 for BasisLZ.
    alignment = 16 if scheme == 0 else 1
    cursor = end
    lengths = {}
    for p in reversed(range(levels)):
        offset, length, inflated = struct.unpack_from('<QQQ', raw, 80 + 24 * p)
        if offset != align(cursor, alignment):
            bad('level-offset')
        if length == 0:
            bad('level-empty')
        if offset + length > len(raw):
            return found + [(INVALID, 'level-range')]
        blocks = -(-max(1, width >> p) // 4) * -(-max(1, height >> p) // 4)
        if (scheme == 0 and (length != 16 * blocks or inflated != 16 * blocks)) or \
                (scheme == 2 and inflated != 16 * blocks) or (scheme == 1 and inflated != 0):
            bad('level-lengths')
        lengths[p] = length
        cursor = offset + length
    if cursor != len(raw):
        bad('trailing-bytes')

    # BasisLZ global data: 20-byte header, imageCount (= levels for 2D) ImageDescs,
    # then the four data arrays, with no trailing bytes; extendedByteLength 0 for ETC1S.
    if scheme == 1 and sgd_length:
        sgd = raw[sgd_offset:sgd_offset + sgd_length]
        if len(sgd) < 20:
            return found + [(INVALID, 'basislz-length')]
        _, _, endpoints, selectors, tables, extended = struct.unpack_from('<HHIIII', sgd, 0)
        if sgd_length != 20 + 20 * levels + endpoints + selectors + tables + extended or extended:
            return found + [(INVALID, 'basislz-length')]
        for p in range(levels):
            flags, rgb_offset, rgb_length, alpha_offset, alpha_length = struct.unpack_from('<5I', sgd, 20 + 20 * p)
            # isPFrame is only for animation sequences, which are arrays.
            alpha_ok = (alpha_offset == 0 and alpha_length == 0) if samples == 1 else \
                (alpha_length != 0 and alpha_offset + alpha_length <= lengths[p])
            if flags or rgb_length == 0 or rgb_offset + rgb_length > lengths[p] or not alpha_ok:
                bad('image-descriptor')
    return found


def declared_ktx2(image):
    """image/ktx2, or no mimeType and a `.ktx2` URI."""
    if 'mimeType' in image:
        return image['mimeType'] == 'image/ktx2'
    return image.get('uri', '').lower().endswith('.ktx2')


def gltf_violations(doc, image_bytes):
    """Texture/image rules of KHR_texture_basisu plus each KTX2 source's container."""
    found = []
    used, required = set(doc.get('extensionsUsed', [])), set(doc.get('extensionsRequired', []))
    textures, images = doc.get('textures', []), doc.get('images', [])
    basisu_sources, core_sources, material_textures = {}, set(), set()
    for material in doc.get('materials', []):
        pbr = material.get('pbrMetallicRoughness', {})
        for info in (pbr.get('baseColorTexture'), pbr.get('metallicRoughnessTexture'), material.get('normalTexture'),
                     material.get('occlusionTexture'), material.get('emissiveTexture')):
            if info is not None:
                material_textures.add(info['index'])
    for index, texture in enumerate(textures):
        if 'source' in texture:
            core_sources.add(texture['source'])
        extension = texture.get('extensions', {}).get(EXT)
        if extension is None:
            continue
        # glTF 2.0: every extension object is declared in extensionsUsed.
        if EXT not in used:
            found.append((INVALID, 'undeclared-extension'))
        # Extension: "To use ... without a fallback, define KHR_texture_basisu in both
        # extensionsUsed and extensionsRequired."
        if 'source' not in texture and EXT not in required:
            found.append((INVALID, 'fallback-required'))
        if 'source' in extension:
            basisu_sources.setdefault(extension['source'], False)
            basisu_sources[extension['source']] |= index in material_textures
    for index, image in enumerate(images):
        ktx2 = declared_ktx2(image)
        # Extension: image/ktx2 is allowed for images referenced by its `source`.
        if index in basisu_sources and not ktx2:
            found.append((INVALID, 'mime-type'))
        if index in core_sources and ktx2 and EXT in used:
            found.append((INVALID, 'fallback-is-ktx2'))
        if index in basisu_sources:
            found += ktx2_violations(image_bytes[index], basisu_sources[index])
    return found


def category(violations):
    categories = {c for c, _ in violations}
    return INVALID if INVALID in categories else UNSUPPORTED if categories else 'admitted'
