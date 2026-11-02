#!/bin/sh
# Multi-Finder.app 번들을 만든다 (Xcode 프로젝트 없이 SwiftPM 빌드 + 수동 패키징).
#   scripts/build-app.sh            → build/Multi-Finder.app (이 Mac에서 쓰는 개발용)
#   scripts/build-app.sh --run      → 만들고 바로 실행 (앞으로 가져오지 않음)
# 배포용은 scripts/make-dmg.sh가 아래 환경 변수로 이 스크립트를 부른다.
#   MF_APP_PATH        만들 위치 (기본 build/Multi-Finder.app)
#   MF_UNIVERSAL=1     Apple 실리콘 + 인텔 겸용 바이너리
#   MF_SIGN_IDENTITY   서명 인증서 이름, "-"면 임시(ad-hoc) 서명
#   MF_VERSION / MF_BUILD / MF_WEBSITE_URL / MF_SUPPORT_URL   Info.plist에 넣을 값
set -eu
cd "$(dirname "$0")/.."

APP="${MF_APP_PATH:-build/Multi-Finder.app}"
if [ "${MF_UNIVERSAL:-0}" = "1" ]; then
  swift build -c release --product MultiFinderApp --arch arm64 --arch x86_64
  BIN="$(swift build -c release --product MultiFinderApp --arch arm64 --arch x86_64 --show-bin-path)/MultiFinderApp"
else
  swift build -c release --product MultiFinderApp
  BIN="$(swift build -c release --product MultiFinderApp --show-bin-path)/MultiFinderApp"
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Multi-Finder"
cp Packaging/Info.plist "$APP/Contents/Info.plist"
if [ -f Packaging/AppIcon.icns ]; then cp Packaging/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"; fi
printf 'APPL????' > "$APP/Contents/PkgInfo"

# 다국어: 문자열 카탈로그 → 언어별 .lproj (한국어·영어·일본어·중국어 간체).
# macOS가 사용자의 언어 설정에 맞춰 고르고, 목록에 없는 언어면 영어(CFBundleDevelopmentRegion)로 보인다.
for catalog in Packaging/Localizable.xcstrings Packaging/InfoPlist.xcstrings; do
  xcrun xcstringstool compile "$catalog" --output-directory "$APP/Contents/Resources" >/dev/null
done

PB=/usr/libexec/PlistBuddy
PLIST="$APP/Contents/Info.plist"
if [ -n "${MF_VERSION:-}" ]; then $PB -c "Set :CFBundleShortVersionString $MF_VERSION" "$PLIST"; fi
if [ -n "${MF_BUILD:-}" ]; then $PB -c "Set :CFBundleVersion $MF_BUILD" "$PLIST"; fi
if [ -n "${MF_WEBSITE_URL:-}" ]; then $PB -c "Set :MFWebsiteURL $MF_WEBSITE_URL" "$PLIST"; fi
if [ -n "${MF_SUPPORT_URL:-}" ]; then $PB -c "Set :MFSupportURL $MF_SUPPORT_URL" "$PLIST"; fi

# 서명. 기본은 키체인의 Apple Development 인증서 — 다시 빌드해도 앱의 신원이 그대로라
# '전체 디스크 접근' 같은 권한을 한 번만 주면 된다. 없거나 "-"면 임시(ad-hoc) 서명.
# Developer ID 인증서면 공증에 필요한 hardened runtime과 보안 타임스탬프를 붙인다.
IDENTITY="${MF_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Apple Development/ {print $2; exit}')}"
case "$IDENTITY" in
  ""|"-")
    codesign --force --sign - "$APP"
    echo "서명: 임시(ad-hoc)" ;;
  "Developer ID Application"*)
    codesign --force --timestamp --options runtime --sign "$IDENTITY" "$APP"
    echo "서명: $IDENTITY (hardened runtime)" ;;
  *)
    if codesign --force --timestamp=none --sign "$IDENTITY" "$APP" >/dev/null 2>&1; then
      echo "서명: $IDENTITY"
    else
      codesign --force --sign - "$APP"
      echo "서명: 임시(ad-hoc) — '$IDENTITY'로 서명하지 못함"
    fi ;;
esac
codesign --verify --strict "$APP"
echo "→ $APP"

if [ "${1:-}" = "--run" ]; then
  open -g "$APP"
fi
