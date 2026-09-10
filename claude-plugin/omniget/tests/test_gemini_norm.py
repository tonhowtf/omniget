import importlib.util
import os
import sys
import unittest

HERE = os.path.dirname(__file__)
SCRIPTS = os.path.join(HERE, "..", "scripts")
sys.path.insert(0, SCRIPTS)

import srt_tools  # noqa: E402

spec = importlib.util.spec_from_file_location("gemini_transcribe", os.path.join(SCRIPTS, "gemini_transcribe.py"))
gemini = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gemini)


class NormalizeTests(unittest.TestCase):
    def _first(self, raw):
        cues = srt_tools.parse_cues(gemini.normalize_srt_timestamps(raw))
        self.assertTrue(cues, "no cue parsed from %r" % raw)
        return srt_tools.fmt_srt_ts(cues[0].start), srt_tools.fmt_srt_ts(cues[0].end)

    def test_mm_ss_ms_with_colon(self):
        self.assertEqual(self._first("1\n00:17:200 --> 00:18:400\nx\n"), ("00:00:17,200", "00:00:18,400"))

    def test_dot_millis(self):
        self.assertEqual(self._first("1\n00:00:01.500 --> 00:00:03.000\nx\n"), ("00:00:01,500", "00:00:03,000"))

    def test_no_millis_hms(self):
        self.assertEqual(self._first("1\n01:02:03 --> 01:02:05\nx\n"), ("01:02:03,000", "01:02:05,000"))

    def test_bare_mm_ss(self):
        self.assertEqual(self._first("1\n00:05 --> 00:09\nx\n"), ("00:00:05,000", "00:00:09,000"))

    def test_correct_form_untouched(self):
        self.assertEqual(self._first("1\n00:00:17,200 --> 00:00:18,400\nx\n"), ("00:00:17,200", "00:00:18,400"))

    def test_non_timestamp_lines_survive(self):
        raw = "WEBVTT stays\n1\n00:00:01,000 --> 00:00:02,000\nhello\n"
        self.assertIn("hello", gemini.normalize_srt_timestamps(raw))


if __name__ == "__main__":
    unittest.main()
