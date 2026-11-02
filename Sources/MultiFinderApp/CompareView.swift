import MultiFinderCore
import SwiftUI

/// '폴더 비교' — 두 폴더를 좌우에 놓고 내용 기준으로 맞춰 본 뒤, 한쪽으로 몰거나 겹치는 쪽을 치운다.
struct CompareView: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        @Bindable var cmp = app.compare
        VStack(spacing: 0) {
            header
            Divider()
            if cmp.running {
                VStack(spacing: 10) {
                    ProgressView()
                    Text(cmp.progress).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let result = cmp.result {
                table(result)
                Divider()
                footer(result)
            } else {
                ContentUnavailableView {
                    Label("폴더 두 개를 고르세요", systemImage: "rectangle.split.2x1")
                } description: {
                    Text("같은 자리의 파일끼리 맞춰 보고, 한쪽에만 있는 파일은 반대쪽 다른 자리에 같은 내용이 있는지도 찾습니다.")
                } actions: {
                    Button("비교") { cmp.run() }.disabled(!cmp.canRun).keyboardShortcut(.defaultAction)
                }
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                if cmp.result != nil {
                    Picker("보기", selection: $cmp.statusFilter) {
                        Text("전체").tag(DiffStatus?.none)
                        ForEach(DiffStatus.allCases, id: \.self) { s in
                            Label(s.label, systemImage: s.symbol).tag(DiffStatus?.some(s))
                        }
                    }
                    .pickerStyle(.menu)
                    .help("상태별로 보기")
                }
                Button { cmp.run() } label: { Label("비교", systemImage: "arrow.triangle.2.circlepath") }
                    .disabled(!cmp.canRun)
                    .help("다시 비교")
            }
        }
    }

    // MARK: - 좌우 선택

    private var header: some View {
        let cmp = app.compare
        return HStack(spacing: 12) {
            FolderPickerButton(placeholder: "왼쪽 폴더 선택…", path: cmp.left) { cmp.pick(side: .left) }
            Button { cmp.swap() } label: { Image(systemName: "arrow.left.arrow.right") }
                .buttonStyle(.borderless)
                .help("좌우 바꾸기")
                .disabled(cmp.left == nil && cmp.right == nil)
            FolderPickerButton(placeholder: "오른쪽 폴더 선택…", path: cmp.right) { cmp.pick(side: .right) }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    // MARK: - 표

    private func table(_ r: FolderDiffResult) -> some View {
        @Bindable var cmp = app.compare
        return Table(cmp.visibleEntries, selection: $cmp.selection) {
            TableColumn("이름") { e in
                HStack(spacing: 6) {
                    if let rec = e.left ?? e.right {
                        Image(nsImage: IconCache.icon(for: rec)).resizable().frame(width: 16, height: 16)
                    }
                    VStack(alignment: .leading, spacing: 1) {
                        Text((e.relativePath as NSString).lastPathComponent).lineLimit(1).truncationMode(.middle)
                        let dir = (e.relativePath as NSString).deletingLastPathComponent
                        if !dir.isEmpty {
                            Text(dir).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                        }
                    }
                }
                .help((e.left?.path ?? e.right?.path ?? ""))
            }
            TableColumn("상태") { e in
                Label(e.status.label, systemImage: e.status.symbol)
                    .foregroundStyle(e.status.color)
                    .help(e.status.explanation)
            }
            .width(min: 110, ideal: 130, max: 160)
            TableColumn((r.left as NSString).lastPathComponent) { e in
                sideCell(e.left, relocatedTo: e.status == .relocated && e.left == nil ? e.counterpart : nil, root: r.left)
            }
            TableColumn((r.right as NSString).lastPathComponent) { e in
                sideCell(e.right, relocatedTo: e.status == .relocated && e.right == nil ? e.counterpart : nil, root: r.right)
            }
        }
        .contextMenu(forSelectionType: String.self) { ids in
            if !ids.isEmpty {
                let rows = r.entries.filter { ids.contains($0.id) }
                Button("Finder에서 보기") {
                    let paths = rows.flatMap { [$0.left?.path, $0.right?.path].compactMap { $0 } }
                    NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) })
                }
                Divider()
                // 양쪽에 다 있는 줄이 있으니 어느 쪽 파일을 지울지 분명히 고르게 한다
                Button("왼쪽 파일 휴지통으로…", role: .destructive) { app.compare.selection = ids; app.compare.trashSelected(side: .left) }
                    .disabled(!rows.contains { $0.left != nil })
                Button("오른쪽 파일 휴지통으로…", role: .destructive) { app.compare.selection = ids; app.compare.trashSelected(side: .right) }
                    .disabled(!rows.contains { $0.right != nil })
            }
        } primaryAction: { ids in
            if let id = ids.first, let e = r.entries.first(where: { $0.id == id }), let p = e.left?.path ?? e.right?.path {
                app.open(p)
            }
        }
    }

    @ViewBuilder
    private func sideCell(_ rec: FileRecord?, relocatedTo: String?, root: String) -> some View {
        if let rec {
            HStack(spacing: 8) {
                Text(ByteCountFormatter.string(fromByteCount: rec.size, countStyle: .file)).monospacedDigit()
                Text(rec.modified, format: .dateTime.year().month(.twoDigits).day(.twoDigits)).foregroundStyle(.secondary).monospacedDigit()
            }
        } else if let relocatedTo {
            Label(relocatedTo.replacingOccurrences(of: root + "/", with: ""), systemImage: "arrow.turn.down.right")
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .help("같은 내용이 이 자리에 있음")
        } else {
            Text("없음").foregroundStyle(.quaternary)
        }
    }

    // MARK: - 요약 + 정리 메뉴

    private func footer(_ r: FolderDiffResult) -> some View {
        let cmp = app.compare
        let leftName = (r.left as NSString).lastPathComponent
        let rightName = (r.right as NSString).lastPathComponent
        return HStack(spacing: 14) {
            ForEach(DiffStatus.allCases, id: \.self) { s in
                let n = r.count(s)
                if n > 0 {
                    Button {
                        cmp.statusFilter = cmp.statusFilter == s ? nil : s
                    } label: {
                        Label("\(s.label) \(n)", systemImage: s.symbol)
                            .foregroundStyle(cmp.statusFilter == s ? .primary : s.color)
                    }
                    .buttonStyle(.borderless)
                    .help(s.explanation)
                }
            }
            Spacer()
            if !cmp.selection.isEmpty {
                Text("\(cmp.selection.count)개 선택").foregroundStyle(.secondary)
                Menu("휴지통으로…") {
                    Button("왼쪽 파일 휴지통으로…", role: .destructive) { cmp.trashSelected(side: .left) }
                        .disabled(!cmp.selectionHas(side: .left))
                    Button("오른쪽 파일 휴지통으로…", role: .destructive) { cmp.trashSelected(side: .right) }
                        .disabled(!cmp.selectionHas(side: .right))
                }
                .fixedSize()
            }
            Menu {
                Section("한쪽에만 있는 파일 옮기기 — 자리(상대 경로) 유지") {
                    Button("\(leftName)에만 있는 \(r.count(.leftOnly))개 → \(rightName)") { cmp.moveOnly(from: .left) }
                        .disabled(r.count(.leftOnly) == 0)
                    Button("\(rightName)에만 있는 \(r.count(.rightOnly))개 → \(leftName)") { cmp.moveOnly(from: .right) }
                        .disabled(r.count(.rightOnly) == 0)
                }
                Section("양쪽에 똑같이 있는 \(r.count(.identical))개 중 한쪽 치우기") {
                    Button("\(leftName) 쪽 사본 휴지통으로…") { cmp.trashIdentical(on: .left) }
                        .disabled(r.count(.identical) == 0)
                    Button("\(rightName) 쪽 사본 휴지통으로…") { cmp.trashIdentical(on: .right) }
                        .disabled(r.count(.identical) == 0)
                }
            } label: {
                Label("정리", systemImage: "wand.and.stars")
            }
            .fixedSize()
            .help("옮기기·휴지통 모두 확인을 거치고, ⌘Z로 한 번에 되돌릴 수 있습니다")
        }
        .font(.callout)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
    }
}

