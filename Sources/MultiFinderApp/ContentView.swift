import MultiFinderCore
import SwiftUI

struct ContentView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 180, ideal: 220)
        } detail: {
            VStack(spacing: 0) {
                Group {
                    switch model.screen {
                    case .collection: CollectionView()
                    case .duplicates: DuplicatesView()
                    case .compare: CompareView()
                    case .categories: CategoriesView()
                    case .leftovers: LeftoversView()
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                StatusBar()
            }
        }
        .navigationTitle(title)
        .toolbar { toolbar }
        .background(UndoManagerBridge())
        .onChange(of: model.screen) { _, _ in model.refreshStaleScreen() }
        .searchable(text: $model.filter.text, placement: .toolbar, prompt: "이름으로 찾기")
        .confirmationDialog(
            "\(model.trashRequest?.count ?? 0)개 항목을 휴지통으로 옮길까요?",
            isPresented: Binding(get: { model.trashRequest != nil }, set: { if !$0 { model.trashRequest = nil } }),
            titleVisibility: .visible
        ) {
            Button("휴지통으로 이동", role: .destructive) { model.confirmTrash() }
            Button("취소", role: .cancel) { model.trashRequest = nil }
        } message: {
            Text(trashMessage)
        }
        .confirmationDialog(
            model.moveRequest?.title ?? "",
            isPresented: Binding(get: { model.moveRequest != nil }, set: { if !$0 { model.moveRequest = nil } }),
            titleVisibility: .visible
        ) {
            Button("이동") { model.confirmMove() }
            Button("취소", role: .cancel) { model.moveRequest = nil }
        } message: {
            Text(moveMessage)
        }
        .alert(model.errorTitle, isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.clearError() } })) {
            if let action = model.errorAction {
                Button(action == .openFullDiskAccessSettings ? "전체 디스크 접근 설정 열기" : "Finder에서 보기") {
                    model.perform(action)
                    model.clearError()
                }
            }
            Button("확인", role: .cancel) { model.clearError() }
        } message: {
            Text(model.errorMessage ?? "")
        }
    }

    private var title: String {
        switch model.screen {
        case .collection: return String(localized: "모아 보기")
        case .duplicates: return String(localized: "중복 파일")
        case .compare: return String(localized: "폴더 비교")
        case .categories: return String(localized: "자동 분류")
        case .leftovers: return String(localized: "앱 잔여물")
        }
    }

    /// 확인 창의 항목 한 줄: 이름 — 들어 있는 폴더 (같은 이름이 여러 곳에 있어도 어느 것인지 알 수 있게)
    private static func line(_ path: String) -> String {
        let folder = ((path as NSString).deletingLastPathComponent).replacingOccurrences(of: NSHomeDirectory(), with: "~")
        return "\((path as NSString).lastPathComponent) — \(folder)"
    }

    private var moveMessage: String {
        let items = model.moveRequest?.items ?? []
        let names = items.prefix(5).map { Self.line($0.source) }
        let more = items.count - names.count
        return names.joined(separator: "\n") + (more > 0 ? "\n… " + AppModel.andMore(more) : "")
            + "\n\n" + String(localized: "옮긴 뒤에도 ⌘Z로 한 번에 되돌릴 수 있습니다.")
    }

    private var trashMessage: String {
        let paths = model.trashRequest ?? []
        let names = paths.prefix(5).map { Self.line($0) }
        let more = paths.count - names.count
        let bytes = model.trashRequestBytes ?? paths.compactMap { model.rootRecord($0) }
            .reduce(Int64(0)) { $0 + ($1.kind == .file ? $1.size : (model.folderSummary($1.path)?.bytes ?? 0)) }
        let undo = model.hasFullDiskAccess
            ? String(localized: "휴지통에서 복구하거나 ⌘Z로 되돌릴 수 있습니다.")
            : String(localized: "Finder 휴지통에서 ‘되돌려 놓기’로 복구할 수 있습니다.")
        var text = names.joined(separator: "\n") + (more > 0 ? "\n… " + AppModel.andMore(more) : "")
        text += "\n\n\(ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)) · \(undo)"
        if let note = model.trashRequestNote { text += "\n\(note)" }
        return text
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        @Bindable var model = model
        ToolbarItemGroup(placement: .navigation) {
            Button { model.addFolderViaPanel() } label: { Label("폴더 추가", systemImage: "plus") }
                .help("모아 보기에 폴더 추가 (⌘O)")
            Button { model.rescanAll() } label: { Label("다시 스캔", systemImage: "arrow.clockwise") }
                .help("다시 스캔 (⌘R)")
                .disabled(model.isScanning)
        }
        ToolbarItemGroup(placement: .primaryAction) {
            if model.screen == .collection {
                Button { model.requestMoveSelection() } label: { Label("이동", systemImage: "folder") }
                    .help("선택 항목을 다른 폴더로 이동 (⇧⌘M)")
                    .disabled(model.selection.isEmpty)
                Button { model.requestTrashSelection() } label: { Label("휴지통", systemImage: "trash") }
                    .help("선택 항목을 휴지통으로 (⌘⌫)")
                    .disabled(model.selection.isEmpty)
                Menu {
                    Picker("정렬 기준", selection: $model.sortField) {
                        ForEach(SortField.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.inline)
                    Divider()
                    Picker("순서", selection: $model.sortAscending) {
                        Text("오름차순").tag(true)
                        Text("내림차순").tag(false)
                    }
                    .pickerStyle(.inline)
                } label: {
                    Label("정렬", systemImage: "arrow.up.arrow.down")
                }
                .help("정렬: \(model.sortField.label) \(model.sortAscending ? "↑" : "↓")")
            }
            Menu {
                Picker("형식", selection: $model.filter.category) {
                    ForEach(FileCategory.allCases) { Text($0.label).tag($0) }
                }
                .pickerStyle(.inline)
                Divider()
                Picker("크기", selection: $model.filter.sizeFloor) {
                    ForEach(SizeFloor.allCases) { Text($0.label).tag($0) }
                }
                .pickerStyle(.inline)
                Divider()
                Picker("기간", selection: $model.filter.recent) {
                    ForEach(RecentRange.allCases) { Text($0.label).tag($0) }
                }
                .pickerStyle(.inline)
                if model.filter.isActive {
                    Divider()
                    Button("필터 지우기") { model.filter = FileFilter() }
                }
            } label: {
                Label("필터", systemImage: model.filter.isActive ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
            }
            .help(model.filter.isActive ? "필터: \(model.filterDescription)" : "형식·크기·기간으로 거르기")
        }
    }
}

/// 창 하단 상태 줄 — 스캔 진행 · 마지막 작업 · 선택한 항목 경로.
struct StatusBar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 12) {
            if model.isScanning {
                ProgressView().controlSize(.small)
                Text(model.progressText.isEmpty ? String(localized: "스캔 중…") : model.progressText)
                    .lineLimit(1).truncationMode(.middle)
            } else if model.selection.count == 1, let p = model.selection.first {
                Image(systemName: "location")
                Text(p.replacingOccurrences(of: NSHomeDirectory(), with: "~"))
                    .lineLimit(1).truncationMode(.middle)
                    .textSelection(.enabled)
            } else if let summary = model.lastOperationSummary {
                Text(summary)
            } else if let s = model.stats {
                Text(Self.statsText(s))
            } else {
                Text("").task { model.loadStats() }
            }
            Spacer()
            if model.selection.count > 1 {
                Text("\(model.selection.count)개 선택")
            }
            if model.canUndo {
                Button("되돌리기") { model.undo() }
                    .controlSize(.small)
                    .help("마지막 이동·휴지통 작업 되돌리기 (⌘Z)")
            }
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
    }

    private static func statsText(_ s: IndexStore.Stats) -> String {
        let bytes = ByteCountFormatter.string(fromByteCount: s.bytes, countStyle: .file)
        let files = s.files.formatted(), folders = s.directories.formatted()
        return String(localized: "파일 \(files)개 · 폴더 \(folders)개 · \(bytes) · 스페이스: 미리보기 · 끌어 놓기: 이동",
                      comment: "Status bar: indexed file count, folder count, total size, then two keyboard/mouse hints")
    }
}

/// 창의 실행 취소 관리자를 모델에 이어 준다 — 파일 작업을 편집 › 실행 취소(⌘Z)로 되돌릴 수 있게.
private struct UndoManagerBridge: View {
    @Environment(\.undoManager) private var undoManager
    @Environment(AppModel.self) private var model

    var body: some View {
        Color.clear
            .onAppear { model.undoManager = undoManager }
            .onChange(of: undoManager.map(ObjectIdentifier.init)) { _, _ in model.undoManager = undoManager }
    }
}
