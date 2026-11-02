import Foundation

/// 무엇을 인덱스에 넣지 않을지 정하는 규칙.
///
/// 원칙: 탐색은 디스크 전체로 하되, 결과가 나와도 사용자가 손댈 수 없거나 손대면 안 되는 곳
/// (시스템 영역, 캐시, 빌드 산출물, 앱 샌드박스 컨테이너)은 처음부터 제외한다.
public struct ScanRules: Sendable {
    /// 숨김 파일/폴더(.으로 시작, 또는 hidden 플래그) 건너뛰기.
    public var skipHidden: Bool
    /// 이 경로 또는 그 아래는 전부 제외.
    public var excludedPrefixes: [String]
    /// 어디에 있든 이 이름의 폴더는 제외 (node_modules 등).
    public var excludedDirectoryNames: Set<String>

    public init(skipHidden: Bool, excludedPrefixes: [String], excludedDirectoryNames: Set<String>) {
        self.skipHidden = skipHidden
        self.excludedPrefixes = excludedPrefixes.map { Self.normalize($0) }
        self.excludedDirectoryNames = excludedDirectoryNames
    }

    public static func `default`(home: String = NSHomeDirectory()) -> ScanRules {
        let h = Self.normalize(home)
        return ScanRules(
            skipHidden: true,
            excludedPrefixes: [
                // 시스템 영역: 읽기 전용이거나 사용자가 손댈 수 없음
                "/System", "/usr", "/bin", "/sbin", "/dev", "/private", "/cores", "/opt",
                "/Library",
                // 외장·네트워크 볼륨(/Volumes)은 제외하지 않는다 — 사용자가 직접 고른 폴더만 스캔하므로 외장 디스크 폴더도 다룰 수 있게
                // 홈 아래 캐시·컨테이너·개발 도구 산출물
                h + "/Library/Caches",
                h + "/Library/Containers",
                h + "/Library/Group Containers",
                h + "/Library/Developer",
                h + "/Library/Logs",
                h + "/Library/Metadata",
                h + "/Library/Application Support/MultiFinder",
                // 휴지통: 이미 버린 것은 정리 대상이 아니다 (중복으로도 잡지 않음)
                h + "/.Trash",
            ],
            excludedDirectoryNames: [
                "node_modules", ".git", ".build", "DerivedData", "Pods",
                ".venv", "venv", "__pycache__", ".cache", ".npm", ".nvm", ".Trash",
            ]
        )
    }

    /// `path`를 건너뛰어야 하는지. 폴더이면 그 아래 전체가 함께 빠진다.
    /// 스캔 중 열거기가 넘기는 경로뿐 아니라, 변경 감시가 넘기는 임의 깊이의 경로에도 쓰이므로
    /// 이름만이 아니라 **상위 폴더 이름 전부**를 검사한다 (예: `~/.Trash/x.mp4`, `a/node_modules/b/c.js`).
    public func isExcluded(path: String, name: String) -> Bool {
        if excludedDirectoryNames.contains(name) { return true }
        for prefix in excludedPrefixes {
            if path == prefix || path.hasPrefix(prefix + "/") { return true }
        }
        for component in path.split(separator: "/") {
            if excludedDirectoryNames.contains(String(component)) { return true }
            if skipHidden && component.hasPrefix(".") { return true }
        }
        return false
    }

    /// 경로 정규화. 인덱스의 모든 경로는 이 형태로 저장·조회한다.
    ///
    /// 파일 열거기는 `/tmp` → `/private/tmp` 처럼 심볼릭 링크가 풀린 경로를 돌려주므로,
    /// 조회 경로도 같은 식으로 **부모 폴더까지만** 풀어서 맞춘다. 마지막 구성요소는 풀지 않는다 —
    /// 심볼릭 링크 자체를 옮기거나 휴지통에 넣으려는데 그 대상이 대신 처리되는 일을 막기 위해.
    public static func normalize(_ p: String) -> String {
        var s = (p as NSString).standardizingPath
        while s.count > 1 && s.hasSuffix("/") { s.removeLast() }
        guard s != "/" else { return s }
        let url = URL(fileURLWithPath: s)
        let parent = realPath(url.deletingLastPathComponent().path)
        // 한글 등은 파일 시스템이 분해형(NFD)으로 돌려줄 수 있다. 인덱스의 모든 경로는 조합형(NFC)으로 통일.
        return ((parent as NSString).appendingPathComponent(url.lastPathComponent)).precomposedStringWithCanonicalMapping
    }

    /// 심볼릭 링크를 끝까지 푼 경로. Foundation의 resolvingSymlinksInPath는 `/private`를 떼어내
    /// 파일 열거기가 돌려주는 형태(`/private/tmp/...`)와 어긋나므로 libc realpath를 쓴다.
    /// 존재하지 않는 경로는 존재하는 가장 긴 앞부분만 풀고 나머지는 그대로 붙인다.
    public static func realPath(_ p: String) -> String {
        if let resolved = realpath(p, nil) {
            defer { free(resolved) }
            return String(cString: resolved)
        }
        let ns = p as NSString
        guard ns.length > 1, ns.deletingLastPathComponent != p else { return p }
        return (realPath(ns.deletingLastPathComponent) as NSString).appendingPathComponent(ns.lastPathComponent)
    }
}

/// 실행기(이동·휴지통)가 절대 건드리지 않는 경로.
///
/// 인덱스에는 들어가되(보기는 가능) 변경은 거부한다. 동기화 폴더에서 지우면 다른 기기에서도 사라지므로 포함.
public struct ProtectedPaths: Sendable {
    public var prefixes: [String]
    /// 홈 바로 아래 표준 폴더 — 폴더 자체를 옮기거나 지우는 일은 거부.
    public var rootFolders: Set<String>

    public init(prefixes: [String], rootFolders: Set<String>) {
        self.prefixes = prefixes.map { ScanRules.normalize($0) }
        self.rootFolders = rootFolders
    }

    public static func `default`(home: String = NSHomeDirectory(), extra: [String] = []) -> ProtectedPaths {
        let h = ScanRules.normalize(home)
        return ProtectedPaths(
            prefixes: [
                "/System", "/usr", "/bin", "/sbin", "/dev", "/private", "/Library", "/Applications",
                h + "/Library/Mobile Documents",   // iCloud Drive
            ] + extra,
            rootFolders: [
                h,
                h + "/Desktop", h + "/Documents", h + "/Downloads", h + "/Library",
                h + "/Pictures", h + "/Movies", h + "/Music", h + "/Public", h + "/Applications",
            ]
        )
    }

    public func isProtected(_ path: String) -> Bool {
        let p = ScanRules.normalize(path)
        if rootFolders.contains(p) || p == "/" { return true }
        for prefix in prefixes where p == prefix || p.hasPrefix(prefix + "/") { return true }
        return false
    }
}
