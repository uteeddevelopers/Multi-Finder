import Foundation

/// 어떤 경로가 "프로젝트"(코드 저장소·패키지) 안에 있는지, 그 프로젝트 루트는 어디인지.
///
/// 같은 프로젝트 안에서 같은 내용이 두 번 나오는 것(예: `public/landing/x.mp4`와 `marketing/x.mp4`)은
/// 대개 의도된 것이라 중복 정리 대상에서 기본으로 숨긴다.
public final class ProjectRoots {
    /// 이 이름이 폴더 안에 있으면 그 폴더를 프로젝트 루트로 본다.
    public static let markers: [String] = [
        ".git", "package.json", "Package.swift", "Cargo.toml", "pyproject.toml", "requirements.txt",
        "pom.xml", "build.gradle", "build.gradle.kts", "go.mod", "Gemfile", "composer.json", "Podfile",
        "pubspec.yaml", "CMakeLists.txt", "Makefile",
    ]

    private var cache: [String: String?] = [:]
    private let ceilings: Set<String>
    private let fm = FileManager.default

    /// - Parameter ceilings: 이 폴더들(과 그 위)은 프로젝트 루트로 보지 않는다. 기본: 홈과 /.
    public init(ceilings: [String] = [NSHomeDirectory(), "/"]) {
        self.ceilings = Set(ceilings.map { ScanRules.normalize($0) })
    }

    /// `path`를 포함하는 가장 가까운 프로젝트 루트. 없으면 nil.
    public func root(of path: String) -> String? {
        var dir = (path as NSString).deletingLastPathComponent
        var visited: [String] = []
        var found: String?
        while !dir.isEmpty, dir != "/", !ceilings.contains(dir) {
            if let cached = cache[dir] { found = cached; break }
            visited.append(dir)
            if isProjectRoot(dir) { found = dir; break }
            dir = (dir as NSString).deletingLastPathComponent
        }
        for v in visited { cache[v] = found }
        return found
    }

    private func isProjectRoot(_ dir: String) -> Bool {
        for m in Self.markers where fm.fileExists(atPath: (dir as NSString).appendingPathComponent(m)) { return true }
        // Xcode 프로젝트/워크스페이스
        if let items = try? fm.contentsOfDirectory(atPath: dir),
           items.contains(where: { $0.hasSuffix(".xcodeproj") || $0.hasSuffix(".xcworkspace") }) {
            return true
        }
        return false
    }
}
