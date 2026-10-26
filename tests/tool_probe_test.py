"""Provider qualification of tool updates, with every subprocess replaced."""
import importlib.util
import json
from pathlib import Path
import unittest
from unittest import mock


SOURCE = Path(__file__).resolve().parents[1] / "apps/server/src/tools/package.py"
SPEC = importlib.util.spec_from_file_location("package", SOURCE)
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)

OFFLINE = "Package subprocess failed: error: No playable streams found on this URL"
BROKEN = "Package subprocess failed: ModuleNotFoundError: No module named 'streamlink.plugins'"


def streamlink(outcomes):
    """Plays the CLI and the resident worker of each package per channel."""
    def run(args, timeout=60, cwd=None, request=None):
        generation = Path(args[0]).parts[-4]
        channel = json.loads(request)["channel"] if request else args[-2].rsplit("/", 1)[1]
        outcome = outcomes.get((generation, channel), OFFLINE)
        if outcome != "live":
            raise RuntimeError(outcome)
        if request:
            variants = {"is_live": True, "duration": 0, "formats": [{"url": "https://video.invalid/1080p60.m3u8"}]}
            return '{"ready":true}\n' + json.dumps(variants) + "\n"
        return "https://video.invalid/best.m3u8\n"
    return run


def packages(generation):
    return {"streamlink": {"id": generation, "executables": {"python": "env/bin/python"}}}


class StreamlinkQualification(unittest.TestCase):
    def probe(self, outcomes, current=packages("current")):
        value = {"kind": "streamlink", "candidate": packages("candidate"), "current": current, "streamlink_script": ""}
        with mock.patch.object(package, "run", streamlink(outcomes)):
            return package.probe(Path("/tools"), value)

    def test_a_live_channel_qualifies_the_candidate_from_its_worker_variants(self):
        result = self.probe({("candidate", "lck"): "live", ("candidate", "xqc"): "live"})
        self.assertTrue(result["ready"])
        twitch, kick = result["probes"]
        self.assertEqual([a["identifier"] for a in twitch["attempts"]], ["monstercat", "lck"])
        self.assertEqual(twitch["candidate"]["status"], "passed")
        self.assertEqual(kick["candidate"]["status"], "passed")

    def test_channels_offline_for_both_versions_show_no_regression(self):
        result = self.probe({})
        self.assertTrue(result["ready"])
        self.assertTrue(all(p["unchanged"] and p["candidate"]["reason"] == "offline_or_removed" for p in result["probes"]))

    def test_a_channel_only_the_current_version_plays_blocks_the_candidate(self):
        result = self.probe({("current", "riotgames"): "live"})
        self.assertFalse(result["ready"])
        self.assertFalse(result["probes"][0]["unchanged"])

    def test_a_failure_unlike_the_current_version_blocks_the_candidate(self):
        result = self.probe({("candidate", channel): BROKEN for channel in ("lofi", "xqc", "adinross")})
        self.assertFalse(result["ready"])
        self.assertEqual(result["probes"][1]["candidate"]["reason"], "tool_invocation_failed")

    def test_without_an_installed_version_offline_channels_stay_inconclusive(self):
        self.assertFalse(self.probe({}, current={"streamlink": None})["ready"])


class YoutubeQualification(unittest.TestCase):
    def test_a_failure_shared_with_the_current_version_still_blocks_the_candidate(self):
        def packages(generation):
            return {"yt-dlp": {"id": generation, "executables": {"yt_dlp": "yt-dlp", "module": "yt-dlp-module"}},
                    "deno": {"id": generation, "executables": {"deno": "deno"}}}
        value = {"kind": "youtube", "candidate": packages("candidate"), "current": packages("current"), "youtube_script": ""}
        blocked = RuntimeError("Package subprocess failed: Sign in to confirm you're not a bot")
        with mock.patch.object(package, "run", side_effect=blocked):
            result = package.probe(Path("/tools"), value)
        self.assertFalse(result["ready"])
        self.assertEqual(result["probes"][0]["candidate"]["reason"], "network_blocked")
