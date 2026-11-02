import MultiFinderCore
import QuickLook
import SwiftUI
import UniformTypeIdentifiers

/// 모아 보기 — 모은 폴더 여러 개가 한 화면에 나오고, 각각 펼치면 그 아래 내용이 이어서 보인다.
/// Finder 목록 보기와 같은 펼침 삼각형(outline) 방식이되, 루트가 여러 개다.
struct CollectionView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        if model.folders.isEmpty {
            ContentUnavailableView {
                Label("모은 폴더가 없습니다", systemImage: "square.stack.3d.up.slash")
            } description: {
                Text("한 화면에 같이 볼 폴더를 추가하세요. 데스크탑·다운로드처럼 자주 어지러워지는 폴더부터.")
            } actions: {
                Button("폴더 추가…") { model.addFolderViaPanel() }
            }
        } else {
            VStack(spacing: 0) {
                if model.isSearching { SearchBanner() } else if model.filter.isActive { FilterBanner() }
                ColumnHeader()
                ScrollViewReader { proxy in
                    List(selection: $model.selection) {
                        if model.isSearching {
                            // 검색 중에는 Finder처럼 모은 폴더 아래 전체에서 이름이 맞는 항목을 평평하게 보여 준다
                            ForEach(model.visibleSearchResults) { r in
                                SearchResultRow(record: r)
                                    .tag(r.path)
                                    .draggable(DraggedPaths(paths: model.dragPayload(for: r.path)))
                            }
                        } else {
                            ForEach(model.folders, id: \.self) { path in
                                if let root = model.rootRecord(path) {
                                    FolderNode(record: root, depth: 0)
                                } else {
                                    FolderPlaceholderRow(path: path)
                                }
                            }
                        }
                    }
                    .listStyle(.inset)
                    .alternatingRowBackgrounds()
                    .overlay {
                        if model.isSearching, !model.searchPending, model.visibleSearchResults.isEmpty {
                            ContentUnavailableView.search(text: model.filter.text)
                        }
                    }
                    .contextMenu(forSelectionType: String.self) { paths in
                        selectionMenu(paths)
                    } primaryAction: { paths in
                        model.activate(paths)
                    }
                    .quickLookPreview($model.quickLookURL, in: model.quickLookItems)
                    .onKeyPress(.space) { model.toggleQuickLook(); return .handled }
                    .onKeyPress(.return) { model.activate(model.selection); return .handled }
                    .onKeyPress(.rightArrow) { model.expandSelection(true); return .handled }
                    .onKeyPress(.leftArrow) { model.expandSelection(false); return .handled }
                    .onDeleteCommand { model.requestTrashSelection() }
                    .onChange(of: model.scrollTarget) { _, target in
                        guard let target else { return }
                        // 펼친 줄이 자리를 잡은 뒤에 스크롤
                        Task { @MainActor in
                            try? await Task.sleep(for: .milliseconds(150))
                            withAnimation { proxy.scrollTo(target, anchor: .center) }
                            model.scrollTarget = nil
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func selectionMenu(_ paths: Set<String>) -> some View {
        if paths.isEmpty {
            Button("다시 스캔") { model.rescanAll() }
        } else {
            let single = paths.count == 1 ? paths.first : nil
            let singleDir = single.flatMap { model.rootRecord($0) }?.kind == .directory ? single : nil
            Button("열기") { for p in paths { model.open(p) } }
            if model.isSearching, let single {
                Button("폴더 목록에서 보기") { model.revealInList(single) }
            }
            Button("미리보기 (스페이스)") { model.selection = paths; model.toggleQuickLook() }
            Button("Finder에서 보기") {
                NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) })
            }
            Button("경로 복사") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(paths.sorted().joined(separator: "\n"), forType: .string)
            }
            if let dir = singleDir {
                Divider()
                Button("새 폴더") { model.newFolder(in: dir) }
                Button("이 폴더를 모아 보기에 추가") { model.addFolder(dir) }
                    .disabled(model.folders.contains(dir))
                Button("이 폴더 안 중복 찾기") {
                    model.duplicates.scope = dir
                    model.screen = .duplicates
                    model.duplicates.run()
                }
                Button(model.compare.left == nil ? "폴더 비교의 왼쪽으로" : "폴더 비교의 오른쪽으로") {
                    model.compare.set(side: model.compare.left == nil ? .left : .right, path: dir)
                    if model.compare.canRun { model.screen = .compare; model.compare.run() }
                }
            }
            Divider()
            Button("이동…") { model.selection = paths; model.requestMoveSelection() }
            Button("휴지통으로 이동…", role: .destructive) { model.selection = paths; model.requestTrashSelection() }
        }
    }
}

