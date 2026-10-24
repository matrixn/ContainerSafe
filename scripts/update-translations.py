"""Regenerate bundled Slint gettext from the shared English/Romanian JSON catalog.
Run with --check in validation; no gettext installation is needed.
"""
import json
import re
from pathlib import Path
import sys
root = Path(__file__).resolve().parent.parent
catalog = json.loads((root / 'native/translations/ro.json').read_text(encoding='utf-8'))
output = 'msgid ""\nmsgstr ""\n"Content-Type: text/plain; charset=UTF-8\\n"\n"Language: ro\\n"\n"Plural-Forms: nplurals=3; plural=(n == 1 ? 0 : (n == 0 || (n % 100 > 0 && n % 100 < 20)) ? 1 : 2);\\n"\n\n'
for source, translation in catalog.items():
    if not source or not translation:
        raise ValueError('Empty translation entry')
    if sorted(re.findall(r'\{[0-9]+\}', source)) != sorted(re.findall(r'\{[0-9]+\}', translation)):
        raise ValueError(f'Translation placeholders differ: {source}')
    output += f'msgid {json.dumps(source, ensure_ascii=False)}\nmsgstr {json.dumps(translation, ensure_ascii=False)}\n\n'
for source_file in (root / 'native/ui').rglob('*.slint'):
    for literal in re.findall(r'@tr\(("(?:[^"\\]|\\.)*")\)', source_file.read_text(encoding='utf-8')):
        if json.loads(literal) not in catalog:
            raise ValueError(f'Missing Romanian translation in {source_file}: {literal}')
path = root / 'native/translations/ro/LC_MESSAGES/containersafe-native.po'
if '--check' in sys.argv:
    if path.read_text(encoding='utf-8') != output:
        raise SystemExit('Translation bundle is stale. Run scripts/update-translations.py')
    print(f'Translation catalog OK: {len(catalog)} entries')
else:
    path.write_text(output, encoding='utf-8')
