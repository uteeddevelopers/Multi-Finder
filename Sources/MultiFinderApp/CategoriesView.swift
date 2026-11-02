import MultiFinderCore
import SwiftUI

/// '자동 분류' — 파일 내용을 읽어 사용자의 묶음에 배정해 보여준다. 파일은 제자리, 옮기는 건 고른 뒤에만.
struct CategoriesView: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        @Bindable var cat = app.categories
        HSplitView {
            categoryList
                .frame(minWidth: 180, idealWidth: 210, maxWidth: 280, maxHeight: .infinity)
            fileList
                .frame(minWidth: 400, maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbar { toolbar }
        .sheet(isPresented: $cat.showEditor) {
            CategoryEditor(categories: cat.categories) { cat.saveCategories($0) }
        }
    }

    // MARK: - 툴바

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        @Bindable var cat = app.categories
        ToolbarItemGroup(placement: .primaryAction) {
            Picker("폴더", selection: $cat.scope) {
                ForEach(app.folders, id: \.self) { f in
                    Text((f as NSString).lastPathComponent).tag(f)
                }
            }
            .pickerStyle(.menu)
            .onChange(of: cat.scope) { _, _ in cat.reload() }
            .help("어느 폴더의 파일을 분류할지")
            Toggle(isOn: $cat.includeSubfolders) { Label("하위 폴더 포함", systemImage: "arrow.down.right.and.arrow.up.left") }
                .help(cat.includeSubfolders ? "하위 폴더의 파일까지 분류 (오래 걸림)" : "폴더 바로 아래 파일만 분류 — 켜면 하위 폴더까지")
            Button { cat.showEditor = true } label: { Label("묶음 편집", systemImage: "tag") }
                .help("묶음 이름과 설명 고치기 — 설명이 좋을수록 분류가 정확해집니다")
            if cat.running {
                Button("중지") { cat.cancel() }
            } else {
                Menu {
                    Button("새 파일만 분류") { cat.run() }
                    Button("전부 다시 분류 (직접 고친 건 유지)") { cat.reclassifyAll() }
                } label: {
                    Label("분류", systemImage: "sparkles")
                } primaryAction: {
                    cat.run()
                }
                .disabled(cat.availability != .available)
            }
        }
    }

    // MARK: - 왼쪽: 묶음

    /// '전체' 줄의 선택 값. 목록 선택에서 nil은 '아무것도 안 고름'이라 nil로 표시하면 그 줄을 누를 수 없다 → 따로 값을 둔다.
    private static let allTag = "\u{1}전체"

    private var categoryList: some View {
        let cat = app.categories
        let counts = cat.counts
        let selection = Binding<String?>(
            get: { cat.selectedCategory ?? Self.allTag },
            set: { cat.selectedCategory = ($0 == nil || $0 == Self.allTag) ? nil : $0 })
        return List(selection: selection) {
            Section {
                HStack {
                    Label("전체", systemImage: "tray.full")
                    Spacer()
                    Text("\(cat.results.count)").foregroundStyle(.secondary).monospacedDigit()
                }
                .tag(Self.allTag)
            }
            Section("묶음") {
                ForEach(cat.categories) { c in
                    HStack {
                        Label(c.name, systemImage: c.isFallback ? "questionmark.folder" : "folder.badge.gearshape")
                        Spacer()
                        Text("\(counts[c.name] ?? 0)").foregroundStyle(.secondary).monospacedDigit()
                    }
                    .tag(c.name)
                    .help(c.hint)
                    .dropDestination(for: DraggedPaths.self) { items, _ in
                        cat.move(paths: Set(items.flatMap(\.paths)), to: c.name)
                        return true
                    }
                }
            }
            if !cat.running, cat.pendingCount > 0 {
                Section {
                    Label("아직 분류 안 된 파일 \(cat.pendingCount)개", systemImage: "clock")
                        .foregroundStyle(.secondary)
                        .selectionDisabled()
                }
            }
        }
        .listStyle(.sidebar)
    }

    // MARK: - 오른쪽: 파일

    @ViewBuilder
    private var fileList: some View {
        @Bindable var cat = app.categories
        VStack(spacing: 0) {
            if case .unavailable(let reason) = cat.availability {
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                    Text("온디바이스 모델을 쓸 수 없습니다 — \(reason)")
                    Spacer()
                    Button("다시 확인") { cat.availability = ClassifierEngine.availability }.controlSize(.small)
                }
                .font(.callout)
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(.quaternary.opacity(0.5))
                Divider()
            }
            if cat.running {
                HStack(spacing: 10) {
                    ProgressView(value: Double(cat.progressDone), total: Double(max(cat.progressTotal, 1)))
                        .frame(maxWidth: 240)
                    Text("\(cat.progressDone)/\(cat.progressTotal) · \(cat.currentFile)")
                        .font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    Spacer()
                }
                .padding(.horizontal, 12).padding(.vertical, 6)
                Divider()
            }
            if cat.results.isEmpty && !cat.running {
                ContentUnavailableView {
                    Label("아직 분류된 파일이 없습니다", systemImage: "sparkles")
                } description: {
                    Text("‘분류’를 누르면 \((cat.scope as NSString).lastPathComponent) 폴더의 파일을 읽어 묶음에 넣습니다.\n내용은 이 Mac 안의 모델로만 읽고 밖으로 보내지 않습니다. 파일은 그대로 두고, 옮기는 건 고른 뒤에만.")
                } actions: {
                    Button("분류 시작") { cat.run() }.disabled(cat.availability != .available)
                }
            } else {
                Table(cat.visible, selection: $cat.selection) {
                    TableColumn("파일") { c in
                        let rec = app.rootRecord(c.path)
                        HStack(spacing: 6) {
                            if let rec { Image(nsImage: IconCache.icon(for: rec)).resizable().frame(width: 16, height: 16) }
                            VStack(alignment: .leading, spacing: 1) {
                                Text((c.path as NSString).lastPathComponent).lineLimit(1).truncationMode(.middle)
                                Text(((c.path as NSString).deletingLastPathComponent).replacingOccurrences(of: cat.scope, with: "").trimmingCharacters(in: CharacterSet(charactersIn: "/")))
                                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                        .draggable(DraggedPaths(paths: cat.selection.contains(c.path) ? cat.selection.sorted() : [c.path]))
                        .help(c.path)
                    }
                    TableColumn("묶음 (열 제목)") { c in
                        Menu {
                            ForEach(cat.categories) { k in
                                Button(k.name) { cat.move(paths: cat.selection.contains(c.path) ? cat.selection : [c.path], to: k.name) }
                            }
                        } label: {
                            HStack(spacing: 4) {
                                Text(c.category)
                                if c.manual { Image(systemName: "hand.raised.fill").font(.caption2).foregroundStyle(.secondary) }
                            }
                        }
                        .menuStyle(.borderlessButton)
                        .fixedSize()
                        .help(c.manual ? "직접 고른 묶음" : "모델이 고른 묶음 — 클릭해서 바꾸기")
                    }
                    .width(min: 110, ideal: 130)
                    TableColumn("확신") { c in
                        ConfidenceDots(value: c.confidence)
                    }
                    .width(70)
                    TableColumn("한 줄 요약") { c in
                        Text(c.summary).foregroundStyle(.secondary).lineLimit(1).help(c.summary)
                    }
                }
                .contextMenu(forSelectionType: String.self) { paths in
                    if !paths.isEmpty {
                        Button("열기") { for p in paths { app.open(p) } }
                        Button("Finder에서 보기") { NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) }) }
                        Menu("묶음 바꾸기") {
                            ForEach(cat.categories) { k in Button(k.name) { cat.move(paths: paths, to: k.name) } }
                        }
                        Divider()
                        Button("폴더로 이동…") { cat.selection = paths; cat.moveSelectionToFolder() }
                        Button("휴지통으로 이동…", role: .destructive) { cat.selection = paths; cat.trashSelection() }
                    }
                } primaryAction: { paths in
                    for p in paths { app.open(p) }
                }
                Divider()
                HStack(spacing: 10) {
                    if let sel = cat.selectedCategory {
                        Text("‘\(sel)’ \(cat.visible.count)개").foregroundStyle(.secondary)
                        Button("이 묶음 전부 선택") { cat.selection = Set(cat.visible.map(\.path)) }
                    } else {
                        Text("\(cat.results.count)개 분류됨").foregroundStyle(.secondary)
                    }
                    Spacer()
                    if !cat.selection.isEmpty {
                        Text("\(cat.selection.count)개 선택").foregroundStyle(.secondary)
                        Button("폴더로 이동…") { cat.moveSelectionToFolder() }
                        Button("휴지통으로…", role: .destructive) { cat.trashSelection() }
                    }
                }
                .font(.callout)
                .padding(.horizontal, 16).padding(.vertical, 8)
                .background(.bar)
            }
        }
    }
}

