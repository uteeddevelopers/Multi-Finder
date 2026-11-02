import AppKit
import Foundation

/// 지워진 앱이 ~/Library 에 남긴 흔적 한 묶음(앱 하나).
public struct LeftoverGroup: Sendable, Identifiable {
    public enum Confidence: Int, Sendable { case certain = 2, likely = 1 }
    public var id: String { identifier }
    /// 번들 ID(com.x.y) 또는 폴더 이름
    public let identifier: String
    public let items: [LeftoverItem]
    public let confidence: Confidence
    /// 왜 이렇게 판정했는지 한 줄 (화면 도움말)
    public let reason: String

    public var totalBytes: Int64 { items.reduce(0) { $0 + $1.bytes } }
    public var lastModified: Date { items.map(\.modified).max() ?? .distantPast }
    public var lockedCount: Int { items.filter(\.locked).count }
    public var isFullyLocked: Bool { !items.isEmpty && items.allSatisfy(\.locked) }
}

public struct LeftoverItem: Sendable, Identifiable, Hashable {
    public var id: String { path }
    public let path: String
    /// 읽을 수 없으면(locked) 0
    public let bytes: Int64
    /// 안쪽 파일까지 포함한 가장 최근 수정 시각 (locked면 폴더 자체의 시각)
    public let modified: Date
    /// ~/Library 아래 어느 구역인지 (Caches, Preferences …)
    public let area: String
    /// macOS가 다른 앱의 데이터로 보호하는 곳이라 이 앱이 읽거나 지울 수 없음 — 전체 디스크 접근이 필요
    public let locked: Bool
}

/// 앱 잔여물 탐색. 원칙: **의심스러우면 남긴다.**
///
/// 판정 근거
/// - 설치 여부: LaunchServices(번들 ID 정확히) + Spotlight(디스크 어디든 있는 .app) + /Applications 직접 탐색.
///   같은 회사 접두어(com.회사)의 앱이 하나라도 있으면 그 회사 것은 건드리지 않는다.
/// - 확실(certain): 해당 앱이 없고, **macOS가 앱마다 따로 만드는 폴더**(Containers, Saved Application State 등)가 남아 있거나
///   두 군데 이상에 흔적이 있음. 설정 파일(Preferences)이나 캐시만 남은 경우는 다른 앱·라이브러리가 만든 것일 수 있어 '확인 필요'.
/// - 제외: Apple·macOS 시스템 이름(com.apple, 단축어의 is.workflow, 프린터의 org.cups …), 다른 앱 안에서 쓰는 라이브러리 이름
///   (com.segment, Kingfisher, PLCrashReporter …), 최근에 바뀐 것(확실 30일·확인 필요 90일 — 지워진 앱은 파일을 쓰지 못한다).
public final class LeftoverFinder {
    public struct Options: Sendable {
        public var recentDaysCertain = 30
        public var recentDaysLikely = 90
        public init() {}
    }

    private let fm = FileManager.default
    private let home = NSHomeDirectory()

    public init() {}

    /// macOS가 앱(또는 앱 확장) 단위로만 만드는 구역 — 여기에 번들 ID 폴더가 있으면 '앱이 있었다'는 강한 증거
    static let strongAreas: Set<String> = [
        "Containers", "Group Containers", "Saved Application State", "Application Scripts",
        "HTTPStorages", "WebKit", "Cookies",
    ]
    /// 이름이 곧 번들 ID인 구역 (강한 구역 + 설정 + 로그인 항목).
    /// LaunchAgents는 앱 없이 도는 도구(Homebrew 서비스 등)도 쓰므로 강한 증거가 아니고, 가리키는 프로그램이 살아 있으면 아예 뺀다.
    static let bundleIDAreas: [String] = Array(strongAreas).sorted() + ["Preferences", "LaunchAgents"]
    /// 이름 또는 번들 ID가 섞여 쓰이는 구역
    static let mixedAreas: [String] = ["Application Support", "Caches", "Logs"]

