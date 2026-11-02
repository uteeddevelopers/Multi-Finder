import Foundation
import MultiFinderCore
import Observation

/// '자동 분류' 화면의 상태. 파일은 그대로 두고(가상 묶음), 옮기는 건 사용자가 고른 뒤에만.
@Observable
@MainActor
final class CategoriesModel {
    unowned let app: AppModel

    var scope: String
    /// 하위 폴더까지 분류할지. 기본은 폴더 바로 아래만 — 어지러운 건 대개 거기고, 깊은 곳은 이미 정리된 프로젝트일 때가 많다.
    var includeSubfolders = false { didSet { reload() } }
    var categories: [Bucket] = []
    var results: [Classification] = []
    /// 왼쪽에서 고른 묶음. 바꾸면 안 보이게 된 파일은 선택에서 뺀다 (안 보이는 파일이 함께 옮겨지거나 지워지지 않게).
    var selectedCategory: String? = nil {
        didSet { selection.formIntersection(Set(visible.map(\.path))) }
    }
    var selection: Set<String> = []
    /// 아직 분류 안 된 파일 수 (다시 읽을 때만 센다 — 화면을 그릴 때마다 세지 않게)
    var pendingCount = 0
    var running = false
    var progressDone = 0
    var progressTotal = 0
    var currentFile = ""
    var availability = ClassifierEngine.availability
    var showEditor = false
    private var cancelRequested = false

    /// 처음 설치했을 때의 묶음 — 누구에게나 맞는 일반적인 구분. 화면 언어로 만들어지고, 이후엔 사용자가 고친다.
    /// (사용자의 묶음은 인덱스 DB에 저장되므로 이 목록을 바꿔도 이미 쓰던 묶음은 그대로다.)
    static var defaultCategories: [Bucket] {
        [
            Bucket(name: String(localized: "업무", comment: "Default bucket name"),
                   hint: String(localized: "회사·업무 문서 — 보고서, 기획서, 계약서, 회의 자료, 거래처와 주고받은 파일", comment: "Default bucket description")),
            Bucket(name: String(localized: "학교·공부", comment: "Default bucket name"),
                   hint: String(localized: "수업 자료, 강의 노트, 과제·레포트, 시험 자료, 학사 서류, 공부용 교재", comment: "Default bucket description")),
            Bucket(name: String(localized: "개인", comment: "Default bucket name"),
                   hint: String(localized: "개인 생활 — 영수증, 계약서, 여행, 취미, 건강·금융 등 사적인 문서", comment: "Default bucket description")),
            Bucket(name: String(localized: "개발", comment: "Default bucket name"),
                   hint: String(localized: "소스 코드, 프로젝트 압축본, 설정 파일, 개발 문서, 기술 자료", comment: "Default bucket description")),
            Bucket(name: String(localized: "디자인·미디어", comment: "Default bucket name"),
                   hint: String(localized: "로고, 사진, 이미지, 영상, 음악, 폰트, 디자인 시안", comment: "Default bucket description")),
            Bucket(name: installersName,
                   hint: String(localized: "dmg·pkg 설치 파일, 압축 해제 전 zip, 임시 캡처, 다시 받을 수 있는 것", comment: "Default bucket description")),
            Bucket(name: Bucket.fallbackName, hint: Bucket.fallbackHint),
        ]
    }

    /// 설치 파일이 모이는 묶음 — 모델 없이도 확장자만으로 넣는다. 어느 언어로 만든 이름이든 알아본다.
    static var installersName: String { String(localized: "설치·임시", comment: "Default bucket name: installers and temporary files") }
    static let installersNames: Set<String> = ["설치·임시", "Installers & Temporary", "インストーラ・一時ファイル", "安装包与临时文件"]

    init(app: AppModel) {
        self.app = app
        scope = app.folders.first ?? NSHomeDirectory() + "/Downloads"
        categories = (try? app.store.categories()) ?? []
        // 묶음이 하나도 없을 때(처음 설치)만 기본 묶음을 만든다. 이미 있는 묶음은 절대 덮어쓰지 않는다.
        if categories.isEmpty {
            categories = Self.defaultCategories
            try? app.store.saveCategories(categories)
        }
        reload()
    }

    func reload() {
        results = (try? app.store.classifications(under: scope, recursive: includeSubfolders)) ?? []
        selection.formIntersection(Set(visible.map(\.path)))
        pendingCount = (try? app.store.unclassifiedFiles(under: scope, extensions: Self.classifiableExtensions, recursive: includeSubfolders).count) ?? 0
    }

