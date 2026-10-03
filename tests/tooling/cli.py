#!/usr/bin/env python3
"""Exercise the public build CLI in an isolated checkout with a fake compiler."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class IntegrationFixtures(unittest.TestCase):
    def test_embedded_files_resolve_on_every_platform(self):
        for source in (ROOT / 'tests/integration').rglob('*.rs'):
            for relative in re.findall(r'include_(?:bytes|str)!\("([^"]+)"\)', source.read_text()):
                with self.subTest(source=source.name, fixture=relative):
                    self.assertTrue((source.parent / relative).is_file(), relative)


class BuildCli(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='carlitos-cli-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        shutil.copy2(ROOT / 'build.sh', self.root)
        (self.root / 'scripts').mkdir()
        shutil.copy2(ROOT / 'scripts/menu.sh', self.root / 'scripts')
        (self.root / 'bin').mkdir()
        cargo = self.root / 'bin/cargo'
        cargo.write_text('#!/usr/bin/env bash\nprintf "cargo:%s\\n" "$*"\n'
                         'echo "compiler diagnostic" >&2\nexit "${FAKE_EXIT:-0}"\n')
        cargo.chmod(0o755)
        self.env = dict(os.environ, CARLITOS_DEV_SHELL='1',
                        PATH=f'{self.root}/bin:{os.environ["PATH"]}')
        self.env.pop('CARLITOS_BUILD_LOG_ACTIVE', None)

    def run_cli(self, *args, input=None, code=0):
        result = subprocess.run(['./build.sh', *args], cwd=self.root, env=self.env,
                                input=input, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        return result.stdout

    def test_logs_capture_both_streams_and_replace_old_run(self):
        self.run_cli('test', 'unit', 'old-filter')
        log = self.root / 'target/logs/test-unit.log'
        self.assertIn('old-filter', log.read_text())
        output = self.run_cli('test', 'unit', 'new-filter')
        self.assertNotIn('old-filter', log.read_text())
        self.assertIn('cargo:test --locked new-filter', log.read_text())
        self.assertIn('compiler diagnostic', log.read_text())
        self.assertIn('compiler diagnostic', output)

    def test_pipeline_preserves_failure(self):
        self.env['FAKE_EXIT'] = '42'
        self.run_cli('test', 'unit', code=42)
        self.assertIn('compiler diagnostic', (self.root / 'target/logs/test-unit.log').read_text())

    def test_groups_keep_separate_logs(self):
        self.run_cli('test', 'unit')
        self.run_cli('test', 'ui')
        self.assertIn('cargo:test --locked', (self.root / 'target/logs/test-unit.log').read_text())
        self.assertIn('cargo:run --locked -- --self-test', (self.root / 'target/logs/test-ui.log').read_text())

    def test_unknown_group_does_not_run_compiler(self):
        output = self.run_cli('test', 'unknown', code=2)
        self.assertNotIn('cargo:', output)

    def test_menu_opens_groups_without_running_them(self):
        menu = (self.root / 'scripts/menu.sh').read_text()
        main = menu.split('main_items=(', 1)[1].split(')', 1)[0].splitlines()
        choices = [line for line in main if "'" in line]
        test_index = next(i + 1 for i, line in enumerate(choices) if '@tests|' in line)
        output = self.run_cli(input=f'{test_index}\n0\n0\n')
        self.assertIn('группы тестов', output)
        self.assertIn('Android — выбрать группу', output)
        self.assertNotIn('cargo:', output)
        self.assertFalse((self.root / 'target/logs').exists())


class WindowsArtifacts(unittest.TestCase):
    def build(self, names):
        with tempfile.TemporaryDirectory(prefix='carlitos-windows-artifacts-') as temporary:
            root = Path(temporary)
            (root / 'scripts').mkdir()
            shutil.copy2(ROOT / 'scripts/windows.sh', root / 'scripts')
            (root / 'Cargo.toml').write_text('[package]\nversion = "1.0.1"\n')
            (root / 'tests/windows').mkdir(parents=True)
            (root / 'tests/windows/package.py').write_text(
                'import sys\nprint("selected:" + sys.argv[1])\n')
            (root / 'bin').mkdir()
            rustup = root / 'bin/rustup'
            rustup.write_text(
                '#!/usr/bin/env bash\n'
                'if [[ "$*" == *--message-format=json-render-diagnostics* ]]; then\n'
                '  cat "$FAKE_REPORT"\n'
                'fi\n')
            rustup.chmod(0o755)
            xvfb = root / 'bin/xvfb-run'
            xvfb.write_text('#!/usr/bin/env bash\nprintf "installer:%s\\n" "$*"\n')
            xvfb.chmod(0o755)
            report = root / 'report.json'
            events = [{'reason': 'build-script-executed', 'package_id': 'carlitos',
                       'out_dir': 'release-assets'}]
            events.extend({'reason': 'compiler-artifact', 'package_id': 'carlitos',
                           'target': {'name': 'carlitos', 'kind': ['bin']},
                           'profile': {'test': test}, 'executable': name}
                          for name, test in names)
            report.write_text(''.join(json.dumps(event) + '\n' for event in events))
            return subprocess.run(['bash', 'scripts/windows.sh', '--all-targets'],
                                  cwd=root, capture_output=True, text=True, timeout=10,
                                  env=dict(os.environ, CARLITOS_DEV_SHELL='windows',
                                           CARLITOS_RUST_VERSION='audit', FAKE_REPORT=str(report),
                                           PATH=f'{root}/bin:{os.environ["PATH"]}'))

    def test_test_harness_is_never_packaged_as_the_application(self):
        for artifacts in [[('app.exe', False), ('test-harness.exe', True)],
                          [('test-harness.exe', True), ('app.exe', False)]]:
            with self.subTest(artifacts=artifacts):
                result = self.build(artifacts)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn('selected:app.exe', result.stdout)
                self.assertNotIn('selected:test-harness.exe', result.stdout)

    def test_test_only_build_cannot_publish_an_installer(self):
        result = self.build([('test-harness.exe', True)])
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn('installer:', result.stdout)


if __name__ == '__main__':
    unittest.main()
