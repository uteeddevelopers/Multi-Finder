#!/bin/sh
# 배포용 DMG: 앱(Apple 실리콘·인텔 겸용) + 응용 프로그램 폴더 바로가기 + '처음 여는 법' 안내.
#   scripts/make-dmg.sh                                  → dist/Multi-Finder-<버전>.dmg (+ .sha256)
#   MF_SUPPORT_URL=https://… scripts/make-dmg.sh         → 도움말 메뉴·안내문에 후원 링크
#   MF_WEBSITE_URL=https://…                             → 도움말 메뉴에 웹사이트 링크
#   MF_UNIVERSAL=0                                       → 이 Mac 아키텍처만 (빠름)
#
# 서명
#   - 키체인에 'Developer ID Application' 인증서가 있으면(애플 개발자 프로그램 가입 필요) 그것으로 서명하고,
#     MF_NOTARY_PROFILE(xcrun notarytool store-credentials로 저장한 이름)이 있으면 공증 + 스테이플까지 한다.
#     이렇게 만든 DMG는 받는 사람이 경고 없이 바로 연다.
#   - 없으면 임시(ad-hoc) 서명. 받는 사람은 처음 한 번 '시스템 설정 › 개인정보 보호 및 보안 › 그래도 열기'를 거친다.
#     개인 Apple Development 인증서는 쓰지 않는다 — 서명 정보에 이메일 주소가 그대로 보이고, 어차피 Gatekeeper를 통과하지 못한다.
set -eu
cd "$(dirname "$0")/.."

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' Packaging/Info.plist)"
DIST=dist
STAGE="$DIST/stage"
APP="$STAGE/Multi-Finder.app"
DMG="$DIST/Multi-Finder-$VERSION.dmg"

DEVID="$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Developer ID Application/ {print $2; exit}')"
SIGN="${MF_SIGN_IDENTITY:-${DEVID:--}}"

rm -rf "$STAGE" "$DMG" "$DMG.sha256"
mkdir -p "$STAGE"
MF_APP_PATH="$APP" MF_UNIVERSAL="${MF_UNIVERSAL:-1}" MF_SIGN_IDENTITY="$SIGN" scripts/build-app.sh

# DMG 안: 앱 + /Applications 바로가기 + 안내문 (후원 링크가 있으면 안내문 끝에 붙인다)
ln -s /Applications "$STAGE/Applications"
README="$STAGE/How to Open · 처음 여는 법.txt"
sed "s/{VERSION}/$VERSION/g" Packaging/HowToOpen.txt > "$README"
if [ -n "${MF_SUPPORT_URL:-}" ]; then
  printf '\n♥ Support · 후원 · 支援 · 支持: %s\n' "$MF_SUPPORT_URL" >> "$README"
fi

hdiutil create -volname "Multi-Finder $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG" >/dev/null

if [ "$SIGN" != "-" ]; then
  codesign --force --timestamp --sign "$SIGN" "$DMG"
  if [ -n "${MF_NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$DMG" --keychain-profile "$MF_NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
  fi
fi

( cd "$DIST" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256" )
rm -rf "$STAGE"
echo "서명: $SIGN"
echo "→ $DMG ($(du -h "$DMG" | cut -f1))"
