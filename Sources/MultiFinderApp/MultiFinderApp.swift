import SwiftUI
import MultiFinderCore

@main
struct MultiFinderApp: App {
    @State private var model = AppModel.load()

    var body: some Scene {
        WindowGroup("Multi-Finder") {
            ContentView()
                .environment(model)
                .frame(minWidth: 820, minHeight: 520)
        }
        .defaultSize(width: 1200, height: 760)   // 처음 실행할 때의 창 크기 (그 뒤엔 사용자가 정한 크기를 시스템이 기억)
        .commands {
            // 실행 취소(⌘Z)는 시스템 기본 메뉴 그대로 — 글자를 입력하는 중엔 글자를, 그 밖에선 창에 등록된 파일 작업을 되돌린다
            CommandGroup(after: .newItem) {
                Button("폴더 추가…") { model.addFolderViaPanel() }
                    .keyboardShortcut("o", modifiers: .command)
                Button("다시 스캔") { model.rescanAll() }
                    .keyboardShortcut("r", modifiers: .command)
                    .disabled(model.isScanning)
            }
            // 아래 명령은 지금 보이는 화면에서 고른 것에만 작동한다
            CommandGroup(after: .pasteboard) {
                Button("Finder에서 보기") { model.revealOnCurrentScreen() }
                    .keyboardShortcut("r", modifiers: [.command, .shift])
                    .disabled(!model.hasSelectionOnCurrentScreen)
                Divider()
                Button("휴지통으로 이동") { model.trashOnCurrentScreen() }
                    .keyboardShortcut(.delete, modifiers: .command)
                    .disabled(!model.hasSelectionOnCurrentScreen)
                Button("이동…") { model.moveOnCurrentScreen() }
                    .keyboardShortcut("m", modifiers: [.command, .shift])
                    .disabled(!model.canMoveOnCurrentScreen)
            }
            // 도움말 메뉴: 기본 'Multi-Finder 도움말'은 내용이 없어 빼고, 배포 빌드에 주소가 있으면 웹사이트·후원 링크만 둔다.
            CommandGroup(replacing: .help) {
                if let url = AppLinks.website {
                    Button("Multi-Finder 웹사이트") { NSWorkspace.shared.open(url) }
                }
                if let url = AppLinks.support {
                    Button("Multi-Finder 후원하기…") { NSWorkspace.shared.open(url) }
                }
            }
        }
    }
}

/// 배포 빌드가 Info.plist에 넣는 주소 (scripts/make-dmg.sh의 MF_WEBSITE_URL·MF_SUPPORT_URL). 비어 있으면 메뉴를 숨긴다.
enum AppLinks {
    static let website = url(forKey: "MFWebsiteURL")
    static let support = url(forKey: "MFSupportURL")

    private static func url(forKey key: String) -> URL? {
        guard let s = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !s.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return URL(string: s)
    }
}
