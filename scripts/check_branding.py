#!/usr/bin/env python3
"""Audit product identifiers, Cargo paths, SDK names and shipped brand assets."""
import json
from pathlib import Path
import re
import sys
import tomllib
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
LEGACY = re.compile('j' + 'code', re.I)
EXEMPT = {'NOTICE', 'LICENSE', 'sdk/typescript/LICENSE'}
SKIP = {'.git', 'target', 'node_modules', '__pycache__', '.venv'}
errors = []
count = 0

for path in ROOT.rglob('*'):
    relative = path.relative_to(ROOT)
    if any(part in SKIP for part in relative.parts):
        continue
    if LEGACY.search(str(relative)):
        errors.append(f'Legacy path: {relative}')
    if not path.is_file() or path.is_symlink() or str(relative) in EXEMPT:
        continue
    try:
        content = path.read_text()
    except UnicodeError:
        continue
    count += 1
    if LEGACY.search(content):
        errors.append(f'Legacy product identifier: {relative}')
    if path.name == 'Cargo.toml':
        data = tomllib.loads(content)
        for member in data.get('workspace', {}).get('members', []):
            if not (path.parent / member / 'Cargo.toml').is_file():
                errors.append(f'Missing workspace member: {member}')
        def check_tables(table):
            for key, value in table.items():
                if isinstance(value, dict):
                    if 'path' in value and isinstance(value['path'], str):
                        if not (path.parent / value['path']).exists():
                            errors.append(f'Missing path in {relative}: {value["path"]}')
                    check_tables(value)
        check_tables(data)

manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text())
if manifest['package']['name'] != 'nexode':
    errors.append('Root package is not nexode')
if 'nexode' not in {b['name'] for b in manifest['bin']}:
    errors.append('Missing nexode CLI target')
for path in (ROOT / 'sdk').rglob('package.json'):
    if any(part in SKIP for part in path.relative_to(ROOT).parts):
        continue
    data = json.loads(path.read_text())
    if not data.get('private'):
        errors.append(f'SDK package must remain private until configured: {path}')

for asset in ['assets/brand/nexode-mark.svg', 'assets/brand/nexode-wordmark.svg',
              'assets/brand/nexode-harness-banner.svg']:
    ET.parse(ROOT / asset)
for asset, magic in [('assets/app-icons/Nexode.icns', b'icns'),
                     ('assets/app-icons/Nexode.ico', b'\0\0\1\0'),
                     ('assets/app-icons/Nexode.png', b'\x89PNG')]:
    if not (ROOT / asset).read_bytes().startswith(magic):
        errors.append(f'Invalid image signature: {asset}')

if errors:
    print('\n'.join(errors), file=sys.stderr)
    sys.exit(1)
print(f'Brand audit passed: {count} text files, workspace paths, SDK metadata and assets.')
