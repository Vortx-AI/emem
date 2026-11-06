#!/usr/bin/env python3
"""Tests for scripts/ste_lint.py. Standard library only: python3 scripts/ste_lint_test.py"""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ste_lint as doc_lint  # noqa: E402

RULES = doc_lint.load_rules()


def lint_text(md):
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
        f.write(md)
    return doc_lint.lint(Path(f.name), RULES)["findings"]


def rules_of(md):
    return sorted({f["rule"] for f in lint_text(md)})


class DocLint(unittest.TestCase):
    def test_clean_text_passes(self):
        self.assertEqual(rules_of("# Read a fact\n\nSend the request. The answer has a signed receipt.\n"), [])

    def test_dash(self):
        self.assertIn("dash", rules_of("A fact is signed — always.\n"))
        self.assertIn("dash", rules_of("Pages 3–5 hold the table.\n"))

    def test_dash_in_code_is_ignored(self):
        self.assertNotIn("dash", rules_of("Run `a — b` now.\n\n```\nx — y\n```\n"))

    def test_long_instruction(self):
        long = "Send " + " ".join(["the"] * 22) + " request.\n"
        self.assertIn("sentence-length", rules_of(long))

    def test_banned_word(self):
        self.assertIn("banned-word", rules_of("We utilize the receipt to check the answer.\n"))

    def test_ai_tell(self):
        self.assertIn("ai-tell", rules_of("emem offers a seamless way to share facts.\n"))
        self.assertIn("ai-tell", rules_of("History is auditable, not just asserted.\n"))

    def test_urls_and_code_skipped(self):
        self.assertEqual(rules_of("Open [the page](https://example.com/utilize-delve) and run `utilize`.\n"), [])

    def test_line_numbers(self):
        f = [x for x in lint_text("Line one is fine.\n\nThis uses a seamless flow.\n") if x["rule"] == "ai-tell"]
        self.assertEqual(f[0]["line"], 3)

    def test_cli_exit_codes(self):
        with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
            f.write("A seamless flow.\n")
        script = str(Path(doc_lint.__file__))
        self.assertEqual(subprocess.run([sys.executable, script, f.name], capture_output=True).returncode, 0)
        self.assertEqual(subprocess.run([sys.executable, script, "--fail-on", "error", f.name],
                                        capture_output=True).returncode, 1)


if __name__ == "__main__":
    unittest.main()
