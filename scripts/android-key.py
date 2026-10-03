#!/usr/bin/env python3
"""Create the local release signing identity once; never replace an existing key."""
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[1]
    destination = root / '.secrets/android'
    if destination.exists():
        raise SystemExit('Signing directory already exists; keep the existing key: .secrets/android')
    for name in ('release.p12', 'signing.properties'):
        subprocess.run(['git', 'check-ignore', '-q', str(destination / name)], cwd=root, check=True)
    os.umask(0o077)
    destination.parent.mkdir(mode=0o700, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='.android-', dir=destination.parent))
    try:
        password = secrets.token_hex(32)
        password_file = staging / 'password'
        password_file.write_text(password)
        subprocess.run([
            'keytool', '-genkeypair', '-noprompt', '-storetype', 'PKCS12',
            '-keystore', str(staging / 'release.p12'), '-alias', 'carlitos',
            '-keyalg', 'RSA', '-keysize', '4096', '-sigalg', 'SHA256withRSA',
            '-validity', '36500', '-dname', 'CN=Carlitos, O=mny315',
            '-storepass:file', str(password_file),
        ], check=True)
        (staging / 'signing.properties').write_text(
            'storeFile=release.p12\n'
            f'storePassword={password}\nkeyAlias=carlitos\nkeyPassword={password}\n'
        )
        password_file.unlink()
        staging.rename(destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    print('Release key created in .secrets/android/ (excluded from Git).')
    print('Back up this directory securely: future APK updates need this same key.')


if __name__ == '__main__':
    main()