/// Finder 목록 보기처럼 열 머리글을 둔다. 클릭하면 그 열로 정렬.
private struct ColumnHeader: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            header(.name).frame(maxWidth: .infinity, alignment: .leading).padding(.leading, 28)
            header(.date).frame(width: 180, alignment: .trailing)
            header(.size).frame(width: 150, alignment: .trailing)
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .background(.bar)
        .overlay(alignment: .bottom) { Divider() }
    }

    private func header(_ field: SortField) -> some View {
        let title = field.label
        return Button {
            if model.sortField == field { model.sortAscending.toggle() } else { model.sortField = field; model.sortAscending = true }
        } label: {
            HStack(spacing: 3) {
                Text(title)
                if model.sortField == field {
                    Image(systemName: model.sortAscending ? "chevron.up" : "chevron.down").font(.caption2)
                }
            }
            .foregroundStyle(model.sortField == field ? .primary : .secondary)
        }
        .buttonStyle(.plain)
        .help("\(title)순 정렬")
    }
}

/// 검색 중일 때 목록 위 안내 줄 — 몇 개 찾았는지, 다른 필터도 걸려 있는지.
private struct SearchBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let count = model.visibleSearchResults.count
        let text = model.filter.text
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
            Text("‘\(text)’ 검색 결과 \(count)개")
            if model.filter.category != .all || model.filter.sizeFloor != .any || model.filter.recent != .any {
                Text("· 필터 적용 중").foregroundStyle(.secondary)
            }
            Spacer()
            Button("검색 지우기") { model.filter.text = "" }
                .controlSize(.small)
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(.quaternary.opacity(0.5))
        .overlay(alignment: .bottom) { Divider() }
    }
}

/// 검색 결과 한 줄: 이름 아래에 들어 있는 폴더를 작게.
private struct SearchResultRow: View {
    let record: FileRecord

    var body: some View {
        HStack(spacing: 6) {
            Image(nsImage: IconCache.icon(for: record)).resizable().frame(width: 16, height: 16)
            VStack(alignment: .leading, spacing: 1) {
                Text(record.name).lineLimit(1).truncationMode(.middle)
                Text(record.parent.replacingOccurrences(of: NSHomeDirectory(), with: "~"))
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 12)
            Text(record.modified, format: .dateTime.year().month(.twoDigits).day(.twoDigits).hour().minute())
                .foregroundStyle(.secondary).monospacedDigit().lineLimit(1).fixedSize()
                .frame(width: 180, alignment: .trailing)
            Group {
                if record.kind == .file {
                    Text(ByteCountFormatter.string(fromByteCount: record.size, countStyle: .file))
                } else if record.kind == .directory {
                    FolderSizeText(path: record.path)
                } else {
                    Text("--")
                }
            }
            .foregroundStyle(.secondary).monospacedDigit().lineLimit(1)
            .frame(width: 150, alignment: .trailing)
        }
        .help(record.path)
    }
}

/// 필터가 켜져 있을 때 목록 위에 뜨는 얇은 안내 줄. 왜 항목이 적게 보이는지 바로 알 수 있게.
private struct FilterBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "line.3.horizontal.decrease.circle.fill")
            Text("필터 적용 중: \(model.filterDescription)")
            Spacer()
            Button("지우기") { model.filter = FileFilter() }
                .controlSize(.small)
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(.quaternary.opacity(0.5))
        .overlay(alignment: .bottom) { Divider() }
    }
}

/// 아직 인덱스에 없는(스캔 전) 폴더.
private struct FolderPlaceholderRow: View {
    @Environment(AppModel.self) private var model
    let path: String

    var body: some View {
        HStack {
            Image(nsImage: IconCache.folderIcon).resizable().frame(width: 16, height: 16)
            Text((path as NSString).lastPathComponent)
            Spacer()
            if model.scanning.contains(path) {
                ProgressView().controlSize(.small)
                Text("스캔 중…").foregroundStyle(.secondary)
            } else if !FileManager.default.fileExists(atPath: path) {
                Text("폴더가 없음").foregroundStyle(.secondary)
                Button("제거") { model.removeFolder(path) }.controlSize(.small)
            } else {
                Text("인덱스 없음").foregroundStyle(.secondary)
                Button("스캔") { model.rescan(path) }.controlSize(.small)
            }
        }
        .tag(path)
    }
}

