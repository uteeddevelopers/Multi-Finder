import Foundation

public enum DiffStatus: String, Sendable, CaseIterable {
    /// 같은 상대 경로, 같은 내용
    case identical
    /// 같은 상대 경로, 다른 내용
    case modified
    /// 왼쪽에만 있고 오른쪽 어디에도 같은 내용이 없음
    case leftOnly
    /// 오른쪽에만 있고 왼쪽 어디에도 같은 내용이 없음
    case rightOnly
    /// 한쪽에만 있지만 반대쪽 다른 경로에 같은 내용이 있음 (옮겨졌거나 이름이 다름)
    case relocated

    public var label: String {
        switch self {
        case .identical: return String(localized: "동일", comment: "Folder compare status: same path, same content")
        case .modified: return String(localized: "다름", comment: "Folder compare status: same path, different content")
        case .leftOnly: return String(localized: "왼쪽에만", comment: "Folder compare status")
        case .rightOnly: return String(localized: "오른쪽에만", comment: "Folder compare status")
        case .relocated: return String(localized: "자리만 다름", comment: "Folder compare status: same content at a different path")
        }
    }
}

public struct DiffEntry: Sendable, Identifiable {
    public var id: String { relativePath + "|" + (left?.path ?? "") + "|" + (right?.path ?? "") }
    public let relativePath: String
    public let left: FileRecord?
    public let right: FileRecord?
    public let status: DiffStatus
    /// relocated일 때 반대쪽에서 같은 내용을 가진 파일의 경로
    public let counterpart: String?
}

public struct FolderDiffResult: Sendable {
    public let left: String
    public let right: String
    public let entries: [DiffEntry]
    public func count(_ s: DiffStatus) -> Int { entries.filter { $0.status == s }.count }
    public func bytes(_ s: DiffStatus) -> Int64 {
        entries.filter { $0.status == s }.reduce(0) { $0 + ($1.left?.size ?? $1.right?.size ?? 0) }
    }
}

/// 두 폴더를 내용 기준으로 비교한다. 지문은 필요한 파일(크기가 맞는 짝이 있는 파일)만 계산하고 인덱스에 저장한다.
public final class FolderDiff {
    private let store: IndexStore
    public init(store: IndexStore) { self.store = store }

    public func compare(left rawLeft: String, right rawRight: String, progress: ((Int, Int) -> Void)? = nil) throws -> FolderDiffResult {
        let left = ScanRules.normalize(rawLeft), right = ScanRules.normalize(rawRight)
        let lFiles = try store.files(under: left).filter { !$0.isDataless }
        let rFiles = try store.files(under: right).filter { !$0.isDataless }
        var lByRel: [String: FileRecord] = [:]
        for f in lFiles { lByRel[String(f.path.dropFirst(left.count + 1))] = f }
        var rByRel: [String: FileRecord] = [:]
        for f in rFiles { rByRel[String(f.path.dropFirst(right.count + 1))] = f }

        var entries: [DiffEntry] = []
        var unmatchedL: [FileRecord] = []
        var unmatchedR: [FileRecord] = []
        let total = Set(lByRel.keys).union(rByRel.keys).count
        var done = 0

        for rel in Set(lByRel.keys).union(rByRel.keys).sorted() {
            defer { done += 1; if done % 50 == 0 { progress?(done, total) } }
            switch (lByRel[rel], rByRel[rel]) {
            case let (l?, r?):
                // 읽을 수 없는 파일(그새 지워짐·권한)은 '같다'고 하지 않는다 — 같다고 하면 한쪽을 지우라고 권하게 된다
                if l.size != r.size {
                    entries.append(DiffEntry(relativePath: rel, left: l, right: r, status: .modified, counterpart: nil))
                } else if let lh = hash(l), lh == hash(r) {
                    entries.append(DiffEntry(relativePath: rel, left: l, right: r, status: .identical, counterpart: nil))
                } else {
                    entries.append(DiffEntry(relativePath: rel, left: l, right: r, status: .modified, counterpart: nil))
                }
            case let (l?, nil): unmatchedL.append(l)
            case let (nil, r?): unmatchedR.append(r)
            case (nil, nil): break
            }
        }

        // 한쪽에만 있는 파일: 반대쪽에서 같은 크기 → 같은 지문인 파일을 찾는다 (옮겨진 것/이름만 다른 것)
        let rBySize = Dictionary(grouping: unmatchedR, by: \.size)
        var claimedR = Set<String>()
        for l in unmatchedL {
            var found: FileRecord?
            if let candidates = rBySize[l.size], l.size > 0, let lh = hash(l) {
                for c in candidates where !claimedR.contains(c.path) {
                    if hash(c) == lh { found = c; break }
                }
            }
            if let found {
                claimedR.insert(found.path)
                entries.append(DiffEntry(relativePath: String(l.path.dropFirst(left.count + 1)), left: l, right: nil, status: .relocated, counterpart: found.path))
                entries.append(DiffEntry(relativePath: String(found.path.dropFirst(right.count + 1)), left: nil, right: found, status: .relocated, counterpart: l.path))
            } else {
                entries.append(DiffEntry(relativePath: String(l.path.dropFirst(left.count + 1)), left: l, right: nil, status: .leftOnly, counterpart: nil))
            }
        }
        for r in unmatchedR where !claimedR.contains(r.path) {
            entries.append(DiffEntry(relativePath: String(r.path.dropFirst(right.count + 1)), left: nil, right: r, status: .rightOnly, counterpart: nil))
        }
        progress?(total, total)
        return FolderDiffResult(left: left, right: right, entries: entries.sorted { $0.relativePath < $1.relativePath })
    }

    private var hashCache: [String: String] = [:]

    /// 내용 지문. 읽을 수 없으면 nil.
    private func hash(_ f: FileRecord) -> String? {
        if let h = f.hash { return h }
        if let h = hashCache[f.path] { return h }
        guard let h = try? ContentHasher.sha256(of: f.url) else { return nil }
        hashCache[f.path] = h
        try? store.setHash(path: f.path, hash: h, modified: f.modified, size: f.size)
        return h
    }
}
