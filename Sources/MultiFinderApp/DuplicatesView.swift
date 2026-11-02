import MultiFinderCore
import SwiftUI

/// '같은 내용' — 내용이 완전히 같은 파일들을 묶어 보여주고, 어느 쪽을 치울지 사람이 고른다.
struct DuplicatesView: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        @Bindable var dup = app.duplicates
        VStack(spacing: 0) {
            if dup.running || dup.waitingForScan {
                VStack(spacing: 10) {
                    ProgressView()
                    Text(dup.progress).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let report = dup.report {
                if report.fileGroups.isEmpty && report.copyTrees.isEmpty {
                    ContentUnavailableView {
                        Label("정리할 중복이 없습니다", systemImage: "checkmark.circle")
                    } description: {
                        Text(hiddenSummary(report))
                    }
                } else {
                    resultList(report)
                }
            } else {
                ContentUnavailableView {
                    Label("중복 파일 찾기", systemImage: "doc.on.doc")
                } description: {
                    Text("이름이나 위치가 달라도 내용이 완전히 같으면 중복으로 봅니다.\n찾기만 할 뿐, 지우는 건 직접 고른 뒤 확인을 거칩니다.")
                } actions: {
                    Button("찾기") { dup.run() }.keyboardShortcut(.defaultAction)
                }
            }
        }
        .toolbar { toolbar }
    }

    // MARK: - 툴바 (조건은 본문이 아니라 툴바에 — Finder·Mail의 관례)

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        @Bindable var dup = app.duplicates
        ToolbarItemGroup(placement: .primaryAction) {
            Picker("범위", selection: $dup.scope) {
                Text("전체 인덱스").tag(String?.none)
                Divider()
                ForEach(app.folders, id: \.self) { s in
                    let name = (s as NSString).lastPathComponent
                    Text("\(name) 안", comment: "Duplicates scope menu: only groups that include files inside this folder").tag(String?.some(s))
                }
            }
            .pickerStyle(.menu)
            .help("어느 폴더의 파일이 포함된 중복을 볼지")
            Picker("크기", selection: $dup.minSize) {
                ForEach(SizeFloor.allCases) { Text($0.label).tag($0) }
            }
            .pickerStyle(.menu)
            .help("이보다 작은 파일은 무시")
            Menu {
                Toggle("같은 프로젝트 안의 중복 숨기기", isOn: $dup.options.hideWithinProject)
                Toggle("폴더끼리 겹치면 폴더 단위로 묶기", isOn: $dup.options.foldCopyTrees)
                if (dup.report?.hiddenByIgnoreList ?? 0) > 0 {
                    Divider()
                    Button("무시 목록 비우기") { dup.clearIgnores() }
                }
            } label: {
                Label("옵션", systemImage: "slider.horizontal.3")
            }
            Button(dup.report == nil ? "찾기" : "다시 찾기") { dup.run() }
                .disabled(dup.running || dup.waitingForScan)
                .help("현재 범위·크기 조건으로 중복 찾기")
        }
    }

    // MARK: - (옛 상단 조건 — 툴바로 이동)

    // MARK: - 결과

    private func resultList(_ report: DuplicateReport) -> some View {
        @Bindable var dup = app.duplicates
        return VStack(spacing: 0) {
            summaryBar(report)
            Divider()
            List(selection: $dup.selection) {
                if !report.copyTrees.isEmpty {
                    Section("폴더끼리 겹침 — ‘비교’를 누르면 폴더 단위로 정리할 수 있습니다") {
                        ForEach(report.copyTrees) { tree in
                            CopyTreeRow(tree: tree)
                        }
                    }
                }
                if !report.fileGroups.isEmpty {
                    Section("똑같은 파일 \(report.fileGroups.count)묶음 — 남길 것만 빼고 고른 뒤 휴지통으로") {
                        ForEach(report.fileGroups) { g in
                            DuplicateGroupRows(group: g)
                        }
                    }
                }
            }
            .listStyle(.inset)
            .alternatingRowBackgrounds()
            .contextMenu(forSelectionType: String.self) { paths in
                if !paths.isEmpty {
                    Button("Finder에서 보기") {
                        NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) })
                    }
                    Button("열기") { for p in paths { app.open(p) } }
                    Divider()
                    Button("선택 항목 휴지통으로…", role: .destructive) { dup.selection = paths; dup.trashSelection() }
                }
            } primaryAction: { paths in
                if let p = paths.first { app.open(p) }
            }
            Divider()
            actionBar(report)
        }
    }

    private func summaryBar(_ report: DuplicateReport) -> some View {
        HStack(spacing: 16) {
            Label {
                Text("똑같은 파일을 하나씩만 남기면 **\(ByteCountFormatter.string(fromByteCount: report.totalWastedBytes, countStyle: .file))** 가 비워집니다")
            } icon: {
                Image(systemName: "internaldrive")
            }
            Text(hiddenSummary(report)).foregroundStyle(.secondary)
            Spacer()
            if app.duplicates.rawGroupCount > 0 {
                Text("전체 \(app.duplicates.rawGroupCount)묶음 중 \(report.fileGroups.count + report.copyTrees.reduce(0) { $0 + $1.groupCount })묶음 표시")
                    .foregroundStyle(.secondary)
            }
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    private func hiddenSummary(_ report: DuplicateReport) -> String {
        var parts: [String] = []
        if report.hiddenWithinProject > 0 { parts.append(String(localized: "프로젝트 내부 \(report.hiddenWithinProject)묶음 숨김")) }
        if report.hiddenByIgnoreList > 0 { parts.append(String(localized: "무시 목록 \(report.hiddenByIgnoreList)묶음")) }
        return parts.isEmpty ? "" : parts.joined(separator: " · ")
    }

    private func actionBar(_ report: DuplicateReport) -> some View {
        let dup = app.duplicates
        return HStack(spacing: 10) {
            Menu {
                Button("각 묶음에서 첫 번째만 남기고 선택") { dup.selectAllButFirst() }
                Divider()
                ForEach(app.folders, id: \.self) { s in
                    let name = (s as NSString).lastPathComponent
                    Button("\(name) 쪽 사본 선택") { dup.selectUnder(s) }
                }
                Divider()
                Button("선택 해제") { dup.selection.removeAll() }
            } label: {
                Label("빠른 선택", systemImage: "checklist")
            }
            .fixedSize()
            Spacer()
            let files = dup.selectedFiles
            if !files.isEmpty {
                Text("\(files.count)개 선택 · \(ByteCountFormatter.string(fromByteCount: files.reduce(0) { $0 + $1.size }, countStyle: .file))")
                    .foregroundStyle(.secondary)
                Button("Finder에서 보기") {
                    NSWorkspace.shared.activateFileViewerSelecting(files.map(\.url))
                }
            }
            Button("선택 항목 휴지통으로…", role: .destructive) { dup.trashSelection() }
                .disabled(files.isEmpty)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
    }
}

/// 묶음 하나: 헤더 줄 + 파일별 줄. 파일 줄을 골라 휴지통으로 보낸다.
///
/// 펼침 그룹(DisclosureGroup)을 쓰지 않고 줄을 나란히 둔다 — 펼침 그룹 안에서는 '헤더만 선택 불가'를 걸면
/// 안쪽 파일 줄까지 선택이 막히는 문제가 있어서, 헤더를 독립된 줄로 두고 그 줄에만 선택 불가를 건다.
/// 묶음을 통째로 고르게 하지 않는 이유: 전부 휴지통으로 보내면 원본까지 사라진다.
private struct DuplicateGroupRows: View {
    @Environment(AppModel.self) private var app
    let group: DuplicateGroup
    var indent: CGFloat = 0

    var body: some View {
        HStack {
            Image(systemName: "doc.on.doc").foregroundStyle(.secondary).frame(width: 16)
            Text(group.files[0].name).fontWeight(.medium).lineLimit(1)
            Text("× \(group.files.count)").foregroundStyle(.secondary)
            Spacer()
            Text(ByteCountFormatter.string(fromByteCount: group.size, countStyle: .file))
                .foregroundStyle(.secondary).monospacedDigit()
            Menu {
                Button("이 묶음은 중복으로 보지 않기") { app.duplicates.ignore(group: group) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
        .padding(.leading, indent)
        .selectionDisabled()
        ForEach(group.files) { f in
            HStack(spacing: 6) {
                Image(nsImage: IconCache.icon(for: f)).resizable().frame(width: 16, height: 16)
                VStack(alignment: .leading, spacing: 1) {
                    Text(f.name).lineLimit(1)
                    Text(folderDisplay(f.parent))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer()
                Text(f.modified, format: .dateTime.year().month(.twoDigits).day(.twoDigits))
                    .foregroundStyle(.secondary).monospacedDigit()
            }
            .padding(.leading, indent + 22)
            .tag(f.path)
            .help(f.path)
        }
    }

    private func folderDisplay(_ parent: String) -> String {
        parent.replacingOccurrences(of: NSHomeDirectory(), with: "~")
    }
}

/// "폴더 A ≈ 폴더 B" 한 줄. 누르면 그 아래 겹치는 파일 묶음이 펼쳐진다. '비교' 버튼으로 폴더 비교 화면으로 간다.
private struct CopyTreeRow: View {
    @Environment(AppModel.self) private var app
    let tree: CopyTreeGroup

    var body: some View {
        let dup = app.duplicates
        let isExpanded = dup.expandedTrees.contains(tree.id)
        let toggle = {
            if isExpanded { dup.expandedTrees.remove(tree.id) } else { dup.expandedTrees.insert(tree.id) }
        }
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Button(action: toggle) {
                Image(systemName: "chevron.right")
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    .foregroundStyle(.secondary)
                    .frame(width: 12)
            }
            .buttonStyle(.plain)
            .help(isExpanded ? "접기" : "겹치는 파일 펼쳐 보기")
            Image(nsImage: IconCache.folderIcon).resizable().frame(width: 16, height: 16)
            VStack(alignment: .leading, spacing: 2) {
                Text(tree.roots.map { $0.replacingOccurrences(of: NSHomeDirectory(), with: "~") }.joined(separator: "   ≈   "))
                    .lineLimit(2)
                    .truncationMode(.middle)
                Text("\(tree.groupCount)개 파일이 양쪽에 똑같이 있음 · 예: \(tree.samples.joined(separator: ", "))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Text(ByteCountFormatter.string(fromByteCount: tree.wastedBytes, countStyle: .file))
                .foregroundStyle(.secondary).monospacedDigit()
            if tree.roots.count == 2 {
                Button("비교") {
                    app.compare.set(side: .left, path: tree.roots[0])
                    app.compare.set(side: .right, path: tree.roots[1])
                    app.screen = .compare
                    app.compare.run()
                }
                .controlSize(.small)
            }
            Menu {
                Button("이 폴더들 사이의 중복은 무시") { app.duplicates.ignore(tree: tree) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: toggle)
        .selectionDisabled()
        if isExpanded {
            ForEach(tree.groups) { g in
                DuplicateGroupRows(group: g, indent: 20)
            }
        }
    }
}
