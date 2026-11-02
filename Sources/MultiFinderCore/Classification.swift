import Foundation
import SQLite3

/// 사용자가 정한 묶음(분류 체계). 모델은 이 목록 안에서만 고른다.
public struct Bucket: Sendable, Identifiable, Hashable, Codable {
    public var id: String { name }
    public var name: String
    /// 모델에게 주는 한 줄 설명 — "어떤 파일이 여기 속하는가"
    public var hint: String

    public init(name: String, hint: String) {
        self.name = name
        self.hint = hint
    }

    /// 어디에도 분명히 속하지 않는 파일이 가는 묶음의 이름 (화면 언어로).
    public static var fallbackName: String { String(localized: "기타", comment: "Bucket for files that fit nowhere else") }
    public static var fallbackHint: String { String(localized: "위 어디에도 분명히 속하지 않음", comment: "Description of the fallback bucket") }
    /// 묶음은 만들 때의 언어로 저장되므로, 어느 언어로 만든 '기타'든 알아본다 (시스템 언어를 바꿔도 특별 취급 유지).
    public static let fallbackNames: Set<String> = ["기타", "Other", "その他", "其他"]
    public var isFallback: Bool { Self.fallbackNames.contains(name) }
}

extension Array where Element == Bucket {
    /// 이 목록의 '기타' 묶음 이름 — 없으면 화면 언어의 기본 이름.
    public var fallbackName: String { first(where: \.isFallback)?.name ?? Bucket.fallbackName }
}

/// 파일 하나의 분류 결과. 파일이 바뀌면(수정일·크기) 무효.
public struct Classification: Sendable, Identifiable, Hashable {
    public var id: String { path }
    public let path: String
    public let category: String
    public let confidence: Double
    public let summary: String
    public let modified: Date
    public let size: Int64
    /// 사용자가 직접 고친 결과면 true — 재분류해도 덮어쓰지 않는다
    public let manual: Bool

    public init(path: String, category: String, confidence: Double, summary: String, modified: Date, size: Int64, manual: Bool) {
        self.path = path
        self.category = category
        self.confidence = confidence
        self.summary = summary
        self.modified = modified
        self.size = size
        self.manual = manual
    }
}

extension IndexStore {
    private func ensureClassificationTables() throws {
        try execPublic("""
        CREATE TABLE IF NOT EXISTS categories(name TEXT PRIMARY KEY, hint TEXT NOT NULL, position INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS classes(
            path TEXT PRIMARY KEY, category TEXT NOT NULL, confidence REAL NOT NULL, summary TEXT NOT NULL,
            mtime REAL NOT NULL, size INTEGER NOT NULL, manual INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS classes_category ON classes(category);
        """)
    }

    // MARK: 묶음 체계

    public func categories() throws -> [Bucket] {
        try ensureClassificationTables()
        return try withStatement("SELECT name, hint FROM categories ORDER BY position") { st in
            var out: [Bucket] = []
            while st.step() { out.append(Bucket(name: st.text(0) ?? "", hint: st.text(1) ?? "")) }
            return out
        }
    }

    public func saveCategories(_ list: [Bucket]) throws {
        try ensureClassificationTables()
        try transaction {
            try execInTransaction("DELETE FROM categories")
            let st = try statement("INSERT INTO categories(name, hint, position) VALUES(?,?,?)")
            for (i, c) in list.enumerated() {
                st.reset(); st.bind(1, c.name); st.bind(2, c.hint); st.bind(3, Int64(i)); try st.run()
            }
        }
    }

    // MARK: 분류 결과

    public func classification(at path: String) throws -> Classification? {
        try ensureClassificationTables()
        return try withStatement("SELECT path, category, confidence, summary, mtime, size, manual FROM classes WHERE path = ?") { st in
            st.bind(1, ScanRules.normalize(path))
            return st.step() ? Self.readClass(st) : nil
        }
    }

