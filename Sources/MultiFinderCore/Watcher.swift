import Foundation
import CoreServices

/// FSEvents로 변경을 받아 인덱스를 증분 갱신한다. 전체 재스캔 없이 "상시 최신"을 유지하는 부품.
public final class Watcher {
    private var stream: FSEventStreamRef?
    private let paths: [String]
    private let latency: TimeInterval
    private let queue = DispatchQueue(label: "multifinder.watcher")
    private let handler: ([String]) -> Void

    /// - Parameter onChange: 바뀐 경로 목록(파일 단위). 존재하면 갱신, 없으면 삭제로 처리하면 된다.
    public init(paths: [String], latency: TimeInterval = 2.0, onChange: @escaping ([String]) -> Void) {
        self.paths = paths
        self.latency = latency
        self.handler = onChange
    }

    deinit { stop() }

    public func start() {
        guard stream == nil else { return }
        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil, release: nil, copyDescription: nil)
        let flags = UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagNoDefer)
        guard let s = FSEventStreamCreate(
            nil,
            { _, info, count, eventPaths, _, _ in
                guard let info else { return }
                let watcher = Unmanaged<Watcher>.fromOpaque(info).takeUnretainedValue()
                let array = Unmanaged<CFArray>.fromOpaque(eventPaths).takeUnretainedValue() as NSArray
                var changed: [String] = []
                changed.reserveCapacity(count)
                for case let p as String in array { changed.append(p) }
                watcher.handler(changed)
            },
            &context,
            paths as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            latency,
            flags) else { return }
        stream = s
        FSEventStreamSetDispatchQueue(s, queue)
        FSEventStreamStart(s)
    }

    public func stop() {
        guard let s = stream else { return }
        FSEventStreamStop(s)
        FSEventStreamInvalidate(s)
        FSEventStreamRelease(s)
        stream = nil
    }

    /// 표준 처리기: 바뀐 경로를 인덱스에 반영한다.
    public static func indexUpdater(store: IndexStore, rules: ScanRules) -> ([String]) -> Void {
        return { changed in
            let gen = store.currentGeneration
            var upserts: [FileRecord] = []
            var newFolders: [String] = []
            for raw in changed {
                let path = ScanRules.normalize(raw)
                let name = (path as NSString).lastPathComponent
                if rules.isExcluded(path: path, name: name) {
                    // 제외 영역으로 들어간 파일(예: 휴지통으로 이동)은 인덱스에서도 빼준다
                    try? store.remove(path: path)
                    continue
                }
                if let rec = Scanner.record(for: URL(fileURLWithPath: path)) {
                    // Finder에서 폴더째 옮겨 오거나 복사해 온 폴더는 안쪽 항목의 이벤트가 따로 오지 않는다 → 통째로 스캔
                    if rec.kind == .directory, (try? store.record(at: path)) == nil { newFolders.append(path) }
                    upserts.append(rec)
                } else {
                    try? store.remove(path: path)
                }
            }
            try? store.upsert(upserts, generation: gen)
            // 새 폴더 안의 폴더는 바깥 스캔이 함께 다룬다
            let tops = newFolders.filter { f in !newFolders.contains { f != $0 && f.hasPrefix($0 + "/") } }
            for folder in tops {
                _ = try? Scanner(rules: rules).scan(roots: [URL(fileURLWithPath: folder)], into: store)
            }
        }
    }
}
