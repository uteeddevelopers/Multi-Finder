#!/bin/sh
# 코드의 화면 문자열을 문자열 카탈로그(Packaging/Localizable.xcstrings)에 반영하고, 번역이 빠진 것을 보여 준다.
#   scripts/update-strings.sh
# 키는 한국어 원문이다. 빠진 번역은 Xcode로 카탈로그를 열어 채우거나 JSON을 직접 고친다.
set -eu
cd "$(dirname "$0")/.."
OUT=build/stringsdata
# 증분 빌드는 바뀐 파일의 문자열만 내놓아 나머지가 '안 씀'으로 잘못 표시된다 → 매번 처음부터
rm -rf "$OUT" .build/l10n; mkdir -p "$OUT"
swift build --scratch-path .build/l10n --product MultiFinderApp \
  -Xswiftc -emit-localized-strings -Xswiftc -emit-localized-strings-path -Xswiftc "$PWD/$OUT" >/dev/null
xcrun xcstringstool sync Packaging/Localizable.xcstrings --stringsdata "$OUT"/*.stringsdata
/usr/bin/python3 - <<'PY'
import json, re
d = json.load(open('Packaging/Localizable.xcstrings'))
langs = ['en', 'ja', 'zh-Hans']
live = {k: v for k, v in d['strings'].items() if v.get('extractionState') != 'stale'}
missing = [(k, [l for l in langs if l not in v.get('localizations', {})]) for k, v in live.items() if re.search('[가-힣]', k)]
missing = [(k, ls) for k, ls in missing if ls]
stale = [k for k, v in d['strings'].items() if v.get('extractionState') == 'stale']
print(f"키 {len(d['strings'])}개 · 번역 빠짐 {len(missing)}개 · 더 이상 안 쓰는 키 {len(stale)}개")
for k, ls in missing: print('  빠짐', ls, repr(k))
for k in stale: print('  안 씀', repr(k))
PY