/// 왼쪽/오른쪽 폴더 선택 버튼 — 폴더 이름을 크게, 경로는 작게.
private struct FolderPickerButton: View {
    let placeholder: LocalizedStringKey
    let path: String?
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Image(nsImage: IconCache.folderIcon).resizable().frame(width: 28, height: 28)
                VStack(alignment: .leading, spacing: 1) {
                    if let path {
                        Text((path as NSString).lastPathComponent).fontWeight(.medium).lineLimit(1)
                        Text(((path as NSString).deletingLastPathComponent).replacingOccurrences(of: NSHomeDirectory(), with: "~"))
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    } else {
                        Text(placeholder).foregroundStyle(.secondary)
                        Text("클릭해서 고르기").font(.caption).foregroundStyle(.tertiary)
                    }
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .help(path ?? String(localized: "폴더를 고르세요"))
    }
}

extension DiffStatus {
    var symbol: String {
        switch self {
        case .identical: return "checkmark.circle.fill"
        case .modified: return "exclamationmark.triangle.fill"
        case .leftOnly: return "arrow.left.circle"
        case .rightOnly: return "arrow.right.circle"
        case .relocated: return "arrow.triangle.swap"
        }
    }

    var color: Color {
        switch self {
        case .identical: return .green
        case .modified: return .orange
        case .leftOnly, .rightOnly: return .blue
        case .relocated: return .purple
        }
    }

    var explanation: String {
        switch self {
        case .identical: return String(localized: "양쪽 같은 자리에 똑같은 내용이 있음 — 한쪽은 치워도 됨")
        case .modified: return String(localized: "같은 자리에 있지만 내용이 다름 — 어느 쪽이 최신인지 확인 필요")
        case .leftOnly: return String(localized: "왼쪽 폴더에만 있음")
        case .rightOnly: return String(localized: "오른쪽 폴더에만 있음")
        case .relocated: return String(localized: "반대쪽의 다른 자리에 같은 내용이 있음 (옮겨졌거나 이름이 다름)")
        }
    }
}
