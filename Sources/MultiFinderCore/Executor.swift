import Foundation

public enum ExecutorError: LocalizedError {
    case notFound(String)
    case protected(String)
    case destinationExists(String)
    case notADirectory(String)
    case undoSourceOccupied(String)
    /// 되돌릴 항목이 더 이상 없음 (휴지통을 비웠거나 사용자가 직접 옮김)
    case undoItemGone(String)
    /// 휴지통에서 꺼낼 권한이 없음 — macOS는 휴지통을 보호하므로 전체 디스크 접근이 있어야 한다
    case undoNeedsTrashAccess(trashed: String, original: String)

    public var errorDescription: String? {
        switch self {
        case .notFound(let p): return String(localized: "없는 경로: \(p)")
        case .protected(let p): return String(localized: "보호된 경로라 변경하지 않음: \(p)")
        case .destinationExists(let p): return String(localized: "같은 이름이 이미 있어 덮어쓰지 않음: \(p)")
        case .notADirectory(let p): return String(localized: "폴더가 아님: \(p)")
        case .undoSourceOccupied(let p): return String(localized: "되돌릴 자리에 다른 파일이 있음: \(p)")
        case .undoItemGone(let p):
            let name = (p as NSString).lastPathComponent
            return String(localized: "되돌릴 항목이 더 이상 없습니다(휴지통을 비웠거나 직접 옮김): \(name)")
        case .undoNeedsTrashAccess(_, let original):
            let name = (original as NSString).lastPathComponent
            return String(localized: "‘\(name)’을(를) 휴지통에서 꺼낼 권한이 없습니다.\n\nFinder 휴지통에서 이 항목을 오른쪽 클릭 › ‘되돌려 놓기’를 하면 원래 자리로 돌아갑니다. Multi-Finder에 전체 디스크 접근을 허용하면 ⌘Z로도 되돌릴 수 있습니다.")
        }
    }
}

/// 되돌리기 한 번의 결과.
public struct UndoResult: Sendable {
    /// 원래 자리로 돌아간 작업 (최근 것부터)
    public var restored: [Operation] = []
    /// 되돌릴 항목이 사라졌거나(휴지통 비움 등) 다른 항목으로 바뀌어 건너뛴 원래 경로
    public var missing: [String] = []
    /// 원래 자리에 이미 다른 항목이 있어 되돌리지 못한 경로 (기록은 남겨 둔다 — 자리를 비우고 다시 시도 가능)
    public var blocked: [String] = []
    /// 휴지통에서 돌아온 폴더 — 안쪽은 호출자가 다시 스캔한다
    public var rescan: [String] = []
    public var isEmpty: Bool { restored.isEmpty && missing.isEmpty && blocked.isEmpty }
}

/// 파일을 실제로 바꾸는 유일한 경로. 규칙:
/// 1. 삭제는 없다 — 휴지통으로 옮긴다.
/// 2. 절대 덮어쓰지 않는다.
/// 3. 보호 경로는 거부한다.
/// 4. 모든 변경은 로그에 남고 되돌릴 수 있다. 한 번의 사용자 동작(여러 파일)은 한 묶음으로 되돌린다.
public final class Executor {
    private let store: IndexStore
    private let protected: ProtectedPaths
    private let fm = FileManager.default
    /// 진행 중인 묶음 번호 (batch { } 안에서만)
    private var currentBatch: Int64?

    public init(store: IndexStore, protected: ProtectedPaths = .default()) {
        self.store = store
        self.protected = protected
    }

    /// 안에서 한 작업을 한 묶음으로 기록한다 — 되돌리기 한 번에 함께 돌아간다.
    public func batch<T>(_ body: () throws -> T) rethrows -> T {
        if currentBatch != nil { return try body() }
        currentBatch = try? store.nextBatchID()
        defer { currentBatch = nil }
        return try body()
    }

