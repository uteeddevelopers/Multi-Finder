import Foundation

/// 내용이 완전히 같은 파일들의 묶음.
public struct DuplicateGroup: Sendable, Identifiable {
    public var id: String { hash }
    public let hash: String
    public let size: Int64
    /// 경로순 정렬. "어느 것을 남길지"는 사람이 정한다.
    public let files: [FileRecord]
    /// 하나만 남기면 돌아오는 용량(명목). APFS 클론이면 실제론 덜 돌아올 수 있다.
    public var wastedBytes: Int64 { size * Int64(max(files.count - 1, 0)) }
}

/// 중복 = 내용 동일. 3단계로 좁힌다: 크기 → 앞·뒤 샘플 지문 → 전체 지문.
/// 전체 지문은 인덱스에 저장되어 파일이 안 바뀌면 다시 계산하지 않는다.
public final class DuplicateFinder {
    private let store: IndexStore

    public init(store: IndexStore) { self.store = store }

    /// - Parameters:
    ///   - minSize: 이보다 작은 파일은 무시 (기본 1바이트, 빈 파일 제외)
    ///   - under: 주면 이 폴더 아래 파일이 포함된 묶음만 돌려준다 (다른 멤버는 어디든)
    ///   - progress: (처리한 크기 묶음 수, 전체 크기 묶음 수)
    public func find(
        minSize: Int64 = 1,
        under: String? = nil,
        progress: ((Int, Int) -> Void)? = nil
    ) throws -> [DuplicateGroup] {
        let sizeGroups = try store.sizeGroups(minSize: minSize, under: under)
        var result: [DuplicateGroup] = []
        let underPrefix = under.map { ScanRules.normalize($0) + "/" }

        for (i, group) in sizeGroups.enumerated() {
            defer { progress?(i + 1, sizeGroups.count) }

            // 인덱스는 낡았을 수 있다(감시하지 않는 폴더 등) — 디스크의 지금 상태와 맞춰 본다.
            // 사라졌거나 크기가 바뀐 파일은 빼고, 수정 시각만 바뀐 파일은 저장된 지문을 버리고 새로 계산한다.
            var current: [FileRecord] = []
            for f in group {
                guard let now = Scanner.record(for: f.url) else { try? store.remove(path: f.path); continue }
                guard now.kind == .file, now.size == f.size, !now.isDataless else { continue }
                if now.modified.timeIntervalSince1970 != f.modified.timeIntervalSince1970 {
                    try? store.upsert([now], generation: store.currentGeneration)   // 바뀐 파일의 지문은 지워진다
                    current.append(now)
                } else {
                    current.append(f)
                }
            }
            guard current.count >= 2 else { continue }

            // 2단계: 빠른 지문(앞·뒤 128KB)으로 후보 좁히기. 전체 지문이 이미 있는 파일도
            // 같은 버킷에 들어가야 비교가 되므로 전부 계산한다 (싸다).
            var byQuick: [String: [FileRecord]] = [:]
            for f in current {
                guard let q = try? ContentHasher.quickHash(of: f.url, size: f.size) else { continue }
                byQuick[q, default: []].append(f)
            }

            // 3단계: 버킷 안에서 전체 지문. 저장된 지문은 재사용.
            var byFull: [String: [FileRecord]] = [:]
            for files in byQuick.values where files.count >= 2 {
                for var f in files {
                    if f.hash == nil {
                        guard let h = try? ContentHasher.sha256(of: f.url) else { continue }
                        try? store.setHash(path: f.path, hash: h, modified: f.modified, size: f.size)
                        f.hash = h
                    }
                    byFull[f.hash!, default: []].append(f)
                }
            }
            for (h, files) in byFull where files.count >= 2 {
                if let underPrefix, !files.contains(where: { $0.path.hasPrefix(underPrefix) }) { continue }
                result.append(DuplicateGroup(hash: h, size: files[0].size, files: files.sorted { $0.path < $1.path }))
            }
        }
        return result.sorted { $0.wastedBytes > $1.wastedBytes }
    }
}
