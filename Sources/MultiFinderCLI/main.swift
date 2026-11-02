import Foundation
import MultiFinderCore

// mfinder — Multi-Finder 코어를 화면 없이 검증하기 위한 명령줄 도구.
//
//   mfinder scan [경로...] [--include-hidden]   인덱스 채우기 (기본: / 전체, 규칙에 따라 제외)
//   mfinder stats                                인덱스 요약
//   mfinder ls <폴더>                            인덱스 기준 폴더 내용
//   mfinder dupes [--min-size 1M] [--under 경로] 중복 묶음 (내용 동일)
//   mfinder trash <경로>                         휴지통으로 (승인은 명령 자체)
//   mfinder move <경로> <폴더>                   폴더 안으로 이동
//   mfinder undo                                 마지막 작업 되돌리기
//   mfinder history                              작업 기록
//   mfinder watch [경로...]                      변경 감지로 인덱스 유지 (Ctrl+C로 종료)

let args = Array(CommandLine.arguments.dropFirst())
let command = args.first ?? "help"
let rest = Array(args.dropFirst())

let formatter: ByteCountFormatter = {
    let f = ByteCountFormatter()
    f.countStyle = .file
    return f
}()

func bytes(_ n: Int64) -> String { formatter.string(fromByteCount: n) }

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

func flag(_ name: String) -> Bool { rest.contains(name) }

func option(_ name: String) -> String? {
    guard let i = rest.firstIndex(of: name), i + 1 < rest.count else { return nil }
    return rest[i + 1]
}

/// 옵션(--x, 그 값)을 뺀 위치 인자
func positionals() -> [String] {
    var out: [String] = []
    var skip = false
    for a in rest {
        if skip { skip = false; continue }
        if a.hasPrefix("--") {
            if a == "--min-size" || a == "--under" { skip = true }
            continue
        }
        out.append(a)
    }
    return out
}

func parseSize(_ s: String) -> Int64 {
    let units: [Character: Int64] = ["K": 1 << 10, "M": 1 << 20, "G": 1 << 30]
    if let last = s.uppercased().last, let mult = units[last] {
        return Int64(Double(s.dropLast()) ?? 0) * mult
    }
    return Int64(s) ?? 0
}

func absolute(_ p: String) -> String {
    let expanded = (p as NSString).expandingTildeInPath
    if expanded.hasPrefix("/") { return (expanded as NSString).standardizingPath }
    return ((FileManager.default.currentDirectoryPath as NSString).appendingPathComponent(expanded) as NSString).standardizingPath
}

let store: IndexStore
do { store = try IndexStore(path: IndexStore.defaultURL().path) } catch { fail("인덱스 열기 실패: \(error.localizedDescription)") }

var rules = ScanRules.default()
if flag("--include-hidden") { rules.skipHidden = false }

