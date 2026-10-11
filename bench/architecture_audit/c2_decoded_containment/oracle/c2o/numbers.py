"""Exact number handling shared by every oracle module.

Every JSON number is read as the nearest binary64 value of its text (contract,
"Numbers"). A literal that overflows binary64, or a NaN/Infinity token, is an
error. Exact values are `fractions.Fraction`; binary64 and f32 values convert
to them without rounding.
"""

import json
import math
import struct
from fractions import Fraction


class JsonNumberError(ValueError):
    """A JSON number literal that has no finite binary64 value."""


class OracleUndecided(Exception):
    """The oracle cannot reach a verdict (limit, unmodelled input or C1-level defect)."""

    def __init__(self, why, **facts):
        super().__init__(why)
        self.why = why
        self.facts = facts


def _parse_float(text):
    value = float(text)
    if not math.isfinite(value):
        raise JsonNumberError("number literal overflows binary64: %s" % text[:40])
    return value


def _parse_int(text):
    value = int(text)
    try:
        float(value)
    except OverflowError:
        raise JsonNumberError("integer literal overflows binary64: %s..." % text[:40])
    return value


def _reject_constant(name):
    raise JsonNumberError("non-JSON constant %s" % name)


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
