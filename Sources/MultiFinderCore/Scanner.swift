import Foundation

public struct ScanProgress: Sendable {
    public let scanned: Int
    public let currentPath: String
}

/// 디스크를 걸어 다니며 인덱스를 채운다. 읽기만 하고 아무것도 바꾸지 않는다.
public final class Scanner {
    public let rules: ScanRules

    private static let keys: Set<URLResourceKey> = [
        .isDirectoryKey, .isSymbolicLinkKey, .isPackageKey, .fileSizeKey,
        .contentModificationDateKey, .fileIdentifierKey,
        .isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey,
    ]

    public init(rules: ScanRules = .default()) {
        self.rules = rules
    }

    /// 스캔은 한 번에 하나씩. 겹치는 폴더를 동시에 스캔하면 먼저 시작한 스캔이 행에 옛 세대를 다시 찍고,
    /// 나중 스캔의 마무리(finishScan)가 그 행을 '사라진 파일'로 지워 버린다.
    private static let lock = NSRecursiveLock()

    /// `roots` 아래 전체를 스캔해 `store`에 반영. 반환값은 기록한 항목 수.
    /// 스캔 뒤 roots 아래에서 이번에 보이지 않은 항목은 인덱스에서 지운다.
    @discardableResult
    public func scan(
        roots: [URL],
        into store: IndexStore,
        batchSize: Int = 2000,
        progress: ((ScanProgress) -> Void)? = nil
    ) throws -> Int {
        Self.lock.lock()
        defer { Self.lock.unlock() }
        let gen = store.beginScan()
        var total = 0
        var options: FileManager.DirectoryEnumerationOptions = [.skipsPackageDescendants]
        if rules.skipHidden { options.insert(.skipsHiddenFiles) }

        var scannedRoots: [String] = []
        for root in roots {
            // 루트는 끝까지 링크를 풀어 열거기가 돌려주는 경로와 같은 형태로 맞춘다
            let rootPath = ScanRules.normalize(ScanRules.realPath(ScanRules.normalize(root.path)))
            scannedRoots.append(rootPath)
            if rules.isExcluded(path: rootPath, name: root.lastPathComponent) { continue }
            // 루트 자체도 한 항목으로 기록 (폴더 트리의 꼭대기). 루트가 파일이면 이것으로 끝.
            guard let rootRecord = Self.record(for: URL(fileURLWithPath: rootPath)) else { continue }
            try store.upsert([rootRecord], generation: gen)
            total += 1
            guard rootRecord.kind == .directory else { continue }

            var batch: [FileRecord] = []
            guard let enumerator = FileManager.default.enumerator(
                at: URL(fileURLWithPath: rootPath),
                includingPropertiesForKeys: Array(Self.keys),
                options: options,
                errorHandler: { _, _ in true }   // 권한 없는 폴더 등은 건너뛰고 계속
            ) else { continue }

            while let url = enumerator.nextObject() as? URL {
                let path = url.path
                if rules.isExcluded(path: path, name: url.lastPathComponent) {
                    enumerator.skipDescendants()
                    continue
                }
                guard let rec = Self.record(for: url) else { continue }
                batch.append(rec)
                total += 1
                if batch.count >= batchSize {
                    try store.upsert(batch, generation: gen)
                    batch.removeAll(keepingCapacity: true)
                    progress?(ScanProgress(scanned: total, currentPath: path))
                }
            }
            try store.upsert(batch, generation: gen)
        }
        try store.finishScan(roots: scannedRoots, generation: gen)
        progress?(ScanProgress(scanned: total, currentPath: ""))
        return total
    }

    /// 경로 하나를 레코드로. 존재하지 않으면 nil.
    public static func record(for url: URL) -> FileRecord? {
        guard let v = try? url.resourceValues(forKeys: keys) else { return nil }
        let isLink = v.isSymbolicLink ?? false
        let isPackage = v.isPackage ?? false
        let isDir = v.isDirectory ?? false
        let kind: FileKind = isLink ? .symlink : isPackage ? .package : isDir ? .directory : .file

        var dataless = false
        if v.isUbiquitousItem == true, let status = v.ubiquitousItemDownloadingStatus, status != .current {
            dataless = true
        }
        if !dataless && kind == .file { dataless = Self.hasDatalessFlag(url.path) }

        // 경로는 조합형(NFC)으로 통일 (ScanRules.normalize와 같은 규칙)
        let path = url.path.precomposedStringWithCanonicalMapping
        let ns = path as NSString
        return FileRecord(
            path: path,
            name: ns.lastPathComponent,
            parent: ns.deletingLastPathComponent,
            ext: kind == .file ? ns.pathExtension.lowercased() : "",
            size: kind == .file ? Int64(v.fileSize ?? 0) : 0,
            modified: v.contentModificationDate ?? .distantPast,
            inode: v.fileIdentifier ?? 0,
            kind: kind,
            isDataless: dataless)
    }

    /// 인덱스에 기록된 뒤로 디스크의 파일이 그대로인가 (있고, 크기·수정 시각이 같음).
    /// 같으면 기록된 지문도 그대로 믿을 수 있다 — 지문은 크기·수정 시각이 바뀌면 지워지므로.
    public static func isUnchanged(_ r: FileRecord) -> Bool {
        guard let now = record(for: r.url) else { return false }
        return now.kind == r.kind && now.size == r.size
            && now.modified.timeIntervalSince1970 == r.modified.timeIntervalSince1970
    }

    /// 파일 공급자(iCloud, Google Drive 등)가 내용을 아직 내려받지 않은 파일에 붙는 SF_DATALESS 플래그.
    private static func hasDatalessFlag(_ path: String) -> Bool {
        var st = stat()
        guard lstat(path, &st) == 0 else { return false }
        let SF_DATALESS: UInt32 = 0x4000_0000
        return (st.st_flags & SF_DATALESS) != 0
    }
}