do {
switch command {
case "scan":
    let roots = positionals().isEmpty ? ["/"] : positionals().map(absolute)
    let started = Date()
    print("스캔 시작: \(roots.joined(separator: ", "))")
    var lastPrinted = Date()
    let count = try Scanner(rules: rules).scan(roots: roots.map { URL(fileURLWithPath: $0) }, into: store) { p in
        if Date().timeIntervalSince(lastPrinted) > 1 {
            lastPrinted = Date()
            print("\u{1B}[2K\r  \(p.scanned)개 … \(p.currentPath.prefix(70))", terminator: "")
            fflush(stdout)
        }
    }
    print("\u{1B}[2K\r완료: \(count)개 항목, \(String(format: "%.1f", Date().timeIntervalSince(started)))초")

case "stats":
    let s = try store.stats()
    print("파일 \(s.files)개 / 폴더 \(s.directories)개 / 총 \(bytes(s.bytes)) / 지문 보유 \(s.hashed)개")
    print("인덱스: \(IndexStore.defaultURL().path)")

case "ls":
    guard let dir = positionals().first else { fail("사용법: mfinder ls <폴더>") }
    let items = try store.children(of: absolute(dir))
    if items.isEmpty { print("(인덱스에 없음 — 먼저 scan)") }
    for r in items {
        let marker = r.kind == .directory ? "/" : r.kind == .package ? "•" : r.kind == .symlink ? "@" : " "
        let size = r.kind == .file ? bytes(r.size) : ""
        let note = r.isDataless ? " [로컬 없음]" : ""
        print("\(marker) \(r.name.padding(toLength: 50, withPad: " ", startingAt: 0)) \(size)\(note)")
    }

case "dupes":
    let minSize = option("--min-size").map(parseSize) ?? 1
    let under = option("--under").map(absolute)
    var lastPrinted = Date()
    let groups = try DuplicateFinder(store: store).find(minSize: minSize, under: under) { done, total in
        if Date().timeIntervalSince(lastPrinted) > 1 {
            lastPrinted = Date()
            print("\u{1B}[2K\r  크기 묶음 \(done)/\(total) 확인 중…", terminator: "")
            fflush(stdout)
        }
    }
    print("\u{1B}[2K\r", terminator: "")
    let wasted = groups.reduce(Int64(0)) { $0 + $1.wastedBytes }
    print("중복 묶음 \(groups.count)개, 하나씩만 남기면 \(bytes(wasted)) 회수 (APFS 클론이면 실제론 덜 돌아올 수 있음)\n")
    for g in groups.prefix(50) {
        print("■ \(bytes(g.size)) × \(g.files.count)  (\(g.hash.prefix(12)))")
        for f in g.files { print("    \(f.path)") }
    }
    if groups.count > 50 { print("… 외 \(groups.count - 50)개 묶음") }

case "trash":
    guard let p = positionals().first else { fail("사용법: mfinder trash <경로>") }
    let dest = try Executor(store: store).trash(absolute(p))
    print("휴지통으로 이동: \(dest.path)")

case "move":
    let ps = positionals()
    guard ps.count == 2 else { fail("사용법: mfinder move <경로> <폴더>") }
    let dest = try Executor(store: store).move(absolute(ps[0]), toDirectory: absolute(ps[1]))
    print("이동: \(dest)")

case "undo":
    let result = try Executor(store: store).undo()
    for op in result.restored { print("되돌림 (\(op.kind.rawValue)): \(op.destination) → \(op.source)") }
    for p in result.missing { print("찾을 수 없어 건너뜀: \(p)") }
    for p in result.blocked { print("원래 자리에 다른 항목이 있어 못 되돌림: \(p)") }
    if result.isEmpty { print("되돌릴 작업이 없음") }

case "history":
    let df = DateFormatter(); df.dateFormat = "MM-dd HH:mm"
    for op in try Executor(store: store).history() {
        print("\(op.undone ? "↩︎" : " ") #\(op.id) \(df.string(from: op.timestamp)) \(op.kind.rawValue.padding(toLength: 5, withPad: " ", startingAt: 0)) \(op.source) → \(op.destination)")
    }

case "watch":
    let roots = positionals().isEmpty ? [NSHomeDirectory()] : positionals().map(absolute)
    let watcher = Watcher(paths: roots, onChange: { changed in
        Watcher.indexUpdater(store: store, rules: rules)(changed)
        print("갱신 \(changed.count)건: \(changed.prefix(3).joined(separator: ", "))\(changed.count > 3 ? " …" : "")")
    })
    watcher.start()
    print("감시 중: \(roots.joined(separator: ", "))  (Ctrl+C로 종료)")
    dispatchMain()

default:
    print("""
    mfinder — 파일 인덱스·중복·이동 도구 (Multi-Finder 코어 검증용)

      scan [경로...] [--include-hidden]    인덱스 채우기 (기본: / 전체)
      stats                                 인덱스 요약
      ls <폴더>                             인덱스 기준 폴더 내용
      dupes [--min-size 1M] [--under 경로]  중복 묶음 (내용 동일 기준)
      trash <경로>                          휴지통으로
      move <경로> <폴더>                    폴더 안으로 이동
      undo                                  마지막 작업 되돌리기
      history                               작업 기록
      watch [경로...]                       변경 감지로 인덱스 유지
    """)
}
} catch {
    fail("실패: \(error.localizedDescription)")
}
