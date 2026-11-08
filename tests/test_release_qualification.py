"""Offline checks for the publisher's native-evidence gate."""
import importlib.util
import json
from pathlib import Path
import re
import tempfile
import tomllib
import unittest


SPEC = importlib.util.spec_from_file_location(
    'publish_release', Path(__file__).resolve().parents[1] / 'scripts/publish-release.py'
)
PUBLISH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PUBLISH)


class QualificationTests(unittest.TestCase):
    def test_bundled_installer_defaults_to_the_release_version(self):
        root = Path(__file__).resolve().parents[1]
        version = tomllib.loads((root / 'Cargo.toml').read_text())['package']['version']
        installer = (root / 'scripts/install-release.sh').read_text()
        self.assertIn(f'version=${{2:-{version}}}', installer)

    def test_workflow_uploads_every_required_summary_and_referenced_smokes_exist(self):
        root = Path(__file__).resolve().parents[1]
        workflow = (root / '.github/workflows/rust.yml').read_text()
        self.assertIn('workflow_dispatch', workflow)
        uploaded = set(re.findall(r'runner\.temp \}\}/([^/]+)/summary\.json', workflow))
        version = tomllib.loads((root / 'Cargo.toml').read_text())['package']['version']
        self.assertEqual(uploaded, PUBLISH.required_summaries('v' + version, 'x86_64'))
        for script in re.findall(r'(?:python3|node) (evaluation/[^\s]+)', workflow):
            self.assertTrue((root / script).is_file(), f'missing workflow smoke {script}')

    def test_historical_versions_keep_their_original_matrix(self):
        self.assertEqual(len(PUBLISH.required_summaries('v0.3.0', 'x86_64')), 10)
        self.assertEqual(len(PUBLISH.required_summaries('v0.3.0', 'aarch64')), 10)
        self.assertEqual(len(PUBLISH.required_summaries('v0.3.1', 'x86_64')), 12)
        self.assertEqual(len(PUBLISH.required_summaries('v0.3.1', 'aarch64')), 11)

    def test_current_release_requires_all_creator_evidence_on_both_architectures(self):
        x86 = PUBLISH.required_summaries('v0.4.0', 'x86_64')
        arm = PUBLISH.required_summaries('v0.4.0', 'aarch64')
        self.assertEqual(len(x86), 35)
        self.assertEqual(len(arm), 30)
        self.assertEqual(x86 - arm, {'browser', 'preserve-browser', 'review-duration', 'review-compare', 'review-import'})
        self.assertTrue({'inspect-range', 'captions-extra', 'caption-sidecar', 'solo',
                         'subject-reframe', 'compare', 'creator-research', 'mcp-media', 'host-readiness',
                         'discovery', 'provider-search', 'thumbnail', 'word-alignment', 'translation', 'audio-cleanup',
                         'delivery-batch'} <= arm)
        for tag, arch in [('v0.4', 'x86_64'), ('0.4.0', 'x86_64'), ('v0.4.0', 'other')]:
            with self.assertRaises(ValueError):
                PUBLISH.required_summaries(tag, arch)

    def test_source_views_are_required_for_new_release_without_changing_old_matrix(self):
        self.assertEqual(len(PUBLISH.required_summaries('v0.5.0', 'x86_64')), 36)
        self.assertEqual(len(PUBLISH.required_summaries('v0.5.0', 'aarch64')), 31)
        self.assertIn('source-views', PUBLISH.required_summaries('v0.5.0', 'aarch64'))
        self.assertNotIn('source-views', PUBLISH.required_summaries('v0.4.0', 'aarch64'))

    def test_052_requires_cache_and_duplicate_asset_evidence(self):
        self.assertEqual(len(PUBLISH.required_summaries('v0.5.1', 'x86_64')), 36)
        self.assertEqual(len(PUBLISH.required_summaries('v0.5.1', 'aarch64')), 31)
        self.assertEqual(len(PUBLISH.required_summaries('v0.5.2', 'x86_64')), 39)
        self.assertEqual(len(PUBLISH.required_summaries('v0.5.2', 'aarch64')), 33)
        self.assertIn('asr-cache', PUBLISH.required_summaries('v0.5.2', 'aarch64'))
        self.assertIn('asr-clock', PUBLISH.required_summaries('v0.5.2', 'aarch64'))
        self.assertIn('duplicate-assets', PUBLISH.required_summaries('v0.5.2', 'x86_64'))
        self.assertNotIn('duplicate-assets', PUBLISH.required_summaries('v0.5.2', 'aarch64'))

    def test_asr_legacy_compatibility_must_be_exercised(self):
        required = PUBLISH.required_summaries('v0.5.2', 'aarch64')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in required:
                (root / name).mkdir()
                (root / name / 'summary.json').write_text('{"passed": true}')
            clock = root / 'asr-clock' / 'summary.json'
            clock.write_text(json.dumps({'passed': True, 'legacy': {'v2Exercised': True}}))
            target = root / 'asr-cache' / 'summary.json'
            for status in (None, 'not-exercised', 'failed'):
                target.write_text(json.dumps({'passed': True, 'legacyV2Compatibility': status}))
                with self.assertRaises(ValueError):
                    PUBLISH.verify_summaries(root, required)
            target.write_text(json.dumps({'passed': True, 'legacyV2Compatibility': 'passed'}))
            PUBLISH.verify_summaries(root, required)
            for status in (None, False, 'passed'):
                clock.write_text(json.dumps({'passed': True, 'legacy': {'v2Exercised': status}}))
                with self.assertRaises(ValueError):
                    PUBLISH.verify_summaries(root, required)
            clock.write_text(json.dumps({'passed': True, 'legacy': {'v2Exercised': True}}))
            PUBLISH.verify_summaries(root, required)

    def test_missing_failed_malformed_and_unexpected_summaries_cannot_pass(self):
        required = PUBLISH.required_summaries('v0.4.0', 'x86_64')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in required:
                (root / name).mkdir()
                (root / name / 'summary.json').write_text('{"passed": true}')
            PUBLISH.verify_summaries(root, required)
            target = root / 'mcp-media' / 'summary.json'
            for content in ('{"passed":false}', '{"passed":1}', '{}', '{bad json'):
                target.write_text(content)
                with self.assertRaises(ValueError):
                    PUBLISH.verify_summaries(root, required)
            target.unlink()
            with self.assertRaises(ValueError):
                PUBLISH.verify_summaries(root, required)
            target.write_text('{"passed":true}')
            (root / 'unexpected').mkdir()
            (root / 'unexpected' / 'summary.json').write_text(json.dumps({'passed': True}))
            with self.assertRaises(ValueError):
                PUBLISH.verify_summaries(root, required)


if __name__ == '__main__':
    unittest.main()
