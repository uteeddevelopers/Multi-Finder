import AppKit
import MultiFinderCore
import Observation
import SwiftUI

enum Screen: Hashable {
    case collection
    case duplicates
    case compare
    case categories
    case leftovers
}

enum SortField: String, CaseIterable, Identifiable {
    case name, date, size
    var id: String { rawValue }
    var label: String {
        switch self {
        case .name: return String(localized: "이름", comment: "Sort field / column: file name")
        case .date: return String(localized: "수정일", comment: "Sort field / column: date modified")
        case .size: return String(localized: "크기", comment: "Sort field / column: size")
        }
    }
}

/// 확인을 기다리는 여러 파일 이동 (폴더 비교의 '한쪽으로 몰기'). 이동은 사용자가 확인한 뒤에만.
struct MoveRequest {
    let title: String
    let items: [(source: String, directory: String)]
    /// 대상 쪽에 같은 상대 경로의 폴더가 없으면 만든다
    let createDirectories: Bool
    /// 요약 문구에 쓸 대상 폴더 이름
    let folderName: String
}

/// 앱 상태의 단일 소유자. 코어(인덱스·실행기)를 감싸고 뷰에 필요한 형태로 내놓는다.
@Observable
@MainActor
final class AppModel {
    // 코어
    let store: IndexStore
    let executor: Executor
    let rules = ScanRules.default()

    // 화면
    var screen: Screen = .collection
    var folders: [String] { didSet { persist(folders, key: "folders"); restartWatcher(); pruneSelection() } }
    var expanded: Set<String> = [] { didSet { persist(Array(expanded), key: "expanded"); pruneSelection() } }
    var selection: Set<String> = []
    var filter = FileFilter() {
        didSet {
            if filter.text != oldValue.text { updateSearch() }
            pruneSelection()
        }
    }
    var sortField: SortField = .name { didSet { UserDefaults.standard.set(sortField.rawValue, forKey: "sortField") } }
    var sortAscending = true { didSet { UserDefaults.standard.set(sortAscending, forKey: "sortAscending") } }
    var quickLookURL: URL?
    /// 검색어가 있을 때의 결과 (모은 폴더 아래 전체, 이름 기준) — Finder처럼 평평한 목록으로 보여 준다
    var searchResults: [FileRecord] = []
    /// 검색어를 바꾼 뒤 결과가 아직 오지 않음 ('결과 없음'을 잠깐 띄우지 않게)
    var searchPending = false
    /// 목록에서 이 항목으로 스크롤 (검색 결과에서 '목록에서 보기')
    var scrollTarget: String?

    // 진행·상태
    var scanning: Set<String> = []
    var progressText = ""
    var errorMessage: String?
    var errorTitle = AppModel.defaultErrorTitle
    static var defaultErrorTitle: String { String(localized: "실패", comment: "Generic error alert title") }
    /// 알림에 붙는 추가 버튼 (설정 열기·Finder에서 보기)
    var errorAction: AlertAction?
    var canUndo = false
    /// 휴지통 이동 확인을 기다리는 경로들 (nil이면 다이얼로그 닫힘)
    var trashRequest: [String]?
    /// 인덱스에 없는 항목(예: ~/Library)이라 화면이 이미 아는 크기
    var trashRequestBytes: Int64?
    /// 확인 창에 덧붙일 안내 (예: 보호된 항목은 건너뜀)
    var trashRequestNote: String?
    /// 확인을 기다리는 여러 파일 이동
    var moveRequest: MoveRequest?
    var lastOperationSummary: String?
    /// 이 앱에 전체 디스크 접근이 있는가 — 휴지통에서 ⌘Z로 꺼내기, 다른 앱 데이터 폴더 지우기에 필요
    var hasFullDiskAccess = Executor.hasFullDiskAccess
    /// 상태 줄의 인덱스 통계 (변경이 있으면 비우고 뒤에서 다시 센다)
    var stats: IndexStore.Stats?

