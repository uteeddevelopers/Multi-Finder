import MultiFinderCore
import Observation
import SwiftUI

@Observable
@MainActor
final class LeftoversModel {
    unowned let app: AppModel
    var groups: [LeftoverGroup] = []
    var running = false
    var progress = ""
    var scanned = false
    /// 앱 묶음 식별자 또는 항목 경로. **앱 줄을 고르면 그 앱의 흔적 전부**, 펼쳐서 항목 줄을 고르면 그것만.
    var selection: Set<String> = []
    var expandedGroups: Set<String> = []
    /// 다른 화면에서 파일이 바뀌어 결과가 낡음 — 이 화면으로 돌아오면 다시 검사한다
    var isStale = false

    init(app: AppModel) { self.app = app }

    func run() {
        guard !running else { return }
        running = true
        isStale = false
        app.hasFullDiskAccess = Executor.hasFullDiskAccess
        Task.detached(priority: .userInitiated) { [weak self] in
            let found = LeftoverFinder().find { step in
                let text = String(localized: "\(step) 확인 중…", comment: "Leftover scan progress, e.g. 'Checking installed apps…'")
                Task { @MainActor in self?.progress = text }
            }
            await MainActor.run { [weak self] in
                guard let self else { return }
                self.groups = found
                self.running = false
                self.scanned = true
                self.progress = ""
                self.selection.removeAll()
            }
        }
    }

    // MARK: - 선택 해석

    /// 선택이 가리키는 실제 항목들 (앱 줄 선택은 그 앱의 항목 전부로 펼침)
    var selectedItems: [LeftoverItem] {
        var out: [LeftoverItem] = []
        var seen = Set<String>()
        for g in groups {
            let whole = selection.contains(g.identifier)
            for item in g.items where (whole || selection.contains(item.path)) && seen.insert(item.path).inserted {
                out.append(item)
            }
        }
        return out
    }

    var selectedAppCount: Int {
        groups.filter { g in selection.contains(g.identifier) || g.items.contains { selection.contains($0.path) } }.count
    }

    var lockedTotal: Int { groups.reduce(0) { $0 + $1.lockedCount } }

    func selectCertain() {
        selection = Set(groups.filter { $0.confidence == .certain && !$0.isFullyLocked }.map(\.identifier))
    }

    func toggleExpanded(_ id: String) {
        if expandedGroups.contains(id) { expandedGroups.remove(id) } else { expandedGroups.insert(id) }
    }

    func reveal(_ ids: Set<String>) {
        let paths = itemPaths(for: ids)
        NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) })
    }

    func itemPaths(for ids: Set<String>) -> [String] {
        groups.flatMap { g in g.items.filter { ids.contains(g.identifier) || ids.contains($0.path) }.map(\.path) }
    }

    /// 보호된 항목은 시도하지 않고 건너뛴다 — 시도해도 macOS가 막아 실패만 쌓이므로.
    func trashSelection() {
        let items = selectedItems
        let unlocked = items.filter { !$0.locked }
        let lockedCount = items.count - unlocked.count
        guard !unlocked.isEmpty else {
            if lockedCount > 0 {
                app.showError(
                    title: String(localized: "macOS가 보호하는 폴더입니다"),
                    message: String(localized: "고른 \(lockedCount)개는 다른 앱의 데이터 폴더(Containers·Group Containers)라 지금은 지울 수 없습니다.\n\n시스템 설정 › 개인정보 보호 및 보안 › 전체 디스크 접근에서 Multi-Finder를 켠 뒤 ‘다시 검사’하면 지울 수 있습니다."),
                    action: .openFullDiskAccessSettings)
            }
            return
        }
        app.requestTrash(
            unlocked.map(\.path),
            knownBytes: unlocked.reduce(0) { $0 + $1.bytes },
            note: lockedCount > 0 ? String(localized: "보호된 \(lockedCount)개는 건너뜁니다 (전체 디스크 접근 필요).") : nil)
    }
}

/// '앱 잔여물' — 지운 앱이 ~/Library에 남긴 폴더를 찾아 보여준다. 확실한 것과 확인이 필요한 것을 나눈다.
struct LeftoversView: View {
    @Environment(AppModel.self) private var app

