"""Offline protocol tests: python -B -m unittest discover -s tests -p '*_test.py'."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "apps/server/src/online/youtube_worker.py"
FAKE_MODULE = '''
from .globals import plugin_dirs
class YoutubeDL:
    def __init__(self, options):
        assert not plugin_dirs.value
        assert not options['cachedir'] and not options['remote_components']
        assert not options.get('cookiefile') and not options.get('cookiesfrombrowser')
        assert options['js_runtimes'] == {'deno': {'path': 'managed-deno'}}
        self.calls = 0
    def __enter__(self): return self
    def __exit__(self, *args): pass
    def extract_info(self, address, download):
        assert not download
        self.calls += 1
        video = address.split('=')[-1]
        if video == 'private0000': raise RuntimeError('Sign in secret-token')
        if video == 'removed0000': raise RuntimeError('Video unavailable secret-url')
        if video == 'failure0000': raise RuntimeError('Internal error secret-cookie')
        if video == 'bigdata0000': return {'id': video, 'large': 'x' * (8 * 1024 * 1024)}
        return {'id': video, 'calls': self.calls}
    def sanitize_info(self, value): return value
'''


class WorkerProtocol(unittest.TestCase):
    def run_worker(self, requests, version="test"):
        with tempfile.TemporaryDirectory() as home:
            module = Path(home) / "yt_dlp"
            module.mkdir()
            (module / "__init__.py").write_text(FAKE_MODULE)
            (module / "version.py").write_text("__version__ = 'test'")
            (module / "globals.py").write_text("class PluginDirs: value = ['default']\nplugin_dirs = PluginDirs()")
            env = {key: home for key in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "TMP", "TEMP"]}
            env.update({key: os.environ[key] for key in ["SystemRoot", "WINDIR"] if key in os.environ})
            result = subprocess.run(
                [sys.executable, "-I", "-u", "-c", SCRIPT.read_text(), home, "managed-deno", version],
                input="".join(json.dumps(request) + "\n" for request in requests), text=True,
                capture_output=True, timeout=10, cwd=home, env=env,
            )
            return result, [json.loads(line) for line in result.stdout.splitlines()]

    def test_reuses_instance_after_success_and_provider_failure(self):
        result, frames = self.run_worker([{"id": video} for video in ["9pkqztOC1WM", "private0000", "sOnfbsuwPGo"]])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, "")
        self.assertEqual(frames, [{"ready": True, "version": "test"},
            {"metadata": {"id": "9pkqztOC1WM", "calls": 1}},
            {"error": "extractor_authentication_required"},
            {"metadata": {"id": "sOnfbsuwPGo", "calls": 3}}])

    def test_errors_and_oversized_metadata_are_redacted(self):
        result, frames = self.run_worker([{"id": video} for video in ["removed0000", "failure0000", "bigdata0000", "invalid&url"]])
        self.assertEqual(result.returncode, 0)
        self.assertNotIn("secret", result.stdout + result.stderr)
        self.assertEqual([frame["error"] for frame in frames[1:]],
                         ["unavailable", "extraction_failed", "extraction_failed", "extraction_failed"])

    def test_version_mismatch_never_reports_ready(self):
        result, frames = self.run_worker([], version="new")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(frames, [])
