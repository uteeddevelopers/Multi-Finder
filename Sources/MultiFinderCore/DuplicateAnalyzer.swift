import Foundation

/// 중복 묶음을 "정리할 가치가 있는 것"만 남기는 2차 해석 규칙.
public struct DuplicateOptions: Sendable, Equatable {
    /// 묶음의 모든 파일이 같은 프로젝트 루트 아래면 숨김 (의도된 중복으로 간주)
    public var hideWithinProject = true
    /// 여러 묶음이 같은 폴더 쌍/세트 사이의 사본이면 파일 단위가 아니라 "폴더 사본" 1건으로 접음
    public var foldCopyTrees = true
    /// 폴더 사본으로 접으려면 같은 폴더 세트 사이의 묶음이 최소 몇 개여야 하는지
    public var minGroupsToFold = 3
    /// 사용자가 무시하기로 한 해시·폴더 적용
    public var applyIgnoreList = true

    public init() {}
}

/// "이 폴더들끼리 내용이 겹친다" — 파일 묶음 여러 개를 폴더 단위로 접은 것.
public struct CopyTreeGroup: Sendable, Identifiable {
    public var id: String { roots.joined(separator: "\n") }
    /// 서로 사본 관계인 폴더들 (경로순)
    public let roots: [String]
    /// 접힌 파일 묶음 수
    public let groupCount: Int
    /// 접힌 묶음들의 회수 가능 용량 합
    public let wastedBytes: Int64
    /// 예시 상대 경로 몇 개
    public let samples: [String]
    /// 접힌 원본 묶음들 (펼쳐 보기용)
    public let groups: [DuplicateGroup]
}

public struct DuplicateReport: Sendable {
    public var fileGroups: [DuplicateGroup]
    public var copyTrees: [CopyTreeGroup]
    public var hiddenWithinProject: Int
    public var hiddenByIgnoreList: Int
    public var totalWastedBytes: Int64 {
        fileGroups.reduce(0) { $0 + $1.wastedBytes } + copyTrees.reduce(0) { $0 + $1.wastedBytes }
    }
}

public struct IgnoreList: Sendable, Equatable {
    public var hashes: Set<String> = []
    public var folders: [String] = []
    public init(hashes: Set<String> = [], folders: [String] = []) {
        self.hashes = hashes
        self.folders = folders.map { ScanRules.normalize($0) }
    }
    public func covers(_ path: String) -> Bool {
        folders.contains { path == $0 || path.hasPrefix($0 + "/") }
    }
}

public enum DuplicateAnalyzer {
    public static func analyze(
        _ groups: [DuplicateGroup],
        options: DuplicateOptions = DuplicateOptions(),
        ignore: IgnoreList = IgnoreList(),
        projects: ProjectRoots = ProjectRoots()
    ) -> DuplicateReport {
        var hiddenIgnored = 0
        var hiddenProject = 0
        var kept: [DuplicateGroup] = []

        for g in groups {
            // 1) 무시 목록
            if options.applyIgnoreList {
                if ignore.hashes.contains(g.hash) { hiddenIgnored += 1; continue }
                let remaining = g.files.filter { !ignore.covers($0.path) }
                if remaining.count < 2 { hiddenIgnored += 1; continue }
                if remaining.count != g.files.count {
                    kept.append(DuplicateGroup(hash: g.hash, size: g.size, files: remaining)); continue
                }
            }
            // 2) 같은 프로젝트 내부
            if options.hideWithinProject {
                let roots = Set(g.files.map { projects.root(of: $0.path) ?? "" })
                if roots.count == 1, let only = roots.first, !only.isEmpty { hiddenProject += 1; continue }
            }
            kept.append(g)
        }

        // 3) 폴더 사본 접기
        var trees: [CopyTreeGroup] = []
        if options.foldCopyTrees {
            var byRootSet: [String: [(group: DuplicateGroup, roots: [String], rel: String)]] = [:]
            for g in kept {
                guard let (roots, rel) = copyRoots(of: g) else { continue }
                byRootSet[roots.joined(separator: "\n"), default: []].append((g, roots, rel))
            }
            var folded = Set<String>()
            for (_, items) in byRootSet where items.count >= options.minGroupsToFold {
                let gs = items.map(\.group)
                trees.append(CopyTreeGroup(
                    roots: items[0].roots,
                    groupCount: gs.count,
                    wastedBytes: gs.reduce(0) { $0 + $1.wastedBytes },
                    samples: Array(items.prefix(3).map(\.rel)),
                    groups: gs.sorted { $0.wastedBytes > $1.wastedBytes }))
                for g in gs { folded.insert(g.hash) }
            }
            kept.removeAll { folded.contains($0.hash) }
            trees.sort { $0.wastedBytes > $1.wastedBytes }
        }

        return DuplicateReport(
            fileGroups: kept.sorted { $0.wastedBytes > $1.wastedBytes },
            copyTrees: trees,
            hiddenWithinProject: hiddenProject,
            hiddenByIgnoreList: hiddenIgnored)
    }

    /// 묶음의 파일들이 공유하는 끝부분(파일명 포함 공통 접미 경로)을 떼어낸 "사본 루트"들.
    /// 파일명이 서로 다르면 nil (사본 구조가 아님).
    static func copyRoots(of g: DuplicateGroup) -> (roots: [String], relative: String)? {
        let comps = g.files.map { $0.path.split(separator: "/", omittingEmptySubsequences: true).map(String.init) }
        guard let shortest = comps.map(\.count).min(), shortest > 1 else { return nil }
        var suffix = 0
        outer: while suffix < shortest - 1 {
            let candidate = comps[0][comps[0].count - 1 - suffix]
            for c in comps.dropFirst() where c[c.count - 1 - suffix] != candidate { break outer }
            suffix += 1
        }
        guard suffix >= 1 else { return nil }
        let roots = comps.map { "/" + $0.dropLast(suffix).joined(separator: "/") }
        guard Set(roots).count == roots.count else { return nil }   // 같은 폴더 안의 중복은 사본 구조가 아님
        let rel = comps[0].suffix(suffix).joined(separator: "/")
        return (roots.sorted(), rel)
    }
}
