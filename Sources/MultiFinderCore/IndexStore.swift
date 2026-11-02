import Foundation
import SQLite3

public enum IndexError: LocalizedError {
    case sqlite(String)
    public var errorDescription: String? {
        switch self { case .sqlite(let m): return "SQLite: \(m)" }
    }
}

/// 실행 기록 한 줄. 되돌리기의 근거.
public struct Operation: Sendable, Identifiable {
    /// mkdir: 이동하면서 새로 만든 폴더 (되돌릴 때 비어 있으면 치운다)
    public enum Kind: String, Sendable { case trash, move, mkdir }
    public let id: Int64
    public let timestamp: Date
    public let kind: Kind
    /// 원래 있던 자리 (mkdir이면 만든 폴더)
    public let source: String
    /// 옮겨진 자리 (휴지통 안 경로 포함)
    public let destination: String
    public let undone: Bool
    /// 한 번의 사용자 동작으로 함께 한 작업들 — 되돌리기는 묶음 단위
    public let batch: Int64?
    /// 옮긴 뒤 그 자리 항목의 inode — 되돌릴 때 같은 항목인지 확인 (0이면 모름)
    public let inode: UInt64
}

private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

/// 파일 인덱스 + 작업 로그 저장소. 외부 의존 없이 시스템 SQLite만 쓴다.
/// 스레드 안전: 모든 접근을 내부 직렬 큐로 보낸다.
public final class IndexStore: @unchecked Sendable {
    private var db: OpaquePointer?
    private let queue = DispatchQueue(label: "multifinder.indexstore")
    private var generation: Int64 = 0