    /// 앱을 켠 시점의 마지막 작업 번호 — 되돌리기는 그 뒤 작업만 (며칠 전 작업이 갑자기 되돌아가지 않게)
    let sessionStartOpID: Int64
    /// 창의 실행 취소 관리자 — 편집 › 실행 취소(⌘Z)가 글자 입력 중엔 글자를, 그 밖에선 파일 작업을 되돌리게 한다
    @ObservationIgnored weak var undoManager: UndoManager?

    enum AlertAction: Equatable {
        case openFullDiskAccessSettings
        case reveal(String)
    }

    private var childrenCache: [String: [FileRecord]] = [:]
    private var folderSummaries: [String: (files: Int, bytes: Int64)] = [:]
    private var summaryLoading: Set<String> = []
    private var statsLoading = false
    private var watcher: Watcher?
    @ObservationIgnored private var searchTask: Task<Void, Never>?

    // 하위 화면 (self가 필요해 init 뒤에 채움)
    @ObservationIgnored private var duplicatesStorage: DuplicatesModel?
    @ObservationIgnored private var compareStorage: CompareModel?
    @ObservationIgnored private var categoriesStorage: CategoriesModel?
    @ObservationIgnored private var leftoversStorage: LeftoversModel?
    var duplicates: DuplicatesModel { duplicatesStorage! }
    var compare: CompareModel { compareStorage! }
    var categories: CategoriesModel { categoriesStorage! }
    var leftovers: LeftoversModel { leftoversStorage! }

    /// 실행 직후 첫 스캔을 시작하기 전(제외 대상 정리 중)도 '스캔 중'으로 본다 — 그사이 찾기를 눌러도 빈 인덱스로 답하지 않게
    var startupPending = true
    var isScanning: Bool { startupPending || !scanning.isEmpty }
    var isSearching: Bool { !filter.text.isEmpty }

    // MARK: - 생성

    static func load() -> AppModel {
        do {
            let m = try AppModel()
            m.applyLaunchArguments(CommandLine.arguments)
            return m
        } catch {
            let alert = NSAlert()
            alert.messageText = String(localized: "인덱스를 열 수 없습니다")
            alert.informativeText = error.localizedDescription
            alert.runModal()
            exit(1)
        }
    }

