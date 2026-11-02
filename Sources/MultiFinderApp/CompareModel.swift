import AppKit
import Observation
import MultiFinderCore

/// '폴더 비교' 화면의 상태.
@Observable
@MainActor
final class CompareModel {
    unowned let app: AppModel

    var left: String?
    var right: String?
    var result: FolderDiffResult?
    var running = false
    var progress = ""
    /// 상태별 보기. 바꾸면 안 보이게 된 줄은 선택에서 뺀다 (안 보이는 줄이 함께 지워지지 않게).
    var statusFilter: DiffStatus? = nil {
        didSet { selection.formIntersection(Set(visibleEntries.map(\.id))) }
    }
    var selection: Set<String> = []   // DiffEntry.id
    /// 다른 화면에서 파일이 바뀌어 결과가 낡음 — 이 화면으로 돌아오면 다시 비교한다
    var isStale = false

    init(app: AppModel) { self.app = app }

    func pick(side: Side) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = side == .left
            ? String(localized: "왼쪽으로", comment: "Open panel button: use the folder as the left side of the comparison")
            : String(localized: "오른쪽으로", comment: "Open panel button: use the folder as the right side of the comparison")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        set(side: side, path: ScanRules.normalize(url.path))
    }

    func set(side: Side, path: String) {
        if side == .left { left = path } else { right = path }
        result = nil
        selection.removeAll()
    }

    func swap() {
        (left, right) = (right, left)
        result = nil
        selection.removeAll()
    }

    var canRun: Bool {
        guard let left, let right, !running else { return false }
        // 같은 폴더이거나 한쪽이 다른 쪽 안에 있으면 비교가 의미 없다 (자기 자신과 겹쳐 보임)
        return left != right && !left.hasPrefix(right + "/") && !right.hasPrefix(left + "/")
    }

    func run() {
        guard let left, let right, canRun else { return }
        running = true
        isStale = false
        progress = String(localized: "폴더 스캔 중…")
        let store = app.store
        let rules = app.rules
        let diff = FolderDiff(store: store)
        Task.detached(priority: .userInitiated) { [weak self] in
            var outcome: Result<FolderDiffResult, Error>
            do {
                // 언제 바뀌었을지 모르므로 매번 두 폴더를 다시 훑는다 (바뀐 파일만 갱신 — 그대로인 파일의 지문은 재사용)
                try Scanner(rules: rules).scan(roots: [URL(fileURLWithPath: left), URL(fileURLWithPath: right)], into: store)
                var last = Date.distantPast
                let r = try diff.compare(left: left, right: right) { done, total in
                    guard Date().timeIntervalSince(last) > 0.3 else { return }
                    last = Date()
                    let text = String(localized: "내용 확인 중 … \(done)/\(total)")
                    Task { @MainActor in self?.progress = text }
                }
                outcome = .success(r)
            } catch { outcome = .failure(error) }
            let final = outcome
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.running = false
                self.progress = ""
                switch final {
                case .success(let r):
                    self.result = r
                    self.selection.formIntersection(Set(r.entries.map(\.id)))
                    self.app.invalidate()
                case .failure(let e): self.app.showError(message: e.localizedDescription)
                }
            }
        }
    }

    var visibleEntries: [DiffEntry] {
        guard let result else { return [] }
        guard let f = statusFilter else { return result.entries }
        return result.entries.filter { $0.status == f }
    }

    private var selectedEntries: [DiffEntry] {
        guard let result else { return [] }
        return result.entries.filter { selection.contains($0.id) }
    }

    /// 고른 줄의 실제 파일 경로. `oneSidedOnly`면 양쪽에 다 있는 줄은 뺀다 (어느 쪽인지 정할 수 없으므로).
    func selectedPaths(oneSidedOnly: Bool) -> [String] {
        selectedEntries.flatMap { e -> [String] in
            let sides = [e.left?.path, e.right?.path].compactMap { $0 }
            return oneSidedOnly && sides.count > 1 ? [] : sides
        }
    }

    /// 고른 줄에 그 쪽 파일이 있는가 (메뉴 활성화용)
    func selectionHas(side: Side) -> Bool {
        selectedEntries.contains { side == .left ? $0.left != nil : $0.right != nil }
    }

    // MARK: - 몰기

    /// 한쪽에만 있는 파일을 반대쪽으로, 상대 경로를 유지한 채 옮긴다 — 확인 창을 거친다.
    func moveOnly(from side: Side) {
        guard let result else { return }
        let status: DiffStatus = side == .left ? .leftOnly : .rightOnly
        let targetRoot = side == .left ? result.right : result.left
        let items: [(source: String, directory: String)] = result.entries.filter { $0.status == status }.compactMap { e in
            guard let rec = side == .left ? e.left : e.right else { return nil }
            let dir = (targetRoot as NSString).appendingPathComponent((e.relativePath as NSString).deletingLastPathComponent)
            return (rec.path, dir)
        }
        guard !items.isEmpty else { return }
        let target = (targetRoot as NSString).lastPathComponent
        app.requestBatchMove(MoveRequest(
            title: String(localized: "\(items.count)개 항목을 \(target)(으)로 옮길까요?"),
            items: items, createDirectories: true, folderName: target))
    }

    // MARK: - 치우기

    /// 양쪽에 똑같이 있는 파일의 한쪽 사본을 휴지통으로 (확인 창 경유).
    /// 비교한 뒤로 어느 한쪽이라도 바뀌었으면 '같다'는 판정이 낡았으므로 그 쌍은 건너뛴다 — 남길 사본이 확실할 때만 지운다.
    func trashIdentical(on side: Side) {
        guard let result else { return }
        var paths: [String] = []
        var changed = 0
        for e in result.entries where e.status == .identical {
            guard let l = e.left, let r = e.right else { continue }
            guard Scanner.isUnchanged(l), Scanner.isUnchanged(r) else { changed += 1; continue }
            paths.append(side == .left ? l.path : r.path)
        }
        let note = changed > 0 ? String(localized: "비교한 뒤 바뀐 \(changed)개는 건너뜁니다 — 다시 비교해 주세요.") : nil
        guard !paths.isEmpty else {
            if let note { app.showError(title: String(localized: "지울 사본이 없습니다"), message: note) }
            return
        }
        app.requestTrash(paths, note: note)
    }

    /// 고른 줄의 파일을 휴지통으로 (확인 창 경유).
    /// 양쪽에 다 있는 줄은 `side`로 어느 쪽인지 정해야 한다 — nil이면 그런 줄은 건너뛴다(⌘⌫).
    func trashSelected(side: Side?) {
        var paths: [String] = []
        var bothSides = 0, changed = 0
        for e in selectedEntries {
            switch (e.left, e.right) {
            case let (l?, r?):
                guard let side else { bothSides += 1; continue }
                // '동일' 줄은 남는 쪽이 여전히 같은 내용일 때만
                if e.status == .identical, !(Scanner.isUnchanged(l) && Scanner.isUnchanged(r)) { changed += 1; continue }
                paths.append(side == .left ? l.path : r.path)
            case let (l?, nil):
                if side != .right { paths.append(l.path) }
            case let (nil, r?):
                if side != .left { paths.append(r.path) }
            case (nil, nil):
                break
            }
        }
        var notes: [String] = []
        if bothSides > 0 { notes.append(String(localized: "양쪽에 모두 있는 \(bothSides)개는 건너뜁니다 — 오른쪽 클릭 메뉴에서 지울 쪽을 고르세요.")) }
        if changed > 0 { notes.append(String(localized: "비교한 뒤 바뀐 \(changed)개는 건너뜁니다 — 다시 비교해 주세요.")) }
        guard !paths.isEmpty else {
            if !notes.isEmpty {
                app.showError(title: String(localized: "어느 쪽을 지울지 골라 주세요"), message: notes.joined(separator: "\n"))
            }
            return
        }
        app.requestTrash(paths, note: notes.joined(separator: "\n"))
    }

    enum Side { case left, right }
}