    public static func defaultURL() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("MultiFinder", isDirectory: true).appendingPathComponent("index.sqlite")
    }

    public init(path: String) throws {
        try FileManager.default.createDirectory(
            at: URL(fileURLWithPath: path).deletingLastPathComponent(),
            withIntermediateDirectories: true)
        var handle: OpaquePointer?
        guard sqlite3_open_v2(path, &handle, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK,
              let handle else {
            throw IndexError.sqlite(String(cString: sqlite3_errmsg(handle)))
        }
        db = handle
        // 다른 프로세스(mfinder, 두 번째 창)가 쓰는 중이면 잠깐 기다린다 — 바로 실패하면 작업 기록이 빠질 수 있다
        sqlite3_busy_timeout(handle, 5000)
        try exec("""
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        PRAGMA temp_store=MEMORY;
        CREATE TABLE IF NOT EXISTS files(
            path TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            parent TEXT NOT NULL,
            ext TEXT NOT NULL,
            size INTEGER NOT NULL,
            mtime REAL NOT NULL,
            inode INTEGER NOT NULL,
            kind INTEGER NOT NULL,
            dataless INTEGER NOT NULL,
            hash TEXT,
            hash_mtime REAL,
            hash_size INTEGER,
            seen INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS files_size ON files(size) WHERE kind = 0;
        CREATE INDEX IF NOT EXISTS files_hash ON files(hash);
        CREATE INDEX IF NOT EXISTS files_parent ON files(parent);
        CREATE TABLE IF NOT EXISTS ops(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts REAL NOT NULL,
            kind TEXT NOT NULL,
            src TEXT NOT NULL,
            dst TEXT NOT NULL,
            undone INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
        """)
        // 경로 표기 규칙(NFC 통일 등)이 바뀌면 옛 인덱스는 버리고 다시 쌓는다 — 스캔은 싸고, 섞인 표기는 비싸다
        let schema = "2"
        let st = try Statement(db: db!, sql: "SELECT value FROM meta WHERE key = 'schema'")
        if !(st.step() && st.text(0) == schema) {
            try exec("DELETE FROM files; INSERT OR REPLACE INTO meta(key, value) VALUES('schema', '\(schema)')")
        }
        generation = Int64(try scalar("SELECT COALESCE(MAX(seen), 0) FROM files"))
        // 작업 기록: 묶음(한 번에 한 작업)과 inode 열 추가 — 옛 기록은 NULL/0
        let opColumns = try Statement(db: db!, sql: "PRAGMA table_info(ops)")
        var have = Set<String>()
        while opColumns.step() { if let n = opColumns.text(1) { have.insert(n) } }
        if !have.contains("batch") { try exec("ALTER TABLE ops ADD COLUMN batch INTEGER") }
        if !have.contains("inode") { try exec("ALTER TABLE ops ADD COLUMN inode INTEGER NOT NULL DEFAULT 0") }
    }

    deinit { if let db { sqlite3_close(db) } }

    // MARK: - 저수준

    /// 확장(분류 등)에서 쓰는 공개 실행기. 호출자가 큐 안에 있지 않을 때만 쓴다.
    func execPublic(_ sql: String) throws { try sync { try exec(sql) } }

    /// 준비문 하나를 큐 안에서 만들어 쓰고 버린다.
    func withStatement<T>(_ sql: String, _ body: (Statement) throws -> T) throws -> T {
        try sync { try body(try Statement(db: db!, sql: sql)) }
    }

    /// 트랜잭션 안(이미 큐 안)에서 준비문 만들기.
    func statement(_ sql: String) throws -> Statement { try Statement(db: db!, sql: sql) }

    /// 트랜잭션 안(이미 큐 안)에서 SQL 실행.
    func execInTransaction(_ sql: String) throws { try exec(sql) }

    static var columnsPublic: String { columns }
    static func readPublic(_ st: Statement) -> FileRecord {
        FileRecord(
            path: st.text(0) ?? "", name: st.text(1) ?? "", parent: st.text(2) ?? "", ext: st.text(3) ?? "",
            size: st.int64(4), modified: Date(timeIntervalSince1970: st.double(5)),
            inode: UInt64(bitPattern: st.int64(6)), kind: FileKind(rawValue: Int(st.int64(7))) ?? .file,
            isDataless: st.int64(8) != 0, hash: st.text(9))
    }

    private func exec(_ sql: String) throws {
        var err: UnsafeMutablePointer<CChar>?
        if sqlite3_exec(db, sql, nil, nil, &err) != SQLITE_OK {
            let msg = err.map { String(cString: $0) } ?? "unknown"
            sqlite3_free(err)
            throw IndexError.sqlite(msg)
        }
    }

    private func scalar(_ sql: String) throws -> Int64 {
        let st = try Statement(db: db!, sql: sql)
        return st.step() ? st.int64(0) : 0
    }

    private func sync<T>(_ body: () throws -> T) throws -> T {
        try queue.sync { try body() }
    }

    /// 여러 쓰기를 한 트랜잭션으로 묶는다. 스캔 배치 삽입에 필수(수십 배 빠름).
    public func transaction(_ body: () throws -> Void) throws {
        try sync {
            try exec("BEGIN IMMEDIATE")
            do { try body(); try exec("COMMIT") } catch { try? exec("ROLLBACK"); throw error }
        }
    }

    // MARK: - 스캔 세대

    public var currentGeneration: Int64 { queue.sync { generation } }

    /// 새 전체 스캔 시작. 반환값을 upsert에 넘기고, 끝나면 finishScan으로 사라진 파일을 지운다.
    public func beginScan() -> Int64 {
        queue.sync { generation += 1; return generation }
    }

    /// `roots` 아래에서 이번 세대에 보이지 않은 행 삭제 (스캔 중 지워진 파일 반영).
    public func finishScan(roots: [String], generation gen: Int64) throws {
        try sync {
            let st = try Statement(db: db!, sql: "DELETE FROM files WHERE seen < ? AND (path = ? OR (path >= ? AND path < ?))")
            for root in roots {
                let r = Self.descendantRange(root)
                st.reset()
                st.bind(1, gen)
                st.bind(2, root)
                st.bind(3, r.lower); st.bind(4, r.upper)
                try st.run()
            }
        }
    }

    // MARK: - 쓰기

    private static let upsertSQL = """
    INSERT INTO files(path,name,parent,ext,size,mtime,inode,kind,dataless,hash,hash_mtime,hash_size,seen)
    VALUES(?,?,?,?,?,?,?,?,?,NULL,NULL,NULL,?)
    ON CONFLICT(path) DO UPDATE SET
        name=excluded.name, parent=excluded.parent, ext=excluded.ext, size=excluded.size,
        mtime=excluded.mtime, inode=excluded.inode, kind=excluded.kind, dataless=excluded.dataless,
        seen=excluded.seen,
        hash      = CASE WHEN files.hash_mtime = excluded.mtime AND files.hash_size = excluded.size THEN files.hash ELSE NULL END,
        hash_mtime= CASE WHEN files.hash_mtime = excluded.mtime AND files.hash_size = excluded.size THEN files.hash_mtime ELSE NULL END,
        hash_size = CASE WHEN files.hash_mtime = excluded.mtime AND files.hash_size = excluded.size THEN files.hash_size ELSE NULL END
    """

    /// 배치 삽입/갱신. 내용(크기·수정일)이 안 바뀐 파일의 지문은 유지한다.
    public func upsert(_ records: [FileRecord], generation gen: Int64) throws {
        guard !records.isEmpty else { return }
        try transaction {
            let st = try Statement(db: db!, sql: Self.upsertSQL)
            for r in records {
                st.reset()
                st.bind(1, r.path); st.bind(2, r.name); st.bind(3, r.parent); st.bind(4, r.ext)
                st.bind(5, r.size); st.bind(6, r.modified.timeIntervalSince1970)
                st.bind(7, Int64(bitPattern: r.inode)); st.bind(8, Int64(r.kind.rawValue))
                st.bind(9, Int64(r.isDataless ? 1 : 0)); st.bind(10, gen)
                try st.run()
            }
        }
    }

    public func setHash(path: String, hash: String, modified: Date, size: Int64) throws {
        try sync {
            let st = try Statement(db: db!, sql: "UPDATE files SET hash=?, hash_mtime=?, hash_size=? WHERE path=?")
            st.bind(1, hash); st.bind(2, modified.timeIntervalSince1970); st.bind(3, size); st.bind(4, path)
            try st.run()
        }
    }

    /// 경로와 그 아래 전부 삭제.
    public func remove(path rawPath: String) throws {
        let path = ScanRules.normalize(rawPath)
        try sync {
            let r = Self.descendantRange(path)
            let st = try Statement(db: db!, sql: "DELETE FROM files WHERE path = ? OR (path >= ? AND path < ?)")
            st.bind(1, path); st.bind(2, r.lower); st.bind(3, r.upper)
            try st.run()
            if let cls = try? Statement(db: db!, sql: "DELETE FROM classes WHERE path = ? OR (path >= ? AND path < ?)") {
                cls.bind(1, path); cls.bind(2, r.lower); cls.bind(3, r.upper); try cls.run()
            }
        }
    }

    /// 이동 반영: 경로 접두어를 바꾼다 (폴더면 자식까지).
    public func rename(from rawOld: String, to rawNew: String) throws {
        let old = ScanRules.normalize(rawOld), new = ScanRules.normalize(rawNew)
        try transaction {
            // 새 자리에 남아 있던 행은 낡은 것 (실제로 비어 있어야 옮길 수 있었다) — 먼저 치워 경로 충돌을 막는다
            let target = Self.descendantRange(new)
            let clear = try Statement(db: db!, sql: "DELETE FROM files WHERE path = ? OR (path >= ? AND path < ?)")
            clear.bind(1, new); clear.bind(2, target.lower); clear.bind(3, target.upper)
            try clear.run()
            let newURL = URL(fileURLWithPath: new)
            let st = try Statement(db: db!, sql: "UPDATE files SET path=?, name=?, parent=?, ext=? WHERE path=?")
            st.bind(1, new); st.bind(2, newURL.lastPathComponent)
            st.bind(3, newURL.deletingLastPathComponent().path)
            st.bind(4, newURL.pathExtension.lowercased()); st.bind(5, old)
            try st.run()
            // SQLite substr는 바이트가 아니라 글자(코드 포인트) 단위 — 한글 폴더 이름에서 어긋나지 않도록 글자 수로 자른다
            let cut = Int64(old.unicodeScalars.count + 1)
            let r = Self.descendantRange(old)
            let children = try Statement(db: db!, sql: "UPDATE files SET path = ? || substr(path, ?), parent = ? || substr(parent, ?) WHERE path >= ? AND path < ?")
            children.bind(1, new); children.bind(2, cut)
            children.bind(3, new); children.bind(4, cut)
            children.bind(5, r.lower); children.bind(6, r.upper)
            try children.run()
            // 분류 결과도 따라간다 (테이블이 아직 없으면 무시)
            if let cls = try? Statement(db: db!, sql: "UPDATE OR REPLACE classes SET path = ? || substr(path, ?) WHERE path = ? OR (path >= ? AND path < ?)") {
                cls.bind(1, new); cls.bind(2, cut); cls.bind(3, old); cls.bind(4, r.lower); cls.bind(5, r.upper)
                try cls.run()
            }
        }
    }

    // MARK: - 읽기

    private static let columns = "path,name,parent,ext,size,mtime,inode,kind,dataless,hash"

    private func read(_ st: Statement) -> FileRecord {
        FileRecord(
            path: st.text(0) ?? "", name: st.text(1) ?? "", parent: st.text(2) ?? "", ext: st.text(3) ?? "",
            size: st.int64(4), modified: Date(timeIntervalSince1970: st.double(5)),
            inode: UInt64(bitPattern: st.int64(6)), kind: FileKind(rawValue: Int(st.int64(7))) ?? .file,
            isDataless: st.int64(8) != 0, hash: st.text(9))
    }

    public func record(at path: String) throws -> FileRecord? {
        try sync {
            let st = try Statement(db: db!, sql: "SELECT \(Self.columns) FROM files WHERE path = ?")
            st.bind(1, ScanRules.normalize(path))
            return st.step() ? read(st) : nil
        }
    }

    public func children(of parent: String) throws -> [FileRecord] {
        try sync {
            let st = try Statement(db: db!, sql: "SELECT \(Self.columns) FROM files WHERE parent = ? ORDER BY kind DESC, name COLLATE NOCASE")
            st.bind(1, ScanRules.normalize(parent))
            var out: [FileRecord] = []
            while st.step() { out.append(read(st)) }
            return out
        }
    }

    /// 폴더 아래(재귀) 일반 파일 전부. 경로순.
    public func files(under rawPrefix: String) throws -> [FileRecord] {
        let prefix = ScanRules.normalize(rawPrefix)
        return try sync {
            let r = Self.descendantRange(prefix)
            let st = try Statement(db: db!, sql: "SELECT \(Self.columns) FROM files WHERE kind = 0 AND path >= ? AND path < ? ORDER BY path")
            st.bind(1, r.lower); st.bind(2, r.upper)
            var out: [FileRecord] = []
            while st.step() { out.append(read(st)) }
            return out
        }
    }

    /// 규칙상 제외돼야 하는데 이전 버전/감시 경로로 들어온 행을 정리한다 (휴지통, node_modules, 숨김 폴더 등).
    @discardableResult
    public func removeExcluded(by rules: ScanRules) throws -> Int {
        try sync {
            var removed = 0
            let st = try Statement(db: db!, sql: "DELETE FROM files WHERE path = ? OR (path >= ? AND path < ?)")
            for prefix in rules.excludedPrefixes {
                let r = Self.descendantRange(prefix)
                st.reset(); st.bind(1, prefix); st.bind(2, r.lower); st.bind(3, r.upper); try st.run()
                removed += Int(sqlite3_changes(db))
            }
            let byName = try Statement(db: db!, sql: "DELETE FROM files WHERE path LIKE ? ESCAPE '\\' OR path LIKE ? ESCAPE '\\'")
            for name in rules.excludedDirectoryNames {
                let escaped = name.replacingOccurrences(of: "_", with: "\\_").replacingOccurrences(of: "%", with: "\\%")
                byName.reset(); byName.bind(1, "%/\(escaped)/%"); byName.bind(2, "%/\(escaped)"); try byName.run()
                removed += Int(sqlite3_changes(db))
            }
            if rules.skipHidden {
                try exec("DELETE FROM files WHERE path LIKE '%/.%'")
                removed += Int(sqlite3_changes(db))
            }
            return removed
        }
    }

    /// 폴더 아래(재귀) 파일 수와 총 용량.
    public func summary(under rawPrefix: String) throws -> (files: Int, bytes: Int64) {
        let prefix = ScanRules.normalize(rawPrefix)
        return try sync {
            let r = Self.descendantRange(prefix)
            let st = try Statement(db: db!, sql: "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files WHERE kind = 0 AND path >= ? AND path < ?")
            st.bind(1, r.lower); st.bind(2, r.upper)
            _ = st.step()
            return (Int(st.int64(0)), st.int64(1))
        }
    }

    // MARK: - 무시 목록 (중복 화면에서 "이건 알고 있는 중복" 처리)

    public func ignoreList() throws -> IgnoreList {
        try sync {
            try exec("CREATE TABLE IF NOT EXISTS ignores(kind TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(kind, value))")
            let st = try Statement(db: db!, sql: "SELECT kind, value FROM ignores")
            var hashes = Set<String>(); var folders: [String] = []
            while st.step() {
                if st.text(0) == "hash" { hashes.insert(st.text(1) ?? "") } else { folders.append(st.text(1) ?? "") }
            }
            return IgnoreList(hashes: hashes, folders: folders)
        }
    }

    public func addIgnore(hash: String) throws { try addIgnore(kind: "hash", value: hash) }
    public func addIgnore(folder: String) throws { try addIgnore(kind: "folder", value: ScanRules.normalize(folder)) }

    private func addIgnore(kind: String, value: String) throws {
        try sync {
            try exec("CREATE TABLE IF NOT EXISTS ignores(kind TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(kind, value))")
            let st = try Statement(db: db!, sql: "INSERT OR IGNORE INTO ignores(kind, value) VALUES(?, ?)")
            st.bind(1, kind); st.bind(2, value); try st.run()
        }
    }

    public func removeIgnore(hash: String) throws { try removeIgnore(kind: "hash", value: hash) }
    public func removeIgnore(folder: String) throws { try removeIgnore(kind: "folder", value: ScanRules.normalize(folder)) }

    private func removeIgnore(kind: String, value: String) throws {
        try sync {
            let st = try Statement(db: db!, sql: "DELETE FROM ignores WHERE kind = ? AND value = ?")
            st.bind(1, kind); st.bind(2, value); try st.run()
        }
    }

    /// 같은 크기의 일반 파일이 2개 이상인 묶음. 중복 탐색의 1차 후보.
    /// 하드링크(같은 inode)는 같은 파일이므로 한 묶음 안에서 하나로 접는다.
    /// `under`를 주면 "그 아래 파일이 적어도 하나 포함된" 크기 묶음만 — 묶음의 다른 멤버는 디스크 어디든 포함된다.
    public func sizeGroups(minSize: Int64 = 1, under: String? = nil) throws -> [[FileRecord]] {
        let under = under.map { ScanRules.normalize($0) }
        return try sync {
            var sql = """
            SELECT \(Self.columns) FROM files WHERE kind = 0 AND dataless = 0 AND size >= ?
              AND size IN (SELECT size FROM files WHERE kind = 0 AND dataless = 0 AND size >= ? GROUP BY size HAVING COUNT(*) > 1)
            """
            if under != nil {
                sql += " AND size IN (SELECT size FROM files WHERE kind = 0 AND dataless = 0 AND path >= ? AND path < ?)"
            }
            sql += " ORDER BY size DESC, path"
            let st = try Statement(db: db!, sql: sql)
            st.bind(1, minSize)
            st.bind(2, minSize)
            if let under { let r = Self.descendantRange(under); st.bind(3, r.lower); st.bind(4, r.upper) }
            var groups: [[FileRecord]] = []
            var current: [FileRecord] = []
            var seenInodes = Set<UInt64>()
            while st.step() {
                let r = read(st)
                if let last = current.last, last.size != r.size {
                    if current.count > 1 { groups.append(current) }
                    current.removeAll(keepingCapacity: true); seenInodes.removeAll()
                }
                if r.inode != 0 && !seenInodes.insert(r.inode).inserted { continue }
                current.append(r)
            }
            if current.count > 1 { groups.append(current) }
            return groups
        }
    }

    public struct Stats: Sendable {
        public let files: Int
        public let directories: Int
        public let bytes: Int64
        public let hashed: Int
    }

    public func stats() throws -> Stats {
        try sync {
            let st = try Statement(db: db!, sql: """
            SELECT SUM(kind = 0), SUM(kind = 1), COALESCE(SUM(CASE WHEN kind = 0 THEN size END), 0), SUM(hash IS NOT NULL) FROM files
            """)
            _ = st.step()
            return Stats(files: Int(st.int64(0)), directories: Int(st.int64(1)), bytes: st.int64(2), hashed: Int(st.int64(3)))
        }
    }

    // MARK: - 작업 로그

    @discardableResult
    public func logOperation(kind: Operation.Kind, source: String, destination: String, batch: Int64? = nil, inode: UInt64 = 0) throws -> Int64 {
        try sync {
            let st = try Statement(db: db!, sql: "INSERT INTO ops(ts, kind, src, dst, batch, inode) VALUES(?,?,?,?,?,?)")
            st.bind(1, Date().timeIntervalSince1970); st.bind(2, kind.rawValue); st.bind(3, source); st.bind(4, destination)
            if let batch { st.bind(5, batch) } else { st.bindNull(5) }
            st.bind(6, Int64(bitPattern: inode))
            try st.run()
            return sqlite3_last_insert_rowid(db)
        }
    }

    /// 새 작업 묶음 번호.
    public func nextBatchID() throws -> Int64 {
        try sync { try scalar("SELECT COALESCE(MAX(batch), 0) + 1 FROM ops") }
    }

    /// 지금까지 기록된 마지막 작업 번호 — 앱을 켠 시점을 기억해 그 뒤 작업만 되돌리게 한다.
    public func maxOperationID() throws -> Int64 {
        try sync { try scalar("SELECT COALESCE(MAX(id), 0) FROM ops") }
    }

    private static let opColumns = "id, ts, kind, src, dst, undone, batch, inode"

    public func lastUndoableOperation(after minID: Int64 = 0) throws -> Operation? {
        try sync {
            let st = try Statement(db: db!, sql: "SELECT \(Self.opColumns) FROM ops WHERE undone = 0 AND id > ? ORDER BY id DESC LIMIT 1")
            st.bind(1, minID)
            return st.step() ? readOp(st) : nil
        }
    }

    /// 되돌릴 차례인 작업 묶음 (최근 것부터). 묶음 번호가 없는 옛 기록은 한 건씩.
    public func lastUndoableBatch(after minID: Int64 = 0) throws -> [Operation] {
        guard let last = try lastUndoableOperation(after: minID) else { return [] }
        guard let batch = last.batch else { return [last] }
        return try sync {
            let st = try Statement(db: db!, sql: "SELECT \(Self.opColumns) FROM ops WHERE undone = 0 AND id > ? AND batch = ? ORDER BY id DESC")
            st.bind(1, minID); st.bind(2, batch)
            var out: [Operation] = []
            while st.step() { out.append(readOp(st)) }
            return out
        }
    }

    public func markUndone(id: Int64) throws {
        try sync {
            let st = try Statement(db: db!, sql: "UPDATE ops SET undone = 1 WHERE id = ?")
            st.bind(1, id); try st.run()
            guard sqlite3_changes(db) == 1 else { throw IndexError.sqlite("작업 기록 #\(id)을 갱신하지 못함") }
        }
    }

    public func history(limit: Int = 50) throws -> [Operation] {
        try sync {
            let st = try Statement(db: db!, sql: "SELECT \(Self.opColumns) FROM ops ORDER BY id DESC LIMIT ?")
            st.bind(1, Int64(limit))
            var out: [Operation] = []
            while st.step() { out.append(readOp(st)) }
            return out
        }
    }

    private func readOp(_ st: Statement) -> Operation {
        Operation(
            id: st.int64(0), timestamp: Date(timeIntervalSince1970: st.double(1)),
            kind: Operation.Kind(rawValue: st.text(2) ?? "") ?? .move,
            source: st.text(3) ?? "", destination: st.text(4) ?? "", undone: st.int64(5) != 0,
            batch: st.isNull(6) ? nil : st.int64(6), inode: UInt64(bitPattern: st.int64(7)))
    }

    // MARK: - 이름 검색

    /// 폴더들 아래(재귀)에서 이름에 `text`가 들어간 항목. 폴더 먼저, 이름순, 최대 `limit`개.
    public func search(name text: String, under roots: [String], limit: Int = 1000) throws -> [FileRecord] {
        let needle = text.precomposedStringWithCanonicalMapping
        guard !needle.isEmpty else { return [] }
        let escaped = needle.replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "%", with: "\\%").replacingOccurrences(of: "_", with: "\\_")
        return try sync {
            let st = try Statement(db: db!, sql: """
                SELECT \(Self.columns) FROM files WHERE name LIKE ? ESCAPE '\\' AND path >= ? AND path < ?
                ORDER BY kind DESC, name COLLATE NOCASE LIMIT ?
                """)
            var out: [FileRecord] = []
            var seen = Set<String>()
            for root in roots {
                let r = Self.descendantRange(ScanRules.normalize(root))
                st.reset()
                st.bind(1, "%" + escaped + "%"); st.bind(2, r.lower); st.bind(3, r.upper); st.bind(4, Int64(limit))
                while st.step() {
                    let rec = read(st)
                    if seen.insert(rec.path).inserted { out.append(rec) }
                }
            }
            return out
        }
    }

    // MARK: - 유틸

    /// `path` 아래 모든 경로를 덮는 범위 [lower, upper). '/' 다음 글자가 '0'이므로 "path/…"는 전부 그 사이에 든다.
    /// LIKE와 달리 기본 키 색인을 그대로 쓰므로 전체 표를 훑지 않는다.
    static func descendantRange(_ path: String) -> (lower: String, upper: String) {
        if path == "/" { return ("/", "0") }
        return (path + "/", path + "0")
    }

    /// `LIKE` 접두어 패턴. `%`와 `_`는 이스케이프.
    static func likePrefix(_ path: String) -> String {
        if path == "/" { return "/%" }
        let escaped = path.replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "%", with: "\\%")
            .replacingOccurrences(of: "_", with: "\\_")
        return escaped + "/%"
    }
}