    init() throws {
        store = try IndexStore(path: IndexStore.defaultURL().path)
        executor = Executor(store: store)
        sessionStartOpID = (try? store.maxOperationID()) ?? 0
        let defaults = UserDefaults.standard
        let home = NSHomeDirectory()
        folders = defaults.stringArray(forKey: "folders") ?? [home + "/Downloads", home + "/Desktop"]
        expanded = Set(defaults.stringArray(forKey: "expanded") ?? folders)
        sortField = SortField(rawValue: defaults.string(forKey: "sortField") ?? "") ?? .name
        sortAscending = defaults.object(forKey: "sortAscending") as? Bool ?? true
        duplicatesStorage = DuplicatesModel(app: self)
        compareStorage = CompareModel(app: self)
        categoriesStorage = CategoriesModel(app: self)
        leftoversStorage = LeftoversModel(app: self)
        restartWatcher()
        // 예전에 들어온 제외 대상(휴지통 등) 정리 → 꺼져 있던 동안의 변화를 따라잡는 재스캔 (뒤에서)
        let store = self.store, rules = self.rules
        Task.detached(priority: .utility) { [weak self] in
            _ = try? store.removeExcluded(by: rules)
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.startupPending = false
                self.rescanAll()
                if self.scanning.isEmpty { self.duplicates.resumeAfterScan() }   // 모은 폴더가 없으면 기다릴 스캔도 없다
            }
        }
    }

    /// 개발·검증용: `--screen duplicates|compare|categories|leftovers [--run] [--left 경로 --right 경로]`
    func applyLaunchArguments(_ args: [String]) {
        func value(_ flag: String) -> String? {
            guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
            return args[i + 1]
        }
        switch value("--screen") {
        case "duplicates": screen = .duplicates
        case "compare": screen = .compare
        case "categories": screen = .categories
        case "leftovers": screen = .leftovers
        default: break
        }
        if let l = value("--left") { compare.set(side: .left, path: l) }
        if let r = value("--right") { compare.set(side: .right, path: r) }
        if let q = value("--search") { filter.text = q }
        if args.contains("--run") {
            switch screen {
            case .duplicates: duplicates.run()
            case .compare: compare.run()
            case .categories: categories.run()
            case .leftovers: leftovers.run()
            case .collection: break
            }
        }
    }

    private func persist(_ value: [String], key: String) {
        UserDefaults.standard.set(value, forKey: key)
    }

    // MARK: - 모은 폴더

    func addFolderViaPanel() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.prompt = String(localized: "추가", comment: "Open panel button: add the chosen folders")
        panel.message = String(localized: "한 화면에 같이 볼 폴더를 고르세요")
        guard panel.runModal() == .OK else { return }
        for url in panel.urls { addFolder(url.path) }
    }

    func addFolder(_ rawPath: String) {
        let p = ScanRules.normalize(rawPath)
        guard !folders.contains(p) else { return }
        folders.append(p)
        expanded.insert(p)
        rescan(p)
    }

    func removeFolder(_ path: String) {
        folders.removeAll { $0 == path }
        expanded = expanded.filter { $0 != path && !$0.hasPrefix(path + "/") }
    }

    func moveFolder(from: IndexSet, to: Int) {
        folders.move(fromOffsets: from, toOffset: to)
    }

    func rootRecord(_ path: String) -> FileRecord? {
        try? store.record(at: path)
    }

    // MARK: - 트리 데이터

    /// 폴더의 직계 자식. 폴더 먼저, 선택한 정렬. 필터 적용.
    func children(of path: String) -> [FileRecord] {
        let all: [FileRecord]
        if let cached = childrenCache[path] {
            all = cached
        } else {
            all = (try? store.children(of: path)) ?? []
            childrenCache[path] = all
        }
        return sorted(all.filter { filter.matches($0) })
    }

    /// 폴더 먼저, 선택한 기준으로 정렬.
    func sorted(_ records: [FileRecord]) -> [FileRecord] {
        records.sorted { a, b in
            let aDir = a.kind == .directory, bDir = b.kind == .directory
            if aDir != bDir { return aDir }
            let result: Bool
            switch sortField {
            case .name: result = a.name.localizedStandardCompare(b.name) == .orderedAscending
            case .date: result = a.modified < b.modified
            case .size:
                let sa = aDir ? (folderSummaries[a.path]?.bytes ?? -1) : a.size
                let sb = bDir ? (folderSummaries[b.path]?.bytes ?? -1) : b.size
                result = sa == sb ? a.name.localizedStandardCompare(b.name) == .orderedAscending : sa < sb
            }
            return sortAscending ? result : !result
        }
    }

    /// 검색 결과 중 형식·크기·기간 필터에 맞는 것, 정렬해서.
    var visibleSearchResults: [FileRecord] { sorted(searchResults.filter { filter.matches($0) }) }

    func invalidate() {
        childrenCache.removeAll()
        folderSummaries.removeAll()
        stats = nil
        if isSearching { updateSearch() }
    }

    func isExpanded(_ path: String) -> Binding<Bool> {
        Binding(
            get: { self.expanded.contains(path) },
            set: { on in if on { self.expanded.insert(path) } else { self.expanded.remove(path) } })
    }

    func expandSelection(_ open: Bool) {
        for p in selection where (rootRecord(p)?.kind ?? .file) == .directory {
            if open { expanded.insert(p) } else { expanded.remove(p) }
        }
    }

    /// 더블클릭·Enter: 폴더는 펼치고 접기(검색 중이면 목록에서 보여 주기), 파일은 열기.
    func activate(_ paths: Set<String>) {
        if paths.count == 1, let p = paths.first, (rootRecord(p)?.kind ?? .file) == .directory {
            if isSearching { revealInList(p) } else { isExpanded(p).wrappedValue.toggle() }
        } else {
            for p in paths where (rootRecord(p)?.kind ?? .file) != .directory { open(p) }
        }
    }

    /// 지금 목록에 보이는 항목 — 필터에 걸리거나 접힌 폴더 안에 있어 안 보이는 항목이 선택에 남아 함께 지워지지 않게.
    func visiblePaths() -> Set<String> {
        if isSearching { return Set(visibleSearchResults.map(\.path)) }
        var out = Set<String>()
        func walk(_ path: String) {
            out.insert(path)
            guard expanded.contains(path) else { return }
            for c in children(of: path) {
                if c.kind == .directory { walk(c.path) } else { out.insert(c.path) }
            }
        }
        for f in folders { walk(f) }
        return out
    }

    private func pruneSelection() {
        guard !selection.isEmpty else { return }
        let visible = visiblePaths()
        let pruned = selection.intersection(visible)
        if pruned.count != selection.count { selection = pruned }
    }

    /// 검색 결과의 항목을 폴더 목록에서 보여 준다 (검색을 끄고, 들어 있는 폴더들을 펼치고, 고른 뒤 스크롤).
    func revealInList(_ path: String) {
        guard let root = folders.first(where: { path == $0 || path.hasPrefix($0 + "/") }) else { reveal(path); return }
        filter.text = ""
        var p = (path as NSString).deletingLastPathComponent
        var ancestors: Set<String> = [root]
        while p.count > root.count, p.hasPrefix(root) {
            ancestors.insert(p)
            p = (p as NSString).deletingLastPathComponent
        }
        expanded.formUnion(ancestors)
        selection = [path]
        scrollTarget = path
    }

    private func updateSearch() {
        searchTask?.cancel()
        let text = filter.text
        guard !text.isEmpty else { searchResults = []; searchPending = false; return }
        searchPending = true
        let store = self.store, roots = folders
        searchTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))   // 글자를 칠 때마다 찾지 않게
            guard !Task.isCancelled else { return }
            let found = await Task.detached(priority: .userInitiated) {
                (try? store.search(name: text, under: roots)) ?? []
            }.value
            guard !Task.isCancelled, let self else { return }
            self.searchResults = found
            self.searchPending = false
            self.pruneSelection()
        }
    }

    func folderSummary(_ path: String) -> (files: Int, bytes: Int64)? { folderSummaries[path] }

    func loadFolderSummary(_ path: String) {
        guard folderSummaries[path] == nil, !summaryLoading.contains(path) else { return }
        summaryLoading.insert(path)
        let store = self.store
        Task.detached(priority: .utility) { [weak self] in
            let s = try? store.summary(under: path)
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.summaryLoading.remove(path)
                if let s { self.folderSummaries[path] = s }
            }
        }
    }

    func loadStats() {
        guard stats == nil, !statsLoading else { return }
        statsLoading = true
        let store = self.store
        Task.detached(priority: .utility) { [weak self] in
            let s = try? store.stats()
            await MainActor.run { [weak self] in
                self?.statsLoading = false
                self?.stats = s
            }
        }
    }

    var filterDescription: String {
        var parts: [String] = []
        if !filter.text.isEmpty { parts.append("“\(filter.text)”") }
        if filter.category != .all { parts.append(filter.category.label) }
        if filter.sizeFloor != .any { parts.append(filter.sizeFloor.label) }
        if filter.recent != .any { parts.append(filter.recent.label) }
        return parts.joined(separator: " · ")
    }

    // MARK: - 미리보기

    var quickLookItems: [URL] {
        let items = selection.sorted().map { URL(fileURLWithPath: $0) }
        return items.isEmpty ? (quickLookURL.map { [$0] } ?? []) : items
    }

    func toggleQuickLook() {
        if quickLookURL != nil { quickLookURL = nil; return }
        guard let first = selection.sorted().first else { return }
        quickLookURL = URL(fileURLWithPath: first)
    }

    // MARK: - 끌어 놓기

    /// 선택된 항목을 끌면 선택 전체가, 아니면 그 항목 하나만.
    func dragPayload(for path: String) -> [String] {
        selection.contains(path) ? selection.sorted() : [path]
    }

    /// 폴더에 떨어뜨린 경로들을 그 폴더로 이동. 실행 즉시(끌어 놓기 자체가 승인) + ⌘Z로 한 번에 되돌리기.
    @discardableResult
    func drop(_ paths: [String], into folder: String) -> Bool {
        let targets = paths.filter { p in
            p != folder && !folder.hasPrefix(p + "/") && (p as NSString).deletingLastPathComponent != folder
        }
        guard !targets.isEmpty else { return false }
        let done = performMoves(targets.map { ($0, folder) }, folderName: (folder as NSString).lastPathComponent)
        expanded.insert(folder)
        return done > 0
    }

    // MARK: - 스캔

    func rescanAll() { for s in folders { rescan(s) } }

    func rescan(_ path: String) {
        guard !scanning.contains(path) else { return }
        scanning.insert(path)
        let scanner = Scanner(rules: rules)
        let store = self.store
        Task.detached(priority: .utility) { [weak self] in
            var failure: String?
            var lastReport = Date.distantPast
            do {
                try scanner.scan(roots: [URL(fileURLWithPath: path)], into: store) { p in
                    let now = Date()
                    guard now.timeIntervalSince(lastReport) > 0.3 else { return }
                    lastReport = now
                    let count = p.scanned.formatted()
                    let current = p.currentPath.replacingOccurrences(of: NSHomeDirectory(), with: "~")
                    let text = String(localized: "\(count)개 … \(current)", comment: "Scan progress: number of items scanned, then the current path")
                    Task { @MainActor in self?.progressText = text }
                }
            } catch { failure = error.localizedDescription }
            let result = failure
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.scanning.remove(path)
                if self.scanning.isEmpty { self.progressText = "" }
                self.invalidate()
                if let result { self.showError(message: result) }
                if self.scanning.isEmpty { self.duplicates.resumeAfterScan() }
            }
        }
    }

    // MARK: - 변경 감시

    private func restartWatcher() {
        watcher?.stop()
        guard !folders.isEmpty else { watcher = nil; return }
        let update = Watcher.indexUpdater(store: store, rules: rules)
        watcher = Watcher(paths: folders, latency: 1.5) { [weak self] changed in
            update(changed)
            Task { @MainActor in self?.invalidate() }
        }
        watcher?.start()
    }

    // MARK: - 지금 화면의 선택 (메뉴 명령은 보이는 화면의 선택에만 작동)

    /// 지금 화면에서 고른 항목의 실제 경로.
    var currentSelectionPaths: [String] {
        switch screen {
        case .collection: return selection.sorted()
        case .duplicates: return duplicates.selectedFiles.map(\.path)
        case .compare: return compare.selectedPaths(oneSidedOnly: false)
        case .categories: return categories.selection.sorted()
        case .leftovers: return leftovers.selectedItems.map(\.path)
        }
    }

    var hasSelectionOnCurrentScreen: Bool { !currentSelectionPaths.isEmpty }
    var canMoveOnCurrentScreen: Bool { screen != .leftovers && hasSelectionOnCurrentScreen }

    /// ⌘⌫ — 다른 화면에서 골라 둔 항목이 섞여 엉뚱한 파일이 지워지지 않도록 지금 화면의 선택만.
    func trashOnCurrentScreen() {
        switch screen {
        case .collection: requestTrashSelection()
        case .duplicates: duplicates.trashSelection()
        case .compare: compare.trashSelected(side: nil)
        case .categories: categories.trashSelection()
        case .leftovers: leftovers.trashSelection()
        }
    }

    /// ⇧⌘M
    func moveOnCurrentScreen() {
        switch screen {
        case .collection: requestMoveSelection()
        case .duplicates: requestMove(duplicates.selectedFiles.map(\.path))
        case .compare: requestMove(compare.selectedPaths(oneSidedOnly: true))
        case .categories: categories.moveSelectionToFolder()
        case .leftovers: break
        }
    }

    /// ⇧⌘R
    func revealOnCurrentScreen() {
        let urls = currentSelectionPaths.map { URL(fileURLWithPath: $0) }
        guard !urls.isEmpty else { return }
        NSWorkspace.shared.activateFileViewerSelecting(urls)
    }

    // MARK: - 실행 (승인 후에만)

    /// 모아 보기의 선택을 휴지통으로 (확인 창 경유). 지금 보이는 항목만.
    func requestTrashSelection() {
        let visible = visiblePaths()
        var paths = selection.filter { visible.contains($0) }
        var note: String?
        // 필터를 켠 채 전부 선택(⌘A)하면 길잡이로 보이는 폴더까지 딸려 온다 → 파일과 섞여 있으면 폴더는 뺀다
        if filter.isActive {
            let dirs = paths.filter { (rootRecord($0)?.kind ?? .file) == .directory }
            if !dirs.isEmpty && dirs.count < paths.count {
                paths.subtract(dirs)
                note = String(localized: "필터가 켜져 있어 함께 선택된 폴더 \(dirs.count)개는 뺐습니다.")
            }
        }
        requestTrash(Array(paths), note: note)
    }

    /// 휴지통으로 옮길지 확인 창을 띄운다. 실제로 존재하는 경로만 — 화면 식별자나 이미 사라진 항목은 조용히 제외.
    func requestTrash(_ candidates: [String], knownBytes: Int64? = nil, note: String? = nil) {
        let paths = candidates.filter { $0.hasPrefix("/") && FileManager.default.fileExists(atPath: $0) }.sorted()
        guard !paths.isEmpty else { return }
        hasFullDiskAccess = Executor.hasFullDiskAccess
        trashRequestBytes = knownBytes
        trashRequestNote = note.flatMap { $0.isEmpty ? nil : $0 }
        trashRequest = paths
    }

    /// 확인 다이얼로그에서 승인된 뒤 호출. 한 묶음으로 기록해 ⌘Z 한 번에 되돌린다. 실패는 하나로 모아서 알린다.
    func confirmTrash() {
        guard let paths = trashRequest else { return }
        trashRequest = nil
        trashRequestBytes = nil
        trashRequestNote = nil
        var done = 0
        var failed: [(path: String, error: Error)] = []
        executor.batch {
            for p in paths {
                do { try executor.trash(p); done += 1 } catch { failed.append((p, error)) }
            }
        }
        if done > 0 { registerUndo(String(localized: "휴지통으로 이동", comment: "Undo action name")) }
        if !failed.isEmpty {
            let names = failed.prefix(3).map { "‘\(($0.path as NSString).lastPathComponent)’" }.joined(separator: ", ")
            let more = failed.count > 3 ? " " + Self.andMore(failed.count - 3) : ""
            let permission = failed.contains { Executor.isPermissionError($0.error) }
            showError(
                title: String(localized: "\(failed.count)개를 휴지통으로 옮기지 못했습니다"),
                message: "\(names)\(more)\n\n" + (permission
                    ? String(localized: "macOS가 다른 앱의 데이터로 보호하는 위치입니다. 시스템 설정 › 개인정보 보호 및 보안 › 전체 디스크 접근에서 Multi-Finder를 켜면 옮길 수 있습니다.")
                    : failed[0].error.localizedDescription),
                action: permission ? .openFullDiskAccessSettings : nil)
        }
        let undoHint = hasFullDiskAccess
            ? String(localized: "⌘Z로 되돌리기")
            : String(localized: "Finder 휴지통에서 ‘되돌려 놓기’로 복구")
        afterOperation(summary: done > 0
            ? String(localized: "\(done)개 항목을 휴지통으로 옮겼습니다 — \(undoHint)")
            : String(localized: "옮긴 항목이 없습니다"))
    }

    /// 모아 보기의 선택을 고른 폴더로 (폴더 고르기 창이 곧 승인).
    func requestMoveSelection() {
        let visible = visiblePaths()
        requestMove(selection.filter { visible.contains($0) }.sorted())
    }

    func requestMove(_ candidates: [String]) {
        let paths = candidates.filter { FileManager.default.fileExists(atPath: $0) }.sorted()
        guard !paths.isEmpty else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "여기로 이동", comment: "Open panel button: move items into the chosen folder")
        panel.message = String(localized: "\(paths.count)개 항목을 옮길 폴더")
        guard panel.runModal() == .OK, let dest = panel.url else { return }
        performMoves(paths.map { ($0, dest.path) }, folderName: dest.lastPathComponent)
    }

    /// 여러 파일 이동을 확인 창으로 묻는다 (폴더 비교의 '한쪽으로 몰기').
    func requestBatchMove(_ request: MoveRequest) {
        guard !request.items.isEmpty else { return }
        moveRequest = request
    }

    func confirmMove() {
        guard let request = moveRequest else { return }
        moveRequest = nil
        performMoves(request.items, createDirectories: request.createDirectories, folderName: request.folderName)
    }

    /// 실제 이동 — 한 묶음으로 기록하고 ⌘Z를 등록한다. 옮긴 개수를 돌려준다.
    @discardableResult
    private func performMoves(_ items: [(source: String, directory: String)], createDirectories: Bool = false, folderName: String) -> Int {
        var done = 0
        var failures: [String] = []
        executor.batch {
            for item in items {
                do {
                    try executor.move(item.source, toDirectory: item.directory, createDirectory: createDirectories)
                    done += 1
                } catch { failures.append(error.localizedDescription) }
            }
        }
        if done > 0 { registerUndo(String(localized: "이동", comment: "Undo action name for moving files")) }
        if let first = failures.first {
            showError(message: failures.count > 1 ? first + "\n" + Self.andMore(failures.count - 1) : first)
        }
        afterOperation(summary: Self.movedSummary(done, to: folderName))
        return done
    }

    /// "n개 항목을 ○○(으)로 옮겼습니다 — ⌘Z로 되돌리기"
    static func movedSummary(_ count: Int, to folderName: String) -> String {
        String(localized: "\(count)개 항목을 \(folderName)(으)로 옮겼습니다 — ⌘Z로 되돌리기")
    }

    /// "1,234개" · "1,234 items" · "1 item" — 천 단위 구분은 숫자 서식에, 단수·복수는 단위 낱말에 맡긴다.
    static func itemCount(_ n: Int) -> String {
        // 문자열 카탈로그의 복수형은 숫자를 문장 안에 넣어야만 해서, 숫자 서식(천 단위 구분)을 지키려고 낱말을 따로 고른다
        let unit = n == 1
            ? String(localized: "개 (항목 하나)", comment: "Unit word after the number 1, e.g. '1 item'. Korean: 개")
            : String(localized: "개 (항목 여러 개)", comment: "Unit word after any number other than 1, e.g. '12 items'. Korean: 개")
        let number = n.formatted()
        return String(localized: "\(number)\(unit)", comment: "An item count: formatted number followed by its unit word. Add a space between them if your language uses one")
    }

    /// 목록을 줄여 보여줄 때 붙이는 "외 n개"
    static func andMore(_ count: Int) -> String {
        String(localized: "외 \(count)개", comment: "Suffix after a shortened list: and N more")
    }

    func showError(title: String? = nil, message: String, action: AlertAction? = nil) {
        errorTitle = title ?? Self.defaultErrorTitle
        errorAction = action
        errorMessage = message
    }

    func clearError() {
        errorMessage = nil
        errorAction = nil
        errorTitle = Self.defaultErrorTitle
    }

    func perform(_ action: AlertAction) {
        switch action {
        case .openFullDiskAccessSettings: openFullDiskAccessSettings()
        case .reveal(let path): reveal(path)
        }
    }

    func openFullDiskAccessSettings() {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") {
            NSWorkspace.shared.open(url)
        }
    }

    func newFolder(in parent: String) {
        let base = String(localized: "제목 없는 폴더", comment: "Default name for a new folder (same as Finder's)")
        var name = base
        var path = (parent as NSString).appendingPathComponent(name)
        var n = 2
        while FileManager.default.fileExists(atPath: path) {
            name = "\(base) \(n)"; n += 1
            path = (parent as NSString).appendingPathComponent(name)
        }
        do {
            try FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: false)
            if let rec = Scanner.record(for: URL(fileURLWithPath: path)) {
                try store.upsert([rec], generation: store.currentGeneration)
            }
            expanded.insert(parent)
            invalidate()
            selection = [path]
            lastOperationSummary = String(localized: "새 폴더를 만들었습니다 — Finder에서 이름을 바꾸세요")
        } catch { showError(message: error.localizedDescription) }
    }

    // MARK: - 되돌리기

    /// 파일 작업 묶음 하나를 창의 실행 취소 목록에 올린다. 되돌릴 때는 '가장 최근에 남은 묶음'을 되돌린다.
    private func registerUndo(_ actionName: String) {
        guard let um = undoManager else { return }
        um.registerUndo(withTarget: self) { model in
            MainActor.assumeIsolated { model.undo() }
        }
        um.setActionName(actionName)
    }

    /// 앱을 켠 뒤 한 마지막 작업 묶음을 되돌린다 (편집 › 실행 취소, 상태 줄의 '되돌리기').
    func undo() {
        do {
            let r = try executor.undo(after: sessionStartOpID)
            for p in r.rescan { rescan(p) }
            guard !r.isEmpty else { refreshUndoState(); return }
            var parts: [String] = []
            if r.restored.count == 1, let op = r.restored.first {
                let name = (op.source as NSString).lastPathComponent
                parts.append(String(localized: "되돌렸습니다: \(name)"))
            } else if !r.restored.isEmpty {
                parts.append(String(localized: "\(r.restored.count)개 항목을 되돌렸습니다"))
            }
            if !r.missing.isEmpty { parts.append(String(localized: "\(r.missing.count)개는 찾을 수 없어 건너뜀")) }
            if !r.blocked.isEmpty {
                let names = r.blocked.prefix(3).map { "‘\(($0 as NSString).lastPathComponent)’" }.joined(separator: ", ")
                let more = r.blocked.count > 3 ? " " + Self.andMore(r.blocked.count - 3) : ""
                showError(
                    title: String(localized: "되돌리지 못한 항목이 있습니다"),
                    message: String(localized: "원래 자리에 같은 이름의 항목이 있어 되돌리지 못했습니다: \(names)\(more)\n\n그 항목을 옮긴 뒤 다시 되돌리기를 누르세요."))
            }
            afterOperation(summary: parts.joined(separator: " · "))
        } catch ExecutorError.undoNeedsTrashAccess(let trashed, let original) {
            refreshUndoState()
            showError(
                title: String(localized: "휴지통에서 꺼낼 권한이 없습니다"),
                message: ExecutorError.undoNeedsTrashAccess(trashed: trashed, original: original).errorDescription ?? "",
                action: .reveal(trashed))
        } catch {
            refreshUndoState()
            showError(message: error.localizedDescription)
        }
    }

    private func refreshUndoState() {
        canUndo = (try? store.lastUndoableOperation(after: sessionStartOpID)) != nil
    }

    private func afterOperation(summary: String) {
        noteOperation(summary)
        // 지금 화면의 결과는 바로 다시 계산하고, 다른 화면의 결과는 낡았다고 표시해 돌아갈 때 다시 계산한다
        switch screen {
        case .duplicates: duplicates.afterOperation()
        case .compare: if compare.result != nil { compare.run() }
        case .categories: categories.reload()
        case .leftovers: leftovers.run()
        case .collection: break
        }
        if screen != .duplicates, duplicates.report != nil { duplicates.isStale = true }
        if screen != .compare, compare.result != nil { compare.isStale = true }
        if screen != .leftovers, leftovers.scanned { leftovers.isStale = true }
        if screen != .categories { categories.reload() }
    }

    /// 다른 화면에서 파일이 바뀐 뒤 이 화면으로 돌아왔을 때 결과를 다시 계산한다.
    func refreshStaleScreen() {
        switch screen {
        case .duplicates: if duplicates.isStale { duplicates.run() }
        case .compare: if compare.isStale { compare.run() }
        case .leftovers: if leftovers.isStale { leftovers.run() }
        case .categories: categories.reload()
        case .collection: break
        }
    }

    /// 파일 변경 뒤 공통 뒷정리 (하위 화면에서 직접 실행기를 쓴 경우도 호출).
    func noteOperation(_ summary: String) {
        selection.removeAll()
        quickLookURL = nil
        invalidate()
        refreshUndoState()
        lastOperationSummary = summary
    }

    func reveal(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    func open(_ path: String) {
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }
}
