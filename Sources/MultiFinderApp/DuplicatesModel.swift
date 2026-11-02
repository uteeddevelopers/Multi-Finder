import Foundation
import Observation
import MultiFinderCore

/// '같은 내용' 화면의 상태.
@Observable
@MainActor
final class DuplicatesModel {
    unowned let app: AppModel

    var scope: String? = nil          // nil = 전체 인덱스, 아니면 이 폴더 아래 파일이 포함된 묶음만
    var minSize: SizeFloor = .mb1
    var options = DuplicateOptions()
    var report: DuplicateReport?
    var rawGroupCount = 0
    var running = false
    var progress = ""
    var selection: Set<String> = []
    /// 펼쳐 보는 폴더 사본 묶음
    var expandedTrees: Set<String> = []
    /// 다른 화면에서 파일이 바뀌어 결과가 낡음 — 이 화면으로 돌아오면 다시 찾는다
    var isStale = false

    init(app: AppModel) { self.app = app }

    func run() {
        guard !running else { return }
        running = true
        isStale = false
        progress = String(localized: "크기가 같은 파일 찾는 중…")
        let store = app.store
        let finder = DuplicateFinder(store: store)
        let minBytes = max(minSize.bytes, 1)
        let scope = scope
        let options = options
        Task.detached(priority: .userInitiated) { [weak self] in
            var result: Result<(DuplicateReport, Int), Error>
            do {
                var last = Date.distantPast
                let groups = try finder.find(minSize: minBytes, under: scope) { done, total in
                    guard Date().timeIntervalSince(last) > 0.3 else { return }
                    last = Date()
                    let text = String(localized: "내용 확인 중 … \(done)/\(total)")
                    Task { @MainActor in self?.progress = text }
                }
                let ignore = (try? store.ignoreList()) ?? IgnoreList()
                let report = DuplicateAnalyzer.analyze(groups, options: options, ignore: ignore)
                result = .success((report, groups.count))
            } catch { result = .failure(error) }
            let final = result
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.running = false
                self.progress = ""
                switch final {
                case .success(let (report, raw)):
                    self.report = report
                    self.rawGroupCount = raw
                    self.selection = self.selection.filter { path in
                        report.fileGroups.contains { $0.files.contains { $0.path == path } }
                    }
                case .failure(let e): self.app.showError(message: e.localizedDescription)
                }
            }
        }
    }

    // MARK: - 선택 도우미

    /// 각 묶음에서 첫 번째(경로순) 파일만 남기고 나머지를 선택 (펼쳐진 폴더 사본 묶음 포함)
    func selectAllButFirst() {
        guard let report else { return }
        let visibleTreeGroups = report.copyTrees.filter { expandedTrees.contains($0.id) }.flatMap(\.groups)
        selection = Set((report.fileGroups + visibleTreeGroups).flatMap { $0.files.dropFirst().map(\.path) })
    }

    /// 묶음 안에서 특정 폴더 아래 있는 파일만 선택 (예: 다운로드 쪽 사본만)
    func selectUnder(_ folder: String) {
        guard let report else { return }
        let prefix = ScanRules.normalize(folder) + "/"
        selection = Set(report.fileGroups.flatMap { g in
            // 묶음 전체가 그 폴더 안이면 하나는 남긴다
            let inside = g.files.filter { $0.path.hasPrefix(prefix) }
            return inside.count == g.files.count ? inside.dropFirst().map(\.path) : inside.map(\.path)
        })
    }

    // MARK: - 무시

    func ignore(group: DuplicateGroup) {
        do { try app.store.addIgnore(hash: group.hash); run() } catch { app.showError(message: error.localizedDescription) }
    }

    func ignore(folder: String) {
        do { try app.store.addIgnore(folder: folder); run() } catch { app.showError(message: error.localizedDescription) }
    }

    func ignore(tree: CopyTreeGroup) {
        do {
            for g in tree.groups { try app.store.addIgnore(hash: g.hash) }
            run()
        } catch { app.showError(message: error.localizedDescription) }
    }

    func clearIgnores() {
        do {
            let list = try app.store.ignoreList()
            for h in list.hashes { try app.store.removeIgnore(hash: h) }
            for f in list.folders { try app.store.removeIgnore(folder: f) }
            run()
        } catch { app.showError(message: error.localizedDescription) }
    }

    // MARK: - 실행

    /// 화면에 보이는 모든 파일 (폴더 사본 묶음 안의 것 포함)
    private var allFiles: [FileRecord] {
        guard let report else { return [] }
        return report.fileGroups.flatMap(\.files) + report.copyTrees.flatMap { $0.groups.flatMap(\.files) }
    }

    /// 선택 중 실제 파일만 — 묶음 헤더 식별자 등은 섞여 들어와도 걸러낸다
    var selectedFiles: [FileRecord] {
        var seen = Set<String>()
        return allFiles.filter { selection.contains($0.path) && seen.insert($0.path).inserted }
    }

    /// 고른 사본을 휴지통으로 (확인 창 경유). 안전장치:
    /// - 묶음마다 **적어도 하나는 남긴다** — 전부 골랐으면(⌘A 등) 첫 번째 사본은 빼고 지운다.
    /// - 찾은 뒤로 바뀌었거나 사라진 파일은 '같은 내용'이 더는 확실하지 않으므로 지우지 않는다.
    func trashSelection() {
        guard let report else { return }
        let groups = report.fileGroups + report.copyTrees.flatMap(\.groups)
        var toTrash: [FileRecord] = []
        var keptOne = 0, skippedGroups = 0, changedFiles = 0
        for g in groups {
            let chosen = g.files.filter { selection.contains($0.path) }
            guard !chosen.isEmpty else { continue }
            var trashing = chosen
            let keepers = g.files.filter { !selection.contains($0.path) && Scanner.isUnchanged($0) }
            if keepers.isEmpty {
                // 남길 사본이 없다 → 고른 것 중 그대로인 첫 번째를 남긴다. 그것도 없으면 이 묶음은 손대지 않는다.
                guard let keep = chosen.first(where: Scanner.isUnchanged) else { skippedGroups += 1; continue }
                trashing.removeAll { $0.path == keep.path }
                keptOne += 1
            }
            let unchanged = trashing.filter(Scanner.isUnchanged)
            changedFiles += trashing.count - unchanged.count
            toTrash += unchanged
        }
        var notes: [String] = []
        if keptOne > 0 { notes.append(String(localized: "\(keptOne)개 묶음은 모든 사본이 선택되어 하나씩 남깁니다.")) }
        if skippedGroups > 0 || changedFiles > 0 {
            notes.append(String(localized: "찾은 뒤에 바뀌었거나 사라진 파일이 있어 \(skippedGroups + changedFiles)곳은 건너뜁니다 — ‘다시 찾기’를 눌러 주세요."))
        }
        guard !toTrash.isEmpty else {
            if !notes.isEmpty { app.showError(title: String(localized: "지울 사본이 없습니다"), message: notes.joined(separator: "\n")) }
            return
        }
        app.requestTrash(toTrash.map(\.path), knownBytes: toTrash.reduce(0) { $0 + $1.size }, note: notes.joined(separator: "\n"))
    }

    func afterOperation() {
        selection.removeAll()
        run()
    }
}