    static var protectedNote: String {
        String(localized: "macOS가 보호하는 다른 앱의 데이터 — 전체 디스크 접근이 있어야 지울 수 있음")
    }

    var body: some View {
        @Bindable var lo = app.leftovers
        VStack(spacing: 0) {
            if lo.running {
                VStack(spacing: 10) {
                    ProgressView()
                    Text(lo.progress.isEmpty ? String(localized: "검사 중…") : lo.progress).foregroundStyle(.secondary).lineLimit(1)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if !lo.scanned {
                ContentUnavailableView {
                    Label("지운 앱의 흔적 찾기", systemImage: "trash.slash")
                } description: {
                    Text("앱을 휴지통에 넣어도 설정·캐시·저장 상태는 ~/Library에 남습니다.\n이 Mac의 앱과 대조해 주인 없는 것만 찾고, 애매하면 남깁니다. 지우는 건 고른 뒤 확인을 거칩니다.")
                } actions: {
                    Button("검사") { lo.run() }.keyboardShortcut(.defaultAction)
                }
            } else if lo.groups.isEmpty {
                ContentUnavailableView("남은 흔적이 없습니다", systemImage: "checkmark.circle", description: Text("지운 앱의 잔여물을 찾지 못했습니다."))
            } else {
                if lo.lockedTotal > 0 && !app.hasFullDiskAccess { ProtectionBanner() }
                list
                Divider()
                footer
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button(lo.scanned ? "다시 검사" : "검사") { lo.run() }.disabled(lo.running)
            }
        }
    }

    private var list: some View {
        @Bindable var lo = app.leftovers
        let certain = lo.groups.filter { $0.confidence == .certain }
        let likely = lo.groups.filter { $0.confidence == .likely }
        return List(selection: $lo.selection) {
            if !certain.isEmpty {
                Section {
                    ForEach(certain) { GroupRows(group: $0) }
                } header: {
                    Text("지운 앱의 흔적 — 이 Mac에 해당 앱이 없고, 앱마다 생기는 폴더가 남아 있음 (\(certain.count)개 앱)")
                }
            }
            if !likely.isEmpty {
                Section {
                    ForEach(likely) { GroupRows(group: $0) }
                } header: {
                    Text("확인 필요 — 설정·캐시만 남았거나 이름으로만 판단, 다른 앱이 쓰는 것일 수 있음 (\(likely.count)개)")
                }
            }
        }
        .listStyle(.inset)
        .alternatingRowBackgrounds()
        .contextMenu(forSelectionType: String.self) { ids in
            if !ids.isEmpty {
                Button("Finder에서 보기") { lo.reveal(ids) }
                Divider()
                Button("휴지통으로 이동…", role: .destructive) { lo.selection = ids; lo.trashSelection() }
            }
        } primaryAction: { ids in
            // 앱 줄 두 번 클릭: 펼치기/접기, 항목 줄 두 번 클릭: Finder에서 보기 (Finder 목록 보기와 같은 동작)
            if ids.count == 1, let id = ids.first, lo.groups.contains(where: { $0.identifier == id }) {
                lo.toggleExpanded(id)
            } else {
                lo.reveal(ids)
            }
        }
    }

    private var footer: some View {
        let lo = app.leftovers
        let items = lo.selectedItems
        let locked = items.filter(\.locked).count
        let bytes = items.reduce(Int64(0)) { $0 + $1.bytes }
        return HStack(spacing: 10) {
            Button("지운 앱 흔적 모두 선택") { lo.selectCertain() }
                .help("위 목록(확실한 것) 중 지울 수 있는 앱을 모두 고릅니다")
            if !lo.selection.isEmpty {
                Button("선택 해제") { lo.selection.removeAll() }
            }
            Spacer()
            if !items.isEmpty {
                let size = ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
                let summary = String(localized: "\(lo.selectedAppCount)개 앱 · \(items.count)개 항목 · \(size)")
                Text(summary + (locked > 0 ? " · " + String(localized: "보호됨 \(locked)") : ""))
                    .foregroundStyle(.secondary)
            }
            Button("휴지통으로…", role: .destructive) { lo.trashSelection() }
                .disabled(items.isEmpty)
        }
        .font(.callout)
        .padding(.horizontal, 16).padding(.vertical, 8)
        .background(.bar)
    }
}

/// 다른 앱의 데이터 폴더가 있는데 전체 디스크 접근이 없을 때만 뜨는 안내 줄.
private struct ProtectionBanner: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "lock.shield").foregroundStyle(.secondary)
            // 높이를 글자 수에 맡기면(fixedSize) 좁은 폭에서 한없이 길어져 창 전체 배치가 밀린다 → 두 줄까지만
            Text("자물쇠 표시 항목은 macOS가 보호하는 다른 앱의 데이터라 지금은 지울 수 없습니다. 지우려면 Multi-Finder에 전체 디스크 접근을 허용하세요.")
                .lineLimit(2)
                .truncationMode(.tail)
                .layoutPriority(1)
                .help("시스템 설정 › 개인정보 보호 및 보안 › 전체 디스크 접근")
            Spacer(minLength: 8)
            Button("설정 열기") { app.openFullDiskAccessSettings() }.fixedSize()
            Button("다시 검사") { app.leftovers.run() }.fixedSize()
        }
        .font(.callout)
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(.quaternary.opacity(0.5))
        .overlay(alignment: .bottom) { Divider() }
    }
}

