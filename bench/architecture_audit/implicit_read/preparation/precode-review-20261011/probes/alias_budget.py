# Reviewer arithmetic: an I1-admissible subtree whose availability bit passes are
# amplified by K content layers aliasing one bufferView. Checks every default
# ValidationLimits value the contract applies before the reservation (no build, no archive).
import json
N, L, K = 8, 10, 20000
T = (N**L - 1) // (N - 1)
view = (T + 7) // 8
doc = {
    "buffers": [{"byteLength": 2 * ((view + 7) // 8 * 8)}],
    "bufferViews": [
        {"buffer": 0, "byteOffset": 0, "byteLength": view},                      # tile bits: only bit 0
        {"buffer": 0, "byteOffset": (view + 7) // 8 * 8, "byteLength": view},    # all-zero content bits
    ],
    "tileAvailability": {"bitstream": 0, "availableCount": 1},
    "contentAvailability": [{"bitstream": 1, "availableCount": 0}] * K,
    "childSubtreeAvailability": {"constant": 0},
}
js = json.dumps(doc, separators=(",", ":")).encode()
def nodes(v):
    if isinstance(v, dict): return 1 + sum(nodes(x) for x in v.values())
    if isinstance(v, list): return 1 + sum(nodes(x) for x in v)
    return 1
member = 24 + (len(js) + 7) // 8 * 8 + doc["buffers"][0]["byteLength"]
tileset_nodes = 1 + 2 * K  # contents array of {"uri": ...}
limits = {"json_bytes": 8 << 20, "document_items": 65536, "member_bytes": 64 << 20,
          "document_decoded_bytes(live path)": 64 << 20}
out = {
    "T_bits": T, "view_bytes": view, "subtree_json_bytes": len(js), "subtree_json_nodes": nodes(doc),
    "subtree_member_bytes": member, "tileset_contents_nodes": tileset_nodes,
    "tile_popcount_reserved": 1, "available_contents": 0,
    "fits": {
        "json_bytes": len(js) <= limits["json_bytes"],
        "subtree_nodes": nodes(doc) <= limits["document_items"],
        "tileset_nodes": tileset_nodes + 20 <= limits["document_items"],
        "member_bytes": member <= limits["member_bytes"],
        "live_path": member <= limits["document_decoded_bytes(live path)"],
    },
    "bytes_scanned_by_contract_bit_passes(availableCount + content=>tile, per layer)": 2 * view * K,
    "GiB": round(2 * view * K / 2**30, 1),
}
print(json.dumps(out, indent=1))
