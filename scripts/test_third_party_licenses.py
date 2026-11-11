#!/usr/bin/env python3
"""Unit tests for the licence allow-list and text splitting in third_party_licenses.py."""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import third_party_licenses as tpl  # noqa: E402


def chosen(expression):
    return tpl.choose(tpl.parse_expression(expression))


class AllowList(unittest.TestCase):
    def test_permissive_expressions_pass(self):
        self.assertEqual(chosen("MIT OR Apache-2.0"), [("MIT", None)])
        self.assertEqual(chosen("Apache-2.0/MIT"), [("MIT", None)])
        self.assertEqual(chosen("(MIT OR Apache-2.0) AND Unicode-3.0"), [("MIT", None), ("Unicode-3.0", None)])
        self.assertTrue(chosen("Apache-2.0 WITH LLVM-exception"))

    def test_copyleft_alternative_is_never_chosen(self):
        self.assertEqual(chosen("MIT OR Apache-2.0 OR LGPL-2.1-or-later"), [("MIT", None)])

    def test_copyleft_unknown_and_exceptions_fail(self):
        for expression in ("GPL-3.0-only", "AGPL-3.0-or-later", "LGPL-2.1-or-later", "MIT AND GPL-2.0-only",
                           "SSPL-1.0", "LicenseRef-custom", "MIT WITH Classpath-exception-2.0"):
            self.assertIsNone(chosen(expression), expression)

    def test_unbalanced_expression_is_rejected(self):
        with self.assertRaises(ValueError):
            tpl.parse_expression("(MIT OR Apache-2.0")


class Splitting(unittest.TestCase):
    def test_mit_notice_is_separated_from_the_standard_body(self):
        text = "MIT License\n\nCopyright (c) 2020 A\nCopyright (c) 2021 B\n\n" + tpl.SPDX["MIT"].split("\n\n", 1)[1]
        (notice, body), = tpl.split_text(text)
        self.assertEqual(notice, "Copyright (c) 2020 A; Copyright (c) 2021 B")
        self.assertTrue(body.startswith("Permission is hereby granted"))

    def test_apache_template_appendix_is_dropped_but_custom_notices_stay(self):
        terms = "                              Apache License\n   Version 2.0\n\nTERMS\n\nEND OF TERMS AND CONDITIONS\n"
        self.assertEqual(len(tpl.split_text(terms + "\nAPPENDIX\nCopyright [yyyy] [name of copyright owner]\n")), 1)
        self.assertEqual(len(tpl.split_text(terms + "\nCopyright 2019 Someone\n")), 2)

    def test_unrecognised_text_is_kept_whole(self):
        self.assertEqual(tpl.split_text("Custom terms."), [("", "Custom terms.")])


if __name__ == "__main__":
    unittest.main()