    /// 휴지통으로. 반환값은 휴지통 안의 실제 위치.
    @discardableResult
    public func trash(_ path: String) throws -> URL {
        let p = ScanRules.normalize(path)
        try precheck(p)
        var trashed: NSURL?
        try fm.trashItem(at: URL(fileURLWithPath: p), resultingItemURL: &trashed)
        let dest = (trashed as URL?)?.path ?? ""
        // 기록이 먼저 — 인덱스 갱신이 실패해도 되돌리기는 가능해야 한다 (인덱스는 감시·재스캔이 바로잡는다)
        try store.logOperation(kind: .trash, source: p, destination: dest, batch: currentBatch, inode: Self.inode(dest))
        try? store.remove(path: p)
        return URL(fileURLWithPath: dest)
    }

    /// `path`를 `directory` 안으로 이동 (이름 유지). 반환값은 새 경로.
    /// - Parameter createDirectory: 대상 폴더가 없으면 만든다 (폴더 비교에서 상대 경로를 유지한 채 몰 때). 만든 폴더도 기록해 되돌릴 때 치운다.
    @discardableResult
    public func move(_ path: String, toDirectory directory: String, createDirectory: Bool = false) throws -> String {
        let p = ScanRules.normalize(path)
        let dir = ScanRules.normalize(directory)
        try precheck(p)
        if createDirectory && !fm.fileExists(atPath: dir) {
            // 없는 가장 위 폴더를 기록해 두면, 되돌릴 때 그 아래가 비어 있을 경우 통째로 치울 수 있다
            var top = dir
            while true {
                let parent = (top as NSString).deletingLastPathComponent
                if parent == top || fm.fileExists(atPath: parent) { break }
                top = parent
            }
            try fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
            try store.logOperation(kind: .mkdir, source: top, destination: "", batch: currentBatch, inode: Self.inode(top))
        }
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: dir, isDirectory: &isDir), isDir.boolValue else {
            throw ExecutorError.notADirectory(dir)
        }
        let dest = (dir as NSString).appendingPathComponent((p as NSString).lastPathComponent)
        guard !fm.fileExists(atPath: dest) else { throw ExecutorError.destinationExists(dest) }
        try fm.moveItem(atPath: p, toPath: dest)
        do {
            try store.logOperation(kind: .move, source: p, destination: dest, batch: currentBatch, inode: Self.inode(dest))
        } catch {
            // 기록하지 못한 이동은 되돌릴 수 없으므로 원위치하고 실패로 알린다
            try? fm.moveItem(atPath: dest, toPath: p)
            throw error
        }
        try? store.rename(from: p, to: dest)
        return dest
    }

    /// 마지막 작업 묶음을 되돌린다. `after`보다 뒤의 기록만 대상 (앱을 켠 뒤 한 작업만 되돌리게).
    /// 사용자가 이미 Finder에서 '되돌려 놓기'를 한 묶음은 조용히 건너뛰고 그 앞 묶음을 되돌린다.
    @discardableResult
    public func undo(after minID: Int64 = 0) throws -> UndoResult {
        while true {
            let ops = try store.lastUndoableBatch(after: minID)
            guard !ops.isEmpty else { return UndoResult() }
            var result = UndoResult()
            for op in ops {   // 최근 것부터
                if op.kind == .mkdir {
                    // 우리가 만든 그 폴더일 때만, 그리고 안이 비어 있을 때만 치운다
                    if op.inode == 0 || Self.inode(op.source) == op.inode { Self.removeIfEmptyTree(op.source) }
                    try store.markUndone(id: op.id)
                    continue
                }
                let destinationExists = fm.fileExists(atPath: op.destination)
                let sourceExists = fm.fileExists(atPath: op.source)
                // 그 자리에 있는 게 옮겼던 바로 그 항목인가 (같은 이름의 다른 파일을 옮기지 않도록)
                let sameItem = op.inode == 0 || Self.inode(op.destination) == op.inode
                if !destinationExists || !sameItem {
                    try store.markUndone(id: op.id)          // 이 기록은 더 이상 되돌릴 대상이 아님
                    if !sourceExists { result.missing.append(op.source) }
                    continue                                 // 이미 원래 자리에 있으면(Finder에서 되돌려 놓음) 조용히 넘어간다
                }
                if sourceExists { result.blocked.append(op.source); continue }
                if let rescan = try restore(op) { result.rescan.append(rescan) }
                result.restored.append(op)
            }
            if !result.isEmpty { return result }
            // 묶음 전체가 이미 밖에서 되돌려졌다 → 그 앞 묶음으로
        }
    }

    /// 한 건을 원래 자리로. 휴지통에서 돌아온 폴더면 다시 스캔할 경로를 돌려준다.
    private func restore(_ op: Operation) throws -> String? {
        try fm.createDirectory(atPath: (op.source as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        do {
            try fm.moveItem(atPath: op.destination, toPath: op.source)
        } catch where op.kind == .trash && Self.isPermissionError(error) {
            throw ExecutorError.undoNeedsTrashAccess(trashed: op.destination, original: op.source)
        }
        try store.markUndone(id: op.id)
        // 인덱스 복원: 이동이면 경로만 바꾸고, 휴지통에서 돌아온 항목은 행을 다시 만든다 (폴더 안쪽은 호출자가 스캔)
        switch op.kind {
        case .move:
            try? store.rename(from: op.destination, to: op.source)
            return nil
        case .trash, .mkdir:
            guard let rec = Scanner.record(for: URL(fileURLWithPath: op.source)) else { return nil }
            try? store.upsert([rec], generation: store.currentGeneration)
            return rec.kind == .directory ? op.source : nil
        }
    }

    public func history(limit: Int = 50) throws -> [Operation] {
        try store.history(limit: limit)
    }

    /// 경로의 inode (링크는 따라가지 않음). 없으면 0.
    static func inode(_ path: String) -> UInt64 {
        var st = stat()
        return lstat(path, &st) == 0 ? UInt64(st.st_ino) : 0
    }

    /// 이동하며 만든 폴더를 되돌릴 때: 안에 폴더와 .DS_Store밖에 없으면 치운다. 파일이 하나라도 있으면 그대로 둔다.
    static func removeIfEmptyTree(_ path: String) {
        let fm = FileManager.default
        guard let entries = try? fm.contentsOfDirectory(atPath: path) else { return }
        for e in entries where e != ".DS_Store" {
            let child = (path as NSString).appendingPathComponent(e)
            var isDir: ObjCBool = false
            guard fm.fileExists(atPath: child, isDirectory: &isDir), isDir.boolValue else { return }
            removeIfEmptyTree(child)
            if fm.fileExists(atPath: child) { return }
        }
        try? fm.removeItem(atPath: path)
    }

    /// 권한 문제로 실패했는가 (macOS 개인정보 보호·앱 데이터 보호 포함)
    public static func isPermissionError(_ error: Error) -> Bool {
        let ns = error as NSError
        if ns.domain == NSCocoaErrorDomain && (ns.code == NSFileWriteNoPermissionError || ns.code == NSFileReadNoPermissionError) { return true }
        if ns.domain == NSPOSIXErrorDomain && (ns.code == Int(EPERM) || ns.code == Int(EACCES)) { return true }
        if let underlying = ns.userInfo[NSUnderlyingErrorKey] as? Error { return isPermissionError(underlying) }
        return false
    }

    /// 이 앱에 전체 디스크 접근이 있는가. 휴지통 목록을 읽을 수 있는지로 판단한다
    /// (macOS는 휴지통을 보호해서 전체 디스크 접근 없이는 목록을 읽지도, 꺼내지도 못한다).
    public static var hasFullDiskAccess: Bool {
        (try? FileManager.default.contentsOfDirectory(atPath: NSHomeDirectory() + "/.Trash")) != nil
    }

    private func precheck(_ p: String) throws {
        guard fm.fileExists(atPath: p) else { throw ExecutorError.notFound(p) }
        guard !protected.isProtected(p) else { throw ExecutorError.protected(p) }
    }
}