    public func find(options: Options = Options(), progress: ((String) -> Void)? = nil) -> [LeftoverGroup] {
        progress?(String(localized: "설치된 앱", comment: "Leftover scan progress: what is being checked"))
        let installed = InstalledApps.collect(home: home)
        let now = Date()
        let cutoffCertain = Calendar.current.date(byAdding: .day, value: -options.recentDaysCertain, to: now)!
        let cutoffLikely = Calendar.current.date(byAdding: .day, value: -options.recentDaysLikely, to: now)!

        // 1) 후보 모으기 (아직 크기·시각은 보지 않음 — 가볍게)
        var byID: [String: [(path: String, area: String)]] = [:]
        var byName: [String: [(path: String, area: String)]] = [:]

        for area in Self.bundleIDAreas + Self.mixedAreas {
            let dir = "\(home)/Library/\(area)"
            progress?(area)
            guard let names = try? fm.contentsOfDirectory(atPath: dir) else { continue }
            for raw in names where !raw.hasPrefix(".") {
                let path = "\(dir)/\(raw)"
                var name = raw
                for suffix in [".plist", ".savedState", ".binarycookies"] where name.hasSuffix(suffix) {
                    name = String(name.dropLast(suffix.count))
                }
                if Self.isExcludedIdentifier(name) { continue }
                if area == "LaunchAgents", Self.launchAgentIsLive(path) { continue }

                if Self.looksLikeBundleID(name) {
                    let bid = name.hasPrefix("group.") ? String(name.dropFirst(6)) : name
                    if installed.owns(bundleID: bid) { continue }
                    byID[bid, default: []].append((path, area))
                } else {
                    // 이름형은 섞인 구역의 '폴더'만 본다 — 낱개 파일(default.store 등)은 주인을 알 수 없어 건드리지 않음
                    guard Self.mixedAreas.contains(area) else { continue }
                    var isDir: ObjCBool = false
                    guard fm.fileExists(atPath: path, isDirectory: &isDir), isDir.boolValue else { continue }
                    if Self.systemNames.contains(name) || name.count < 4 { continue }
                    if installed.matches(name: name) { continue }
                    byName[name, default: []].append((path, area))
                }
            }
        }

        // 2) 후보마다 실제 상태(크기·가장 최근 수정·보호 여부)를 보고 판정
        var groups: [LeftoverGroup] = []
        for (bid, entries) in byID {
            progress?(bid)
            let items = entries.map { inspect(path: $0.path, area: $0.area) }
            let areas = Set(items.map(\.area))
            let strong = !areas.isDisjoint(with: Self.strongAreas) || areas.count >= 2
            let cutoff = strong ? cutoffCertain : cutoffLikely
            // 최근에 바뀐 게 있으면 누군가 아직 쓰고 있다 → 통째로 제외
            // (보호돼서 안을 못 보는 폴더는 폴더 자체의 수정 시각이 유일한 근거라 그것으로 판단)
            if items.contains(where: { $0.modified > cutoff }) { continue }
            let areaList = areas.sorted().joined(separator: ", ")
            groups.append(LeftoverGroup(
                identifier: bid,
                items: items.sorted { $0.path < $1.path },
                confidence: strong ? .certain : .likely,
                reason: strong
                    ? String(localized: "이 번들 ID의 앱이 이 Mac에 없고, macOS가 앱마다 만드는 폴더(\(areaList))가 남아 있음")
                    : String(localized: "이 번들 ID의 앱은 없지만 \(areaList)에만 있어 다른 앱이나 라이브러리가 만든 것일 수 있음")))
        }
        for (name, entries) in byName {
            progress?(name)
            let items = entries.map { inspect(path: $0.path, area: $0.area) }
            if items.contains(where: { $0.modified > cutoffLikely }) { continue }
            groups.append(LeftoverGroup(
                identifier: name,
                items: items.sorted { $0.path < $1.path },
                confidence: .likely,
                reason: String(localized: "이름만으로 판단 — 같은 이름의 앱을 찾지 못했고 \(options.recentDaysLikely)일 넘게 바뀌지 않음")))
        }
        return groups.sorted { ($0.confidence.rawValue, $0.totalBytes) > ($1.confidence.rawValue, $1.totalBytes) }
    }

    // MARK: - 항목 상태

    /// 크기, 안쪽까지 본 가장 최근 수정 시각, 보호 여부.
    func inspect(path: String, area: String) -> LeftoverItem {
        let ownDate = (try? fm.attributesOfItem(atPath: path)[.modificationDate] as? Date) ?? .distantPast
        var isDir: ObjCBool = false
        _ = fm.fileExists(atPath: path, isDirectory: &isDir)
        guard isDir.boolValue else {
            let size = (try? fm.attributesOfItem(atPath: path)[.size] as? Int64) ?? 0
            return LeftoverItem(path: path, bytes: size, modified: ownDate, area: area, locked: false)
        }
        // 다른 앱의 데이터 폴더는 macOS가 막는다 → 목록조차 못 읽으면 '보호됨'
        do { _ = try fm.contentsOfDirectory(atPath: path) } catch {
            return LeftoverItem(path: path, bytes: 0, modified: ownDate, area: area, locked: Executor.isPermissionError(error))
        }
        var total: Int64 = 0
        var newest = ownDate
        var count = 0
        let keys: [URLResourceKey] = [.fileSizeKey, .contentModificationDateKey]
        if let en = fm.enumerator(at: URL(fileURLWithPath: path), includingPropertiesForKeys: keys, options: [], errorHandler: { _, _ in true }) {
            while let u = en.nextObject() as? URL {
                count += 1
                if count > 200_000 { break }   // 아주 큰 폴더는 표본으로 충분
                guard let v = try? u.resourceValues(forKeys: Set(keys)) else { continue }
                total += Int64(v.fileSize ?? 0)
                if let d = v.contentModificationDate, d > newest { newest = d }
            }
        }
        return LeftoverItem(path: path, bytes: total, modified: newest, area: area, locked: false)
    }

