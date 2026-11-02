import XCTest
@testable import MultiFinderCore

final class MultiFinderCoreTests: XCTestCase {
    var base: URL!
    var root: URL!
    var store: IndexStore!

    override func setUpWithError() throws {
        base = FileManager.default.temporaryDirectory
            .appendingPathComponent("MultiFinderTests-\(UUID().uuidString)", isDirectory: true)
        root = base.appendingPathComponent("data", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        // 인덱스 파일(및 WAL 부속 파일)은 스캔 대상 밖에 둔다
        store = try IndexStore(path: base.appendingPathComponent("db/index.sqlite").path)
    }

    override func tearDownWithError() throws {
        store = nil
        try? FileManager.default.removeItem(at: base)
    }

    private func write(_ name: String, _ content: String) throws -> URL {
        let url = root.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try content.data(using: .utf8)!.write(to: url)
        return url
    }

    /// 테스트용 규칙: 임시 폴더는 숨김 경로(/private/var)에 있으므로 제외 목록을 비운다.
    private var openRules: ScanRules { ScanRules(skipHidden: false, excludedPrefixes: [], excludedDirectoryNames: []) }

    func testHashEqualForSameContent() throws {
        let a = try write("a.txt", "hello world")
        let b = try write("sub/b.txt", "hello world")
        let c = try write("c.txt", "hello worlD")
        XCTAssertEqual(try ContentHasher.sha256(of: a), try ContentHasher.sha256(of: b))
        XCTAssertNotEqual(try ContentHasher.sha256(of: a), try ContentHasher.sha256(of: c))
        XCTAssertEqual(try ContentHasher.quickHash(of: a, size: 11), try ContentHasher.quickHash(of: b, size: 11))
    }

    func testScanAndDuplicates() throws {
        _ = try write("one.txt", "same content here")
        _ = try write("deep/two.txt", "same content here")
        _ = try write("three.txt", "different content")
        _ = try write("four.txt", "")   // 빈 파일은 중복에서 제외

        let count = try Scanner(rules: openRules).scan(roots: [root], into: store)
        XCTAssertGreaterThanOrEqual(count, 5)

        let stats = try store.stats()
        XCTAssertEqual(stats.files, 4)

        let groups = try DuplicateFinder(store: store).find()
        XCTAssertEqual(groups.count, 1)
        XCTAssertEqual(groups[0].files.count, 2)
        XCTAssertEqual(groups[0].wastedBytes, 17)

        // 지문이 저장되어 두 번째 탐색은 재계산 없이 같은 결과
        XCTAssertEqual(try store.stats().hashed, 2)
        XCTAssertEqual(try DuplicateFinder(store: store).find().count, 1)

        // under: 그 아래 파일이 포함된 묶음만
        XCTAssertEqual(try DuplicateFinder(store: store).find(under: root.appendingPathComponent("deep").path).count, 1)
        XCTAssertEqual(try DuplicateFinder(store: store).find(under: root.appendingPathComponent("nowhere").path).count, 0)
    }

    func testRescanDropsDeletedFiles() throws {
        let gone = try write("gone.txt", "x")
        _ = try write("stay.txt", "y")
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        XCTAssertNotNil(try store.record(at: gone.path))
        try FileManager.default.removeItem(at: gone)
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        XCTAssertNil(try store.record(at: gone.path))
        XCTAssertNotNil(try store.record(at: root.appendingPathComponent("stay.txt").path))
    }

    func testMoveAndUndo() throws {
        let file = try write("move-me.txt", "payload")
        let dest = root.appendingPathComponent("target", isDirectory: true)
        try FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        try Scanner(rules: openRules).scan(roots: [root], into: store)

        let exec = Executor(store: store, protected: ProtectedPaths(prefixes: [], rootFolders: []))
        let moved = try exec.move(file.path, toDirectory: dest.path)
        XCTAssertTrue(FileManager.default.fileExists(atPath: moved))
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        XCTAssertNotNil(try store.record(at: moved))
        XCTAssertNil(try store.record(at: file.path))

        // 덮어쓰기 금지
        _ = try write("move-me.txt", "another")
        XCTAssertThrowsError(try exec.move(root.appendingPathComponent("move-me.txt").path, toDirectory: dest.path))

        try FileManager.default.removeItem(at: file)
        let result = try exec.undo()
        XCTAssertEqual(result.restored.first?.kind, .move)
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
        XCTAssertNotNil(try store.record(at: file.path))
        XCTAssertNil(try store.record(at: moved))
        XCTAssertTrue(try exec.undo().isEmpty)
    }

    func testProtectedPathsRefused() throws {
        let file = try write("p.txt", "x")
        let exec = Executor(store: store, protected: ProtectedPaths(prefixes: [root.path], rootFolders: []))
        XCTAssertThrowsError(try exec.trash(file.path)) { error in
            guard case ExecutorError.protected = error else { return XCTFail("expected protected, got \(error)") }
        }
        XCTAssertTrue(ProtectedPaths.default().isProtected(NSHomeDirectory() + "/Desktop"))
        XCTAssertFalse(ProtectedPaths.default().isProtected(NSHomeDirectory() + "/Desktop/some.txt"))
        XCTAssertTrue(ProtectedPaths.default().isProtected(NSHomeDirectory() + "/Library/Mobile Documents/com~apple~CloudDocs/a.txt"))
    }

    func testRenameUpdatesChildren() throws {
        _ = try write("dir/a.txt", "1")
        _ = try write("dir/sub/b.txt", "2")
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let old = root.appendingPathComponent("dir").path
        let new = root.appendingPathComponent("renamed").path
        try store.rename(from: old, to: new)
        XCTAssertNotNil(try store.record(at: new + "/sub/b.txt"))
        XCTAssertNil(try store.record(at: old + "/a.txt"))
        XCTAssertEqual(try store.children(of: new).map(\.name).sorted(), ["a.txt", "sub"])
    }

    func testExclusionLooksAtAncestors() {
        let rules = ScanRules.default(home: "/Users/me")
        XCTAssertTrue(rules.isExcluded(path: "/Users/me/.Trash/a.mp4", name: "a.mp4"))
        XCTAssertTrue(rules.isExcluded(path: "/Users/me/dev/app/node_modules/x/y.js", name: "y.js"))
        XCTAssertTrue(rules.isExcluded(path: "/Users/me/Library/Caches/foo", name: "foo"))
        XCTAssertFalse(rules.isExcluded(path: "/Users/me/Downloads/a.mp4", name: "a.mp4"))
        XCTAssertFalse(rules.isExcluded(path: "/Users/me/Desktop/한글폴더/x.pdf", name: "x.pdf"))
    }

    func testRemoveExcludedPurgesTrashRows() throws {
        let rules = ScanRules.default(home: "/Users/me")
        let gen = store.beginScan()
        let mk = { (p: String) in FileRecord(path: p, name: (p as NSString).lastPathComponent, parent: (p as NSString).deletingLastPathComponent, ext: "mp4", size: 10, modified: Date(), inode: 1, kind: .file, isDataless: false) }
        try store.upsert([mk("/Users/me/.Trash/a.mp4"), mk("/Users/me/Downloads/a.mp4"), mk("/Users/me/p/node_modules/b.mp4")], generation: gen)
        XCTAssertEqual(try store.removeExcluded(by: rules), 2)
        XCTAssertNotNil(try store.record(at: "/Users/me/Downloads/a.mp4"))
        XCTAssertNil(try store.record(at: "/Users/me/.Trash/a.mp4"))
    }

    func testKoreanPathsAreNormalizedToNFC() throws {
        let nfd = "한글폴더".decomposedStringWithCanonicalMapping
        let dir = root.appendingPathComponent(nfd, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        _ = try write(nfd + "/자료.pdf", "x")
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let nfc = root.appendingPathComponent("한글폴더").path
        XCTAssertEqual(try store.children(of: nfc).count, 1)
        XCTAssertEqual(try store.children(of: root.appendingPathComponent(nfd).path).count, 1)
        XCTAssertEqual(try store.summary(under: nfc).files, 1)
    }

    func testLeftoverRulesExcludeSystemAndLibraryNames() {
        // macOS 단축어(옛 이름 Workflow) — 앱 번들 ID는 com.apple.shortcuts라 지운 앱처럼 보이지만 시스템 데이터
        XCTAssertTrue(LeftoverFinder.isExcludedIdentifier("group.is.workflow.my.app"))
        XCTAssertTrue(LeftoverFinder.isExcludedIdentifier("is.workflow.my.app"))
        XCTAssertTrue(LeftoverFinder.isExcludedIdentifier("org.cups.PrintingPrefs"))      // 프린터 설정
        XCTAssertTrue(LeftoverFinder.isExcludedIdentifier("com.apple.Safari"))
        XCTAssertTrue(LeftoverFinder.isExcludedIdentifier("com.segment.storage.oai"))     // 라이브러리(SDK) 저장소
        XCTAssertTrue(LeftoverFinder.isExcludedIdentifier("com.onevcat.Kingfisher.ImageCache.default"))
        XCTAssertTrue(LeftoverFinder.isExcludedIdentifier("com.plausiblelabs.crashreporter.data"))
        XCTAssertFalse(LeftoverFinder.isExcludedIdentifier("com.khanov.BlockerX"))
        XCTAssertFalse(LeftoverFinder.isExcludedIdentifier("dev.warp.Warp-Stable"))
        XCTAssertTrue(LeftoverFinder.looksLikeBundleID("com.todesktop.230313mzl4w4u92"))
        XCTAssertFalse(LeftoverFinder.looksLikeBundleID("TorBrowser-Data"))
    }

    func testUndoSkipsOperationsAlreadyRestoredOutsideTheApp() throws {
        let a = try write("a.txt", "1")
        let b = try write("b.txt", "2")
        let dest = root.appendingPathComponent("into", isDirectory: true)
        try FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let exec = Executor(store: store, protected: ProtectedPaths(prefixes: [], rootFolders: []))
        let movedA = try exec.move(a.path, toDirectory: dest.path)
        let movedB = try exec.move(b.path, toDirectory: dest.path)
        // 사용자가 b를 직접 원래 자리로 되돌려 놓은 상황
        try FileManager.default.moveItem(atPath: movedB, toPath: b.path)
        // ⌘Z는 b 기록을 건너뛰고 a를 되돌린다
        let result = try exec.undo()
        XCTAssertEqual(result.restored.first?.source, ScanRules.normalize(a.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: a.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: movedA))
        XCTAssertTrue(try exec.undo().isEmpty)
    }

    // MARK: - 배포 전 점검에서 찾은 문제의 회귀 테스트

    /// 한글 이름 폴더를 옮기면 안쪽 항목 경로가 깨지던 문제 (SQLite substr은 바이트가 아닌 글자 단위)
    func testMovingKoreanFolderKeepsChildPaths() throws {
        _ = try write("사진/a.jpg", "1")
        _ = try write("사진/2024/b.jpg", "2")
        let dest = root.appendingPathComponent("보관", isDirectory: true)
        try FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let exec = Executor(store: store, protected: ProtectedPaths(prefixes: [], rootFolders: []))
        let moved = try exec.move(root.appendingPathComponent("사진").path, toDirectory: dest.path)
        XCTAssertNotNil(try store.record(at: moved + "/a.jpg"))
        XCTAssertNotNil(try store.record(at: moved + "/2024/b.jpg"))
        XCTAssertEqual(try store.children(of: moved + "/2024").map(\.name), ["b.jpg"])
        XCTAssertEqual(try store.files(under: moved).count, 2)
        XCTAssertEqual(try exec.undo().restored.count, 1)
        let back = ScanRules.normalize(root.appendingPathComponent("사진").path)
        XCTAssertNotNil(try store.record(at: back + "/2024/b.jpg"))
        XCTAssertEqual(try store.files(under: back).count, 2)
        XCTAssertEqual(try store.files(under: moved).count, 0)
    }

    /// 여러 파일을 한 번에 옮긴 작업은 ⌘Z 한 번에 전부 돌아오고, 옮기며 만든 빈 폴더도 치운다
    func testBatchUndoRestoresAllAtOnceAndRemovesCreatedFolders() throws {
        let files = try ["a.txt", "b.txt", "c.txt"].map { try write($0, $0) }
        let target = root.appendingPathComponent("new/deeper").path
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let exec = Executor(store: store, protected: ProtectedPaths(prefixes: [], rootFolders: []))
        exec.batch {
            for f in files { _ = try? exec.move(f.path, toDirectory: target, createDirectory: true) }
        }
        XCTAssertTrue(FileManager.default.fileExists(atPath: target + "/c.txt"))
        let result = try exec.undo()
        XCTAssertEqual(result.restored.count, 3)
        for f in files { XCTAssertTrue(FileManager.default.fileExists(atPath: f.path)) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent("new").path))
        XCTAssertTrue(try exec.undo().isEmpty)
    }

    /// 옮긴 자리에 같은 이름의 '다른' 파일이 생겼으면 그 파일을 끌어오지 않는다
    func testUndoDoesNotMoveADifferentItemThatTookThePlace() throws {
        let a = try write("a.txt", "original")
        let dest = root.appendingPathComponent("into", isDirectory: true)
        try FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        let exec = Executor(store: store, protected: ProtectedPaths(prefixes: [], rootFolders: []))
        let moved = try exec.move(a.path, toDirectory: dest.path)
        try FileManager.default.removeItem(atPath: moved)
        try "someone else".data(using: .utf8)!.write(to: URL(fileURLWithPath: moved))
        let result = try exec.undo()
        XCTAssertTrue(result.restored.isEmpty)
        XCTAssertEqual(result.missing.count, 1)
        XCTAssertTrue(FileManager.default.fileExists(atPath: moved))
        XCTAssertFalse(FileManager.default.fileExists(atPath: a.path))
    }

    /// 앱을 켜기 전 작업은 ⌘Z로 되돌리지 않는다
    func testSessionScopedUndoLeavesOlderOperations() throws {
        let a = try write("a.txt", "1")
        let b = try write("b.txt", "2")
        let dest = root.appendingPathComponent("into", isDirectory: true)
        try FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        let exec = Executor(store: store, protected: ProtectedPaths(prefixes: [], rootFolders: []))
        _ = try exec.move(a.path, toDirectory: dest.path)
        let sessionStart = try store.maxOperationID()
        _ = try exec.move(b.path, toDirectory: dest.path)
        XCTAssertEqual(try exec.undo(after: sessionStart).restored.count, 1)
        XCTAssertTrue(FileManager.default.fileExists(atPath: b.path))
        XCTAssertTrue(try exec.undo(after: sessionStart).isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: a.path))
    }