/// 앱 한 줄(선택하면 그 앱의 흔적 전부) + 펼치면 항목 줄(하나씩 선택).
private struct GroupRows: View {
    @Environment(AppModel.self) private var app
    let group: LeftoverGroup

    var body: some View {
        let lo = app.leftovers
        let expanded = Binding(
            get: { lo.expandedGroups.contains(group.identifier) },
            set: { on in if on { lo.expandedGroups.insert(group.identifier) } else { lo.expandedGroups.remove(group.identifier) } })
        DisclosureGroup(isExpanded: expanded) {
            ForEach(group.items) { item in
                ItemRow(item: item).tag(item.path)
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: group.confidence == .certain ? "app.dashed" : "questionmark.app")
                    .foregroundStyle(group.confidence == .certain ? Color.accentColor : .secondary)
                Text(group.identifier).fontWeight(.medium).lineLimit(1).truncationMode(.middle)
                Text("\(group.items.count)개").foregroundStyle(.secondary)
                if group.isFullyLocked {
                    Image(systemName: "lock.fill").font(.caption).foregroundStyle(.secondary)
                        .help("macOS가 보호하는 다른 앱의 데이터 — 전체 디스크 접근이 있어야 지울 수 있음")
                }
                Spacer()
                Text("마지막 변경 \(group.lastModified, format: .dateTime.year().month(.twoDigits).day(.twoDigits))")
                    .foregroundStyle(.secondary).monospacedDigit()
                Text(group.isFullyLocked ? "—" : ByteCountFormatter.string(fromByteCount: group.totalBytes, countStyle: .file))
                    .foregroundStyle(.secondary).monospacedDigit().frame(width: 80, alignment: .trailing)
            }
            .help(group.reason)
        }
        .tag(group.identifier)
    }
}

private struct ItemRow: View {
    let item: LeftoverItem

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: item.locked ? "lock.fill" : "folder").foregroundStyle(.secondary)
                .frame(width: 16)
            Text(item.area).foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
            Text((item.path as NSString).lastPathComponent).lineLimit(1).truncationMode(.middle)
            Spacer()
            Text(item.modified, format: .dateTime.year().month(.twoDigits).day(.twoDigits)).foregroundStyle(.secondary).monospacedDigit()
            Text(item.locked ? String(localized: "보호됨") : ByteCountFormatter.string(fromByteCount: item.bytes, countStyle: .file))
                .foregroundStyle(.secondary).monospacedDigit().frame(width: 80, alignment: .trailing)
        }
        .help(item.locked ? item.path + "\n" + LeftoversView.protectedNote : item.path)
    }
}