    /// 로그인 항목(LaunchAgent)이 가리키는 프로그램이 아직 있으면 살아 있는 것. 읽을 수 없거나 경로가 아니면 판단하지 않고 남긴다.
    static func launchAgentIsLive(_ plistPath: String) -> Bool {
        guard let dict = NSDictionary(contentsOfFile: plistPath) as? [String: Any] else { return true }
        let program = (dict["Program"] as? String) ?? (dict["ProgramArguments"] as? [String])?.first
        guard let program, program.hasPrefix("/") else { return true }
        return FileManager.default.fileExists(atPath: program)
    }

    // MARK: - 이름 규칙 (테스트 대상)

    /// 번들 ID처럼 생겼는가 (com.회사.앱, group.com.회사.앱 …)
    public static func looksLikeBundleID(_ s: String) -> Bool {
        let comps = s.split(separator: ".")
        guard comps.count >= 3, !s.contains(" ") else { return false }
        return ["com", "org", "net", "io", "app", "dev", "co", "kr", "jp", "us", "uk", "de", "fr", "group", "me", "tv", "ai", "so", "xyz", "sh", "cc", "is", "edu"]
            .contains(String(comps[0]).lowercased())
    }

    /// 절대 잔여물로 보지 않는 이름 — Apple·macOS 시스템, 다른 앱 안에서 쓰는 라이브러리(SDK)
    public static func isExcludedIdentifier(_ s: String) -> Bool {
        let l = s.lowercased()
        if l == "com.apple" { return true }
        return excludedPrefixes.contains { l.hasPrefix($0) }
    }

    static let excludedPrefixes: [String] = [
        // Apple · macOS (단축어는 옛 이름 Workflow의 is.workflow.*, 프린터는 org.cups.*)
        "com.apple.", "group.com.apple.", "is.workflow.", "group.is.workflow.", "org.cups.", "edu.mit.", "org.openldap.", "org.ntp.",
        "systemgroup.", "com.apple",
        // 다른 앱 안에 들어 있는 라이브러리 이름 — 앱이 아니라서 '지운 앱'의 근거가 될 수 없음
        "com.segment.", "com.onevcat.", "com.plausiblelabs.", "io.sentry", "com.crashlytics", "io.fabric", "com.firebase",
        "com.google.firebase", "com.amplitude", "com.mixpanel", "com.bugsnag", "io.realm", "com.squareup", "org.sparkle-project.",
        "com.github.electron", "org.chromium.", "org.webkit.", "com.microsoft.appcenter", "com.adjust.", "com.appsflyer",
        "com.facebook.sdk", "com.braze", "io.branch", "com.datadoghq", "com.newrelic", "com.launchdarkly", "com.revenuecat",
        "com.instabug", "com.pspdfkit", "net.hockeyapp", "org.alamofire", "com.alamofire", "com.sdwebimage", "com.rs.sdwebimage",
    ].map { $0.lowercased() }

    /// 이름형 구역에서 Apple·공용으로 알려진 폴더 — 절대 후보로 올리지 않음
    static let systemNames: Set<String> = [
        "AddressBook", "Accounts", "AccessibilityBundles", "Animoji", "App Store", "AppStoreComponents", "Apple", "AppleMediaServices",
        "CallHistoryDB", "CallHistoryTransactions", "CallServices", "CloudDocs", "CloudKit", "CrashReporter", "DifferentialPrivacy",
        "DiskImages", "Dock", "FaceTime", "Family", "FileProvider", "GameKit", "iCloud", "IdentityServices", "iLifeMediaBrowser",
        "Knowledge", "MobileSync", "networkserviceproxy", "Quick Look", "SESStorage", "SyncServices", "TrustedPeersHelper", "Ubiquity",
        "icdd", "iMovie", "Keynote", "Numbers", "Pages", "Script Editor", "Spotlight", "Metadata", "Mail", "Messages", "Notes",
        "Photos", "Safari", "Siri", "Xcode", "Developer", "Logs", "Caches", "Preferences", "homed", "locationaccessstored",
        "DiagnosticReports", "DiagnosticReportsForNewHardware", "CoreAnalytics", "PersonalizationPortrait", "Biome",
        // 여러 앱이 같이 쓰거나 제조사 공용 폴더
        "Firefox", "Google", "Mozilla", "Microsoft", "Adobe", "JetBrains", "Code", "Cursor", "zoom.us", "Homebrew", "Steam",
        "MultiFinder", "Shelf", "Claude", "Figma", "Notion", "Slack", "Discord", "KakaoTalk", "Telegram",
    ]
}

