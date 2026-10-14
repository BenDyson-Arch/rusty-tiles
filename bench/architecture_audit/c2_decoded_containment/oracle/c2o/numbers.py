"""Exact number handling shared by every oracle module.

Every JSON number is read as the nearest binary64 value of its text (contract,
"Numbers"). An exact integer representation (1.0, 1e0, -0.0, ...) is read as
that integer. A literal that overflows binary64 becomes an Overflow marker that
fails any consuming field; a NaN/Infinity token is an error. Exact values are `fractions.Fraction`; binary64 and f32 values convert
to them without rounding.
"""

import json
import math
import struct
import re
from fractions import Fraction


class JsonNumberError(ValueError):
    """A JSON number literal that has no finite binary64 value."""


class OracleUndecided(Exception):
    """The oracle cannot reach a verdict (limit, unmodelled input or C1-level defect)."""

    def __init__(self, why, **facts):
        super().__init__(why)
        self.why = why
        self.facts = facts


class Overflow:
    """A JSON number literal with no finite binary64 value (e.g. 1e400).

    It is not a number for any consumer: a consumed field holding it fails that
    field's type check, while opaque JSON (extras, extension-owned data) may hold
    it, as in C1's admitted profile."""

    def __init__(self, text):
        self.text = text

    def __repr__(self):
        return "Overflow(%s)" % self.text[:40]


_NUMBER = re.compile(r"(-?)(\d+)(?:\.(\d+))?(?:[eE]([+-]?\d+))?")


def _exact_integer(text):
    """The integer whose exact decimal value the literal denotes, or None.

    C1 admits exact integer representations such as 1.0, 1e0, 10e-1, -0.0 and
    0e999...; they stand for the same integer as the plain literal."""
    match = _NUMBER.fullmatch(text)
    if match is None:
        return None
    sign, whole, frac, exp = match.group(1), match.group(2), match.group(3) or "", match.group(4)
    digits = (whole + frac).lstrip("0")
    if not digits:
        return 0  # mathematical zero, whatever the exponent
    exponent = int(exp or "0") - len(frac)
    if exponent >= 0:
        value = int(digits) * 10 ** exponent  # finite binary64, so the size is bounded
    else:
        k = -exponent
        if k >= len(digits) or digits[-k:].strip("0"):
            return None
        value = int(digits[:-k])
    return -value if sign else value


def _parse_float(text):
    value = float(text)
    if not math.isfinite(value):
        return Overflow(text)
    integer = _exact_integer(text)
    return value if integer is None else integer


def _parse_int(text):
    value = int(text)
    try:
        float(value)
    except OverflowError:
        return Overflow(text)
    return value


def _reject_constant(name):
    raise JsonNumberError("non-JSON constant %s" % name)


def contains_overflow(obj):
    if isinstance(obj, Overflow):
        return True
    if isinstance(obj, dict):
        return any(contains_overflow(v) for v in obj.values())
    if isinstance(obj, list):
        return any(contains_overflow(v) for v in obj)
    return False


def load_json(data):
    """Parse UTF-8 JSON bytes with the contract's number rules."""
    if isinstance(data, (bytes, bytearray, memoryview)):
        text = bytes(data).decode("utf-8")
    else:
        text = data
    return json.loads(
        text,
        parse_float=_parse_float,
        parse_int=_parse_int,
        parse_constant=_reject_constant,
    )


def value_nodes(obj):
    """JSON value nodes of a parsed document (C2-WB `J`): the root plus every
    scalar and container value; object member names are not value nodes
    (C1 contract, "JSON value nodes"). Iterative, so depth is not a limit."""
    count = 0
    stack = [obj]
    while stack:
        value = stack.pop()
        count += 1
        if isinstance(value, dict):
            stack.extend(value.values())
        elif isinstance(value, list):
            stack.extend(value)
    return count


def is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def is_index(value):
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def exact(value):
    """Exact rational of the nearest binary64 value of a parsed JSON number."""
    if not is_number(value):
        raise TypeError("not a number: %r" % (value,))
    return Fraction(float(value))


def f32_scaled(bits):
    """Integer v with f32(bits) == v * 2**-149, or None for NaN/Inf."""
    sign = -1 if bits >> 31 else 1
    exponent = (bits >> 23) & 0xFF
    mantissa = bits & 0x7FFFFF
    if exponent == 0xFF:
        return None
    if exponent == 0:
        return sign * mantissa
    return sign * ((mantissa | 0x800000) << (exponent - 1))


F32_SCALE = Fraction(1, 2 ** 149)


def f32_exact(bits):
    v = f32_scaled(bits)
    return None if v is None else v * F32_SCALE


def f32_bits(value):
    return struct.unpack("<I", struct.pack("<f", value))[0]


def lowbit_highbit(x):
    """(exponent of lowest set bit, floor(log2|x|)) of a nonzero dyadic rational."""
    x = Fraction(x)
    if x == 0:
        raise ValueError("zero has no pair")
    num = abs(x.numerator)
    den = x.denominator
    if den & (den - 1):
        raise ValueError("value is not dyadic: %s" % x)
    d = den.bit_length() - 1
    low = (num & -num).bit_length() - 1
    return (low - d, num.bit_length() - 1 - d)


def frac_str(x):
    x = Fraction(x)
    return str(x.numerator) if x.denominator == 1 else "%d/%d" % (x.numerator, x.denominator)
