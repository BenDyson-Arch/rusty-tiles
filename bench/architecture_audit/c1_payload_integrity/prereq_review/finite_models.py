"""Independent finite lexical/size models; no production calls or acceptance."""
import base64
import json
from decimal import Decimal

TOKENS = ["3", "3.0", "3e0", "3.0000000000000001", "30000000000000001e-16",
          "5126.0", "5.126e3", "-0.0", "-3", "1e-99999", "0e99999",
          "18446744073709551615.0", "18446744073709551616.0",
          "18446744073709551614.9", "9007199254740993.0"]

def exact_unsigned(token, maximum=(1 << 64) - 1):
    # Size-limited exponent, linear digit examination, no integer power expansion.
    sign = token.startswith("-")
    body = token.lstrip("-")
    mantissa, _, exp = body.lower().partition("e")
    whole, _, fraction = mantissa.partition(".")
    digits = (whole + fraction).lstrip("0")
    if not digits:
        return "admitted", 0
    if sign:
        return "invalid", None
    # Beyond this cap, a nonzero finite token is either fractional or too large.
    exponent = 0 if not exp else (int(exp) if len(exp.lstrip("+-0")) <= 6 else (10**6 if not exp.startswith("-") else -10**6))
    scale = exponent - len(fraction)
    if scale < 0:
        needed = -scale
        if needed >= len(digits) or any(c != "0" for c in digits[-needed:]):
            return "invalid", None
        digits = digits[:-needed]
    elif len(digits) + scale > len(str(maximum)):
        return "range", None
    else:
        digits += "0" * scale
    if len(digits) > len(str(maximum)) or (len(digits) == len(str(maximum)) and digits > str(maximum)):
        return "range", None
    return "admitted", int(digits)

def canonical_base64(encoded):
    if not encoded or len(encoded) % 4:
        return None
    pad = len(encoded) - len(encoded.rstrip("="))
    interior = encoded[:-pad] if pad else encoded
    if pad > 2 or "=" in interior:
        return None
    try:
        decoded = base64.b64decode(encoded, validate=True)
    except ValueError:
        return None
    # Explicit roundtrip rejects noncanonical trailing low bits.
    if base64.b64encode(decoded).decode() != encoded:
        return None
    maximum = len(encoded) // 4 * 3
    assert len(decoded) == maximum - pad
    assert maximum <= len(decoded) + 2
    return {"actual": len(decoded), "allocation_estimate": maximum, "pad": pad}

result = {"claim": "Finite independent arithmetic model only", "numbers": [], "base64": {}}
for token in TOKENS:
    classification, value = exact_unsigned(token)
    d = Decimal(token)
    integral = d == d.to_integral_value()
    assert (classification != "invalid") == (integral and d >= 0)
    if value is not None:
        assert Decimal(value) == d
    result["numbers"].append({"token": token, "classification": classification, "value": value,
                               "float_appears_integer": float(d).is_integer()})
for token in ["AA==", "AAA=", "AAAA", "AB==", "AAB=", "AA=", "AA===", "=AAA", "AAAA\n", "???=", ""]:
    result["base64"][token] = canonical_base64(token)
print(json.dumps(result, indent=2))