    /// 인덱스가 낡았을 때(가운데만 바뀐 큰 파일) 저장된 지문을 믿고 '같다'고 하지 않는다
    func testDuplicateFinderRechecksFilesChangedSinceScan() throws {
        var data = Data(repeating: 0x41, count: 600_000)
        let one = root.appendingPathComponent("one.bin"), two = root.appendingPathComponent("two.bin")
        try data.write(to: one); try data.write(to: two)
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        XCTAssertEqual(try DuplicateFinder(store: store).find().count, 1)
        data[300_000] = 0x42   // 앞·뒤 128KB 표본에는 안 걸리는 자리
        try data.write(to: one)
        try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(120)], ofItemAtPath: one.path)
        XCTAssertEqual(try DuplicateFinder(store: store).find().count, 0)
        try FileManager.default.removeItem(at: one)
        XCTAssertEqual(try DuplicateFinder(store: store).find().count, 0)
    }

    /// 비교 도중 사라진 파일 때문에 비교 전체가 실패하지 않고, 읽을 수 없는 파일은 '같다'고 하지 않는다
    func testFolderDiffToleratesFilesDeletedAfterScan() throws {
        _ = try write("L/x.txt", "same")
        _ = try write("R/x.txt", "same")
        let gone = try write("L/gone.txt", "abc")
        _ = try write("R/other.txt", "abc")
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        try FileManager.default.removeItem(at: gone)
        let r = try FolderDiff(store: store).compare(left: root.appendingPathComponent("L").path, right: root.appendingPathComponent("R").path)
        XCTAssertEqual(r.count(.identical), 1)
        XCTAssertEqual(r.count(.relocated), 0)
    }

    /// 검색은 모은 폴더 아래 깊은 곳까지 — 폴더 먼저
    func testSearchFindsNestedItems() throws {
        _ = try write("a/b/c/report-2024.pdf", "x")
        _ = try write("a/notes.txt", "y")
        _ = try write("Report folder/z.txt", "z")
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let found = try store.search(name: "report", under: [root.path])
        XCTAssertEqual(Set(found.map(\.name)), ["report-2024.pdf", "Report folder"])
        XCTAssertEqual(found.first?.name, "Report folder")
        XCTAssertTrue(try store.search(name: "100%", under: [root.path]).isEmpty)
    }

    /// 경로 범위 조회는 그 폴더 아래만 — 이름이 비슷한 형제 폴더는 섞이지 않는다
    func testDescendantRangeOnlyCoversChildren() throws {
        let gen = store.beginScan()
        let mk = { (p: String) in FileRecord(path: p, name: (p as NSString).lastPathComponent, parent: (p as NSString).deletingLastPathComponent, ext: "", size: 1, modified: Date(), inode: 0, kind: .file, isDataless: false) }
        try store.upsert(["/t/b/c", "/t/b c", "/t/bc", "/t/b0", "/t/b-x", "/t/b/d/e", "/t/b.x"].map(mk), generation: gen)
        XCTAssertEqual(Set(try store.files(under: "/t/b").map(\.path)), ["/t/b/c", "/t/b/d/e"])
        XCTAssertEqual(try store.summary(under: "/t/b").files, 2)
        try store.remove(path: "/t/b")
        XCTAssertEqual(try store.files(under: "/t").count, 5)
    }

    /// 아직 살아 있는 프로그램을 가리키는 로그인 항목은 잔여물이 아니다
    func testLaunchAgentWithLiveProgramIsNotALeftover() throws {
        let live = root.appendingPathComponent("live.plist")
        try (["Label": "x", "ProgramArguments": ["/bin/ls", "-l"]] as NSDictionary).write(to: live)
        let dead = root.appendingPathComponent("dead.plist")
        try (["Label": "y", "Program": "/nonexistent/tool"] as NSDictionary).write(to: dead)
        XCTAssertTrue(LeftoverFinder.launchAgentIsLive(live.path))
        XCTAssertFalse(LeftoverFinder.launchAgentIsLive(dead.path))
        XCTAssertFalse(LeftoverFinder.strongAreas.contains("LaunchAgents"))
    }

    func testIsUnchangedDetectsEditsAndDeletion() throws {
        let f = try write("e.txt", "abc")
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let rec = try XCTUnwrap(store.record(at: f.path))
        XCTAssertTrue(Scanner.isUnchanged(rec))
        try "abcd".data(using: .utf8)!.write(to: f)
        XCTAssertFalse(Scanner.isUnchanged(rec))
        try FileManager.default.removeItem(at: f)
        XCTAssertFalse(Scanner.isUnchanged(rec))
    }

    /// Finder에서 폴더째 옮겨 온 경우 폴더 이벤트 하나만 온다 → 안쪽까지 인덱스에 들어가야 한다
    func testWatcherUpdaterScansFoldersMovedIn() throws {
        try Scanner(rules: openRules).scan(roots: [root], into: store)
        let outside = base.appendingPathComponent("outside/pack", isDirectory: true)
        try FileManager.default.createDirectory(at: outside.appendingPathComponent("inner"), withIntermediateDirectories: true)
        try "x".data(using: .utf8)!.write(to: outside.appendingPathComponent("inner/f.txt"))
        let dest = root.appendingPathComponent("pack")
        try FileManager.default.moveItem(at: outside, to: dest)
        Watcher.indexUpdater(store: store, rules: openRules)([dest.path])
        XCTAssertNotNil(try store.record(at: dest.path + "/inner/f.txt"))
    }

    /// 화면 언어와 상관없이 '기타' 묶음을 알아본다 (언어를 바꿔도 특별 취급 유지)
    func testFallbackBucketRecognizedInAnyLanguage() {
        for name in ["기타", "Other", "その他", "其他"] { XCTAssertTrue(Bucket(name: name, hint: "").isFallback) }
        XCTAssertFalse(Bucket(name: "업무", hint: "").isFallback)
        XCTAssertEqual([Bucket(name: "Work", hint: ""), Bucket(name: "Other", hint: "")].fallbackName, "Other")
    }
}
