import MultiFinderCore
import SwiftUI

struct SidebarView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let screen = Binding<Screen?>(get: { model.screen }, set: { if let s = $0 { model.screen = s } })
        List(selection: screen) {
            Section("보기") {
                Label("모아 보기", systemImage: "square.stack.3d.up").tag(Screen.collection)
                Label("중복 파일", systemImage: "doc.on.doc").tag(Screen.duplicates)
                Label("폴더 비교", systemImage: "rectangle.split.2x1").tag(Screen.compare)
                Label("자동 분류", systemImage: "sparkles").tag(Screen.categories)
            }
            Section("정리") {
                Label("앱 잔여물", systemImage: "trash.slash").tag(Screen.leftovers)
            }
            Section("모은 폴더") {
                ForEach(model.folders, id: \.self) { path in
                    FolderSidebarRow(path: path)
                }
                .onMove { from, to in model.moveFolder(from: from, to: to) }
                Button { model.addFolderViaPanel() } label: {
                    Label("폴더 추가…", systemImage: "plus")
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
            }
        }
        .listStyle(.sidebar)
        .dropDestination(for: URL.self) { urls, _ in
            // Finder에서 폴더를 끌어다 놓으면 모아 보기에 추가
            var added = false
            for url in urls where (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true {
                model.addFolder(url.path); added = true
            }
            return added
        }
    }
}

private struct FolderSidebarRow: View {
    @Environment(AppModel.self) private var model
    let path: String

    var body: some View {
        HStack {
            Image(nsImage: IconCache.folderIcon).resizable().frame(width: 16, height: 16)
            Text((path as NSString).lastPathComponent)
            Spacer()
            if model.scanning.contains(path) {
                ProgressView().controlSize(.mini)
            } else if let s = model.folderSummary(path) {
                Text(ByteCountFormatter.string(fromByteCount: s.bytes, countStyle: .file))
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                Text("").task { model.loadFolderSummary(path) }
            }
        }
        .help(path)
        .contextMenu {
            Button("Finder에서 보기") { model.reveal(path) }
            Button("다시 스캔") { model.rescan(path) }
            Button("이 폴더 안 중복 찾기") {
                model.duplicates.scope = path
                model.screen = .duplicates
                model.duplicates.run()
            }
            Divider()
            Button("목록에서 제거") { model.removeFolder(path) }
        }
        .onTapGesture {
            model.screen = .collection
            model.expanded.insert(path)
            model.selection = [path]
        }
    }
}