    /// 폴더 아래 파일들의 분류 (파일이 그 뒤 바뀐 것은 제외).
    public func classifications(under rawPrefix: String, recursive: Bool = true) throws -> [Classification] {
        try ensureClassificationTables()
        let prefix = ScanRules.normalize(rawPrefix)
        let scopeClause = recursive ? "c.path >= ? AND c.path < ?" : "f.parent = ?"
        return try withStatement("""
            SELECT c.path, c.category, c.confidence, c.summary, c.mtime, c.size, c.manual
            FROM classes c JOIN files f ON f.path = c.path
            WHERE \(scopeClause) AND f.mtime = c.mtime AND f.size = c.size
            ORDER BY c.category, c.path
            """) { st in
            if recursive {
                let r = Self.descendantRange(prefix); st.bind(1, r.lower); st.bind(2, r.upper)
            } else {
                st.bind(1, prefix)
            }
            var out: [Classification] = []
            while st.step() { out.append(Self.readClass(st)) }
            return out
        }
    }

    /// 아직 분류되지 않았거나(또는 바뀐) 파일 — 분류 대상 후보.
    public func unclassifiedFiles(under rawPrefix: String, extensions: Set<String>, recursive: Bool = true) throws -> [FileRecord] {
        try ensureClassificationTables()
        let prefix = ScanRules.normalize(rawPrefix)
        let scopeClause = recursive ? "f.path >= ? AND f.path < ?" : "f.parent = ?"
        return try withStatement("""
            SELECT \(Self.columnsPublic.split(separator: ",").map { "f.\($0)" }.joined(separator: ",")) FROM files f
            LEFT JOIN classes c ON c.path = f.path AND c.mtime = f.mtime AND c.size = f.size
            WHERE f.kind = 0 AND f.dataless = 0 AND \(scopeClause) AND c.path IS NULL
            ORDER BY (length(f.path) - length(replace(f.path, '/', ''))) ASC, f.mtime DESC
            """) { st in
            if recursive {
                let r = Self.descendantRange(prefix); st.bind(1, r.lower); st.bind(2, r.upper)
            } else {
                st.bind(1, prefix)
            }
            var out: [FileRecord] = []
            while st.step() {
                let r = Self.readPublic(st)
                if extensions.contains(r.ext) { out.append(r) }
            }
            return out
        }
    }

    public func saveClassification(_ c: Classification) throws {
        try ensureClassificationTables()
        try withStatement("""
            INSERT INTO classes(path, category, confidence, summary, mtime, size, manual) VALUES(?,?,?,?,?,?,?)
            ON CONFLICT(path) DO UPDATE SET category=excluded.category, confidence=excluded.confidence, summary=excluded.summary,
                mtime=excluded.mtime, size=excluded.size, manual=excluded.manual
            """) { st in
            st.bind(1, c.path); st.bind(2, c.category); st.bind(3, c.confidence); st.bind(4, c.summary)
            st.bind(5, c.modified.timeIntervalSince1970); st.bind(6, c.size); st.bind(7, Int64(c.manual ? 1 : 0))
            try st.run()
        }
    }

    /// 사용자가 직접 묶음을 바꿈 (manual 표시 → 재분류해도 유지).
    public func reclassifyManually(path: String, to category: String) throws {
        try ensureClassificationTables()
        try withStatement("UPDATE classes SET category = ?, manual = 1, confidence = 1.0 WHERE path = ?") { st in
            st.bind(1, category); st.bind(2, ScanRules.normalize(path)); try st.run()
        }
    }

    public func clearClassifications(under rawPrefix: String?, keepManual: Bool = true) throws {
        try ensureClassificationTables()
        var sql = "DELETE FROM classes WHERE 1=1"
        if rawPrefix != nil { sql += " AND path >= ? AND path < ?" }
        if keepManual { sql += " AND manual = 0" }
        try withStatement(sql) { st in
            if let p = rawPrefix {
                let r = Self.descendantRange(ScanRules.normalize(p)); st.bind(1, r.lower); st.bind(2, r.upper)
            }
            try st.run()
        }
    }

    private static func readClass(_ st: Statement) -> Classification {
        Classification(
            path: st.text(0) ?? "", category: st.text(1) ?? Bucket.fallbackName, confidence: st.double(2),
            summary: st.text(3) ?? "", modified: Date(timeIntervalSince1970: st.double(4)), size: st.int64(5),
            manual: st.int64(6) != 0)
    }
}