/// 확신 정도를 점 다섯 개로. 숫자보다 한눈에 들어온다.
private struct ConfidenceDots: View {
    let value: Double
    var body: some View {
        HStack(spacing: 2) {
            ForEach(0..<5, id: \.self) { i in
                Circle()
                    .fill(Double(i) < value * 5 - 0.01 ? Color.accentColor : Color.secondary.opacity(0.25))
                    .frame(width: 6, height: 6)
            }
        }
        .help(String(localized: "확신 \(value.formatted(.percent.precision(.fractionLength(0))))", comment: "Tooltip: how confident the model was, e.g. 'Confidence 80%'"))
    }
}

/// 묶음 이름·설명 편집. 설명은 모델에게 주는 힌트라 구체적일수록 좋다.
private struct CategoryEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var categories: [Bucket]
    let onSave: ([Bucket]) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("묶음 편집").font(.title3).fontWeight(.semibold)
            Text("설명은 모델이 파일을 배정할 때 보는 기준입니다. 어떤 파일이 여기 들어가야 하는지 구체적으로 적을수록 정확해집니다.")
                .font(.callout).foregroundStyle(.secondary)
            List {
                ForEach($categories) { $c in
                    HStack(alignment: .top, spacing: 10) {
                        TextField("이름", text: $c.name).frame(width: 140)
                        TextField("설명 (어떤 파일이 여기 속하는지)", text: $c.hint, axis: .vertical).lineLimit(1...3)
                        Button { categories.removeAll { $0.id == c.id } } label: { Image(systemName: "minus.circle") }
                            .buttonStyle(.borderless)
                            .disabled(c.isFallback)
                    }
                    .padding(.vertical, 2)
                }
                .onMove { from, to in categories.move(fromOffsets: from, toOffset: to) }
            }
            .frame(minHeight: 280)
            HStack {
                Button { categories.insert(Bucket(name: String(localized: "새 묶음"), hint: ""), at: max(categories.count - 1, 0)) } label: {
                    Label("묶음 추가", systemImage: "plus")
                }
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("저장") { onSave(categories); dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 640)
    }
}