/// 폴더 한 줄 + 펼치면 자식들. 자식 폴더는 재귀. 파일을 끌어다 놓으면 이 폴더로 옮긴다.
struct FolderNode: View {
    @Environment(AppModel.self) private var model
    let record: FileRecord
    let depth: Int
    @State private var dropTargeted = false

    var body: some View {
        DisclosureGroup(isExpanded: model.isExpanded(record.path)) {
            let children = model.children(of: record.path)
            if children.isEmpty {
                Text(model.filter.isActive ? "필터에 맞는 항목 없음" : "비어 있음")
                    .foregroundStyle(.tertiary)
                    .font(.callout)
            } else {
                ForEach(children) { child in
                    if child.kind == .directory {
                        FolderNode(record: child, depth: depth + 1)
                    } else {
                        FileRow(record: child)
                            .tag(child.path)
                            .id(child.path)
                            .draggable(DraggedPaths(paths: model.dragPayload(for: child.path)))
                    }
                }
            }
        } label: {
            FileRow(record: record, isRoot: depth == 0)
                .padding(.vertical, dropTargeted ? 2 : 0)
                .background(dropTargeted ? Color.accentColor.opacity(0.18) : .clear, in: RoundedRectangle(cornerRadius: 4))
                .dropDestination(for: DraggedPaths.self) { items, _ in
                    model.drop(items.flatMap(\.paths), into: record.path)
                } isTargeted: { dropTargeted = $0 }
                .draggable(DraggedPaths(paths: model.dragPayload(for: record.path)))
        }
        .tag(record.path)
        .id(record.path)
    }
}

/// 끌어 놓기 페이로드: 경로 목록. 여러 개를 골라 끌면 전부 함께 간다.
struct DraggedPaths: Codable, Transferable {
    var paths: [String]
    static var transferRepresentation: some TransferRepresentation {
        CodableRepresentation(contentType: .multiFinderPaths)
    }
}

extension UTType {
    static let multiFinderPaths = UTType(exportedAs: "com.taeholim.multifinder.paths")
}

/// Finder 목록 보기의 한 줄: 아이콘 · 이름 · (iCloud 표시) · 수정일 · 크기(폴더는 항목 수·용량)
struct FileRow: View {
    @Environment(AppModel.self) private var model
    let record: FileRecord
    var isRoot = false

    var body: some View {
        HStack(spacing: 6) {
            Image(nsImage: IconCache.icon(for: record))
                .resizable()
                .frame(width: 16, height: 16)
            Text(isRoot ? displayRootName : record.name)
                .lineLimit(1)
                .truncationMode(.middle)
                .fontWeight(isRoot ? .semibold : .regular)
            if record.isDataless {
                Image(systemName: "icloud.and.arrow.down")
                    .foregroundStyle(.secondary)
                    .help("내용이 이 Mac에 없음 (iCloud 등)")
            }
            if isRoot, model.scanning.contains(record.path) {
                ProgressView().controlSize(.mini)
            }
            Spacer(minLength: 12)
            Text(record.modified, format: .dateTime.year().month(.twoDigits).day(.twoDigits).hour().minute())
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .lineLimit(1)
                .fixedSize()
                .frame(width: 180, alignment: .trailing)
            Group {
                if record.kind == .file {
                    Text(ByteCountFormatter.string(fromByteCount: record.size, countStyle: .file))
                } else if record.kind == .directory {
                    FolderSizeText(path: record.path)
                } else {
                    Text("--")
                }
            }
            .foregroundStyle(.secondary)
            .monospacedDigit()
            .lineLimit(1)
            .frame(width: 150, alignment: .trailing)
        }
        .help(record.path)
    }

    private var displayRootName: String {
        record.path.replacingOccurrences(of: NSHomeDirectory(), with: "~")
    }
}

/// 폴더의 "n개 항목 · 용량". 인덱스에서 한 번 계산해 캐시.
private struct FolderSizeText: View {
    @Environment(AppModel.self) private var model
    let path: String

    var body: some View {
        if let s = model.folderSummary(path) {
            let size = ByteCountFormatter.string(fromByteCount: s.bytes, countStyle: .file)
            Text("\(AppModel.itemCount(s.files)) · \(size)")
        } else {
            Text("…").task { model.loadFolderSummary(path) }
        }
    }
}