    func saveCategories(_ list: [Bucket]) {
        var cleaned = list.filter { !$0.name.trimmingCharacters(in: .whitespaces).isEmpty }
        if !cleaned.contains(where: \.isFallback) {
            cleaned.append(Bucket(name: Bucket.fallbackName, hint: Bucket.fallbackHint))
        }
        categories = cleaned
        UserDefaults.standard.set(true, forKey: "bucketsEdited")
        do { try app.store.saveCategories(cleaned) } catch { app.showError(message: error.localizedDescription) }
    }

    // MARK: - 보기

    var counts: [String: Int] {
        Dictionary(grouping: results, by: \.category).mapValues(\.count)
    }

    var visible: [Classification] {
        let base = selectedCategory.map { c in results.filter { $0.category == c } } ?? results
        return base.sorted { a, b in
            if a.confidence != b.confidence { return a.confidence > b.confidence }
            return a.path < b.path
        }
    }

    static let classifiableExtensions: Set<String> = {
        var s = TextExtractor.textExtensions.union(TextExtractor.codeExtensions)
            .union(TextExtractor.officeZipExtensions).union(TextExtractor.imageExtensions)
        s.formUnion(["pdf", "doc", "hwp", "zip", "dmg", "pkg", "mp4", "mov", "mp3", "m4a", "ai", "psd", "fig", "sketch", "ttf", "otf"])
        return s
    }()

    // MARK: - 분류 실행

    func run() {
        guard !running else { return }
        availability = ClassifierEngine.availability
        guard availability == .available else { return }
        running = true
        cancelRequested = false
        let store = app.store
        let scope = self.scope
        let cats = categories
        let recursive = includeSubfolders
        Task { [weak self] in
            guard let self else { return }
            let pending: [FileRecord]
            do { pending = try store.unclassifiedFiles(under: scope, extensions: Self.classifiableExtensions, recursive: recursive) }
            catch { self.app.showError(message: error.localizedDescription); self.running = false; return }
            self.progressTotal = pending.count
            self.progressDone = 0
            for rec in pending {
                if self.cancelRequested { break }
                self.currentFile = rec.name
                // 모델 없이도 분명한 것: 설치 파일 → '설치·임시'
                if ["dmg", "pkg", "iso", "xip", "mpkg"].contains(rec.ext),
                   let bucket = cats.first(where: { Self.installersNames.contains($0.name) }) {
                    try? store.saveClassification(Classification(
                        path: rec.path, category: bucket.name, confidence: 0.95,
                        summary: String(localized: "설치 파일 — 설치했다면 지워도 됨"),
                        modified: rec.modified, size: rec.size, manual: false))
                    self.progressDone += 1
                    continue
                }
                // 내용 추출은 무거우므로 백그라운드에서
                let text = await Task.detached(priority: .utility) {
                    TextExtractor.canExtract(ext: rec.ext) ? TextExtractor.extract(from: rec.url, ext: rec.ext) : nil
                }.value
                do {
                    let r = try await ClassifierEngine.classify(record: rec, text: text, categories: cats)
                    try store.saveClassification(Classification(
                        path: rec.path, category: r.category, confidence: r.confidence, summary: r.summary,
                        modified: rec.modified, size: rec.size, manual: false))
                } catch {
                    // 한 파일 실패는 건너뛴다 — '기타'로 기록해 두어 다음에 또 시도하지 않게
                    let reason = error.localizedDescription
                    try? store.saveClassification(Classification(
                        path: rec.path, category: cats.fallbackName, confidence: 0,
                        summary: String(localized: "분류 실패: \(reason)"),
                        modified: rec.modified, size: rec.size, manual: false))
                }
                self.progressDone += 1
                if self.progressDone % 5 == 0 { self.reload() }
            }
            self.running = false
            self.currentFile = ""
            self.reload()
        }
    }

    func cancel() { cancelRequested = true }

    /// 결과를 비우고 처음부터 (사용자가 직접 고친 건 유지).
    func reclassifyAll() {
        try? app.store.clearClassifications(under: scope, keepManual: true)
        reload()
        run()
    }

    // MARK: - 사용자 수정·이동

    func move(paths: Set<String>, to category: String) {
        for p in paths { try? app.store.reclassifyManually(path: p, to: category) }
        reload()
    }

    /// 선택한 파일들을 실제 폴더로 이동 (승인 = 폴더 선택 패널).
    func moveSelectionToFolder() {
        guard !selection.isEmpty else { return }
        app.requestMove(selection.sorted())
        reload()
    }

    func trashSelection() {
        guard !selection.isEmpty else { return }
        app.requestTrash(selection.sorted())
    }
}
