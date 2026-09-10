import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "scripts"))

import srt_tools  # noqa: E402

SRT = """1
00:00:00,000 --> 00:00:02,500
Hello <b>world</b> &amp; friends

2
00:00:02,500 --> 00:00:05,000
second line
"""

VTT = """WEBVTT
Kind: captions
Language: en

00:00:00.000 --> 00:00:02.500 align:start position:0%
Hello world

00:00:02.500 --> 00:00:05.000
second line
"""

ROLLING = """1
00:00:00,000 --> 00:00:01,000
Hello world

2
00:00:01,000 --> 00:00:02,000
Hello world this is

3
00:00:02,000 --> 00:00:03,000
this is a test
"""


class ParseTests(unittest.TestCase):
    def test_parses_srt_and_strips_markup(self):
        cues = srt_tools.parse_cues(SRT)
        self.assertEqual(len(cues), 2)
        self.assertEqual(cues[0].start, 0.0)
        self.assertEqual(cues[0].end, 2.5)
        self.assertEqual(cues[0].text, "Hello world & friends")
        self.assertEqual(cues[1].text, "second line")

    def test_parses_vtt_with_cue_settings(self):
        cues = srt_tools.parse_cues(VTT)
        self.assertEqual(len(cues), 2)
        self.assertEqual(cues[0].text, "Hello world")
        self.assertEqual(cues[1].start, 2.5)

    def test_hours_are_parsed(self):
        cues = srt_tools.parse_cues("1\n01:02:03,004 --> 01:02:04,000\nx\n")
        self.assertAlmostEqual(cues[0].start, 3723.004)


    def test_parses_cues_without_blank_separators(self):
        # Some models emit consecutive cues with no blank line between blocks.
        raw = ("1\n00:00:00,000 --> 00:00:02,000\nfirst line\n"
               "2\n00:00:02,000 --> 00:00:04,000\nsecond line\n")
        cues = srt_tools.parse_cues(raw)
        self.assertEqual(len(cues), 2)
        self.assertEqual(cues[0].text, "first line")
        self.assertEqual(cues[1].text, "second line")

    def test_final_numeric_caption_is_kept(self):
        raw = "1\n00:00:01,000 --> 00:00:02,000\n2024\n"
        cues = srt_tools.parse_cues(raw)
        self.assertEqual(cues[0].text, "2024")


class TranscriptTests(unittest.TestCase):
    def test_rolling_captions_are_deduplicated(self):
        cues = srt_tools.parse_cues(ROLLING)
        self.assertEqual(srt_tools.to_text(cues), "Hello world this is a test")

    def test_markdown_has_time_markers_per_interval(self):
        cues = [
            srt_tools.Cue(0.0, 2.0, "one"),
            srt_tools.Cue(30.0, 32.0, "two"),
            srt_tools.Cue(70.0, 72.0, "three"),
        ]
        md = srt_tools.to_markdown(cues, interval=60)
        self.assertIn("[00:00] one two", md)
        self.assertIn("[01:10] three", md)

    def test_markdown_marker_uses_hours_when_needed(self):
        cues = [srt_tools.Cue(3725.0, 3726.0, "late")]
        self.assertIn("[1:02:05] late", srt_tools.to_markdown(cues, interval=60))


class ConcatTests(unittest.TestCase):
    def test_offset_and_render_round_trip(self):
        cues = srt_tools.parse_cues(SRT)
        shifted = srt_tools.offset_cues(cues, 1200.0)
        srt = srt_tools.render_srt(shifted)
        again = srt_tools.parse_cues(srt)
        self.assertEqual(again[0].start, 1200.0)
        self.assertEqual(again[1].end, 1205.0)
        self.assertIn("00:20:00,000 --> 00:20:02,500", srt)


if __name__ == "__main__":
    unittest.main()