/// 이 Mac에 설치된 앱(과 그 안의 확장·로그인 항목)의 번들 ID·회사 접두어·이름.
struct InstalledApps {
    var bundleIDs: Set<String> = []
    var prefixes: Set<String> = []
    var names: [String] = []

    /// 이 번들 ID(또는 같은 회사)의 앱이 설치되어 있는가
    func owns(bundleID bid: String) -> Bool {
        let l = bid.lowercased()
        if bundleIDs.contains(l) { return true }
        if NSWorkspace.shared.urlForApplication(withBundleIdentifier: bid) != nil { return true }
        if let p = Self.companyPrefix(l), prefixes.contains(p) { return true }
        // 번들 ID 마지막 부분이 설치된 앱 이름과 겹치면(헬퍼·확장 등) 남긴다
        let last = Self.normalize((bid as NSString).pathExtension)
        if last.count >= 4, names.contains(where: { !$0.isEmpty && ($0.contains(last) || last.contains($0)) }) { return true }
        return false
    }

    /// 폴더 이름이 설치된 앱 이름과 겹치는가
    func matches(name: String) -> Bool {
        let n = Self.normalize(name)
        guard !n.isEmpty else { return true }
        return names.contains { !$0.isEmpty && ($0.contains(n) || n.contains($0)) }
    }

    static func collect(home: String) -> InstalledApps {
        var out = InstalledApps()
        var appPaths = Set<String>()
        // ① Spotlight — 디스크 어디에 있든 (~/Library 안의 도우미 앱 포함). 휴지통 속 앱은 설치로 치지 않음
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/mdfind")
        p.arguments = ["kMDItemContentType == 'com.apple.application-bundle'"]
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = FileHandle.nullDevice
        if (try? p.run()) != nil {
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            p.waitUntilExit()
            for line in String(decoding: data, as: UTF8.self).split(separator: "\n") where !line.contains("/.Trash/") {
                appPaths.insert(String(line))
            }
        }
        // ② 직접 탐색 — Spotlight가 꺼져 있어도 기본 위치는 본다
        let fm = FileManager.default
        for root in ["/Applications", "/Applications/Utilities", "\(home)/Applications", "/System/Applications", "/System/Applications/Utilities"] {
            guard let en = fm.enumerator(at: URL(fileURLWithPath: root), includingPropertiesForKeys: nil, options: [.skipsHiddenFiles, .skipsPackageDescendants]) else { continue }
            while let url = en.nextObject() as? URL {
                if en.level > 3 { en.skipDescendants(); continue }
                if url.pathExtension == "app" { appPaths.insert(url.path) }
            }
        }
        for path in appPaths {
            out.add(bundleAt: URL(fileURLWithPath: path))
            // 앱 안의 확장(.appex)·로그인 항목도 같은 앱의 일부
            let contents = URL(fileURLWithPath: path).appendingPathComponent("Contents")
            for sub in ["PlugIns", "Library/LoginItems", "Library/LaunchServices", "Extensions"] {
                if let items = try? fm.contentsOfDirectory(at: contents.appendingPathComponent(sub), includingPropertiesForKeys: nil) {
                    for item in items where ["appex", "app", "systemextension"].contains(item.pathExtension) { out.add(bundleAt: item) }
                }
            }
        }
        return out
    }

    private mutating func add(bundleAt url: URL) {
        guard let bundle = Bundle(url: url), let bid = bundle.bundleIdentifier?.lowercased() else { return }
        bundleIDs.insert(bid)
        if let p = Self.companyPrefix(bid) { prefixes.insert(p) }
        let info = bundle.infoDictionary ?? [:]
        for n in [info["CFBundleDisplayName"] as? String, info["CFBundleName"] as? String, url.deletingPathExtension().lastPathComponent] {
            if let n { names.append(Self.normalize(n)) }
        }
    }

    /// com.google.chrome → com.google (단, 너무 넓은 접두어는 쓰지 않음)
    static func companyPrefix(_ bid: String) -> String? {
        let comps = bid.split(separator: ".")
        guard comps.count >= 2 else { return nil }
        let prefix = comps.prefix(2).joined(separator: ".")
        // 개인·공용 도메인류(com.github, io.github …)는 회사가 아니라 여러 개발자가 공유하므로 접두어로 묶지 않음
        if ["com.github", "io.github", "com.electron", "org.electron", "com.example", "com.company", "com.mycompany"].contains(prefix) { return nil }
        return prefix
    }

    static func normalize(_ s: String) -> String {
        s.lowercased().replacingOccurrences(of: "[^a-z0-9가-힣]", with: "", options: .regularExpression)
    }
}
