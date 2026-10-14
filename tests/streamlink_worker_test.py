"""Offline provider extraction coverage through the resident worker protocol."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "apps/server/src/online/streamlink_worker.py"
MODULE = '''
from types import SimpleNamespace as S
class Stream:
    def __init__(self, name, width, height, fps, codecs):
        self.url = 'https://video.ttvnw.net/' + name
        info = S(resolution=S(width=width, height=height), framerate=fps,
                 bandwidth=20000000, codecs=codecs)
        self.multivariant = S(playlists=[S(uri=self.url, stream_info=info, is_iframe=False)])
    def to_url(self): return self.url
class Streamlink:
    def set_option(self, *args): pass
    def streams(self, address):
        variants = {
            '1080p': Stream('1080p', 1920, 1080, 29.97, ['avc1.640028', 'mp4a.40.2']),
            '1080p60': Stream('1080p60', 1920, 1080, 59.94, ['avc1.64002a', 'mp4a.40.2']),
            '1440p60': Stream('1440p60', 2560, 1440, 60, ['avc1.640033', 'mp4a.40.2']),
            '2160p60': Stream('2160p60', 3840, 2160, 60, ['av01.0.13M.08', 'mp4a.40.2']),
            '4320p': Stream('4320p', 7680, 4320, 24, ['av01.0.16M.08', 'mp4a.40.2']),
            '720p': Stream('720p', 1280, 720, None, None),
        }
        return {**variants, 'best': variants['4320p'], 'worst': variants['1080p'],
                'audio_only': Stream('audio', 0, 0, 0, ['mp4a.40.2'])}
'''


class ProviderVariants(unittest.TestCase):
    def test_worker_preserves_all_provider_variants_and_exact_frame_rates(self):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "streamlink.py").write_text(MODULE)
            code = "import sys; sys.path.insert(0, sys.argv[1]); " + SCRIPT.read_text()
            result = subprocess.run(
                [sys.executable, "-I", "-B", "-u", "-c", code, directory],
                input=json.dumps({"provider": "twitch", "channel": "fixture"}) + "\n",
                text=True, capture_output=True, timeout=10,
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        frames = [json.loads(line) for line in result.stdout.splitlines()]
        formats = frames[1]["formats"]
        self.assertEqual(len(formats), 6)
        self.assertEqual({f["height"] for f in formats}, {720, 1080, 1440, 2160, 4320})
        self.assertEqual(next(f for f in formats if f["format_id"] == "1080p")["fps"], 29.97)
        self.assertEqual(next(f for f in formats if f["format_id"] == "1080p60")["fps"], 59.94)
        self.assertEqual(next(f for f in formats if f["height"] == 4320)["vcodec"], "av01.0.16M.08")
        self.assertIsNone(next(f for f in formats if f["height"] == 720)["fps"])