/// sqlite3 prepared statement 얇은 래퍼.
final class Statement {
    let stmt: OpaquePointer

    init(db: OpaquePointer, sql: String) throws {
        var s: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &s, nil) == SQLITE_OK, let s else {
            throw IndexError.sqlite(String(cString: sqlite3_errmsg(db)) + " — " + sql)
        }
        stmt = s
    }

    deinit { sqlite3_finalize(stmt) }

    func reset() { sqlite3_reset(stmt); sqlite3_clear_bindings(stmt) }

    func bind(_ i: Int32, _ v: String) { sqlite3_bind_text(stmt, i, v, -1, SQLITE_TRANSIENT) }
    func bind(_ i: Int32, _ v: Int64) { sqlite3_bind_int64(stmt, i, v) }
    func bind(_ i: Int32, _ v: Double) { sqlite3_bind_double(stmt, i, v) }
    func bindNull(_ i: Int32) { sqlite3_bind_null(stmt, i) }

    /// 행이 있으면 true. (읽기 전용 — 쓰기는 run()으로 해서 실패를 놓치지 않는다)
    func step() -> Bool { sqlite3_step(stmt) == SQLITE_ROW }

    /// 쓰기 실행. 실패하면 던진다 (조용히 넘어가면 작업 기록·인덱스가 어긋난다).
    func run() throws {
        let rc = sqlite3_step(stmt)
        guard rc == SQLITE_DONE || rc == SQLITE_ROW else {
            throw IndexError.sqlite(String(cString: sqlite3_errmsg(sqlite3_db_handle(stmt))))
        }
    }

    func isNull(_ i: Int32) -> Bool { sqlite3_column_type(stmt, i) == SQLITE_NULL }

    func text(_ i: Int32) -> String? { sqlite3_column_text(stmt, i).map { String(cString: $0) } }
    func int64(_ i: Int32) -> Int64 { sqlite3_column_int64(stmt, i) }
    func double(_ i: Int32) -> Double { sqlite3_column_double(stmt, i) }
}
