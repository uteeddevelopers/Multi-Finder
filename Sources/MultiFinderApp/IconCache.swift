import AppKit
import MultiFinderCore
import UniformTypeIdentifiers

/// Finder와 같은 시스템 아이콘. 확장자별로 한 번만 만든다.
@MainActor
enum IconCache {
    private static var byExtension: [String: NSImage] = [:]
    private static let folder = NSWorkspace.shared.icon(for: .folder)
    private static let package = NSWorkspace.shared.icon(for: .applicationBundle)
    private static let link = NSWorkspace.shared.icon(for: .symbolicLink)

    static var folderIcon: NSImage { folder }

    static func icon(for record: FileRecord) -> NSImage {
        switch record.kind {
        case .directory: return folder
        case .package: return package
        case .symlink: return link
        case .file:
            if let cached = byExtension[record.ext] { return cached }
            let type = UTType(filenameExtension: record.ext) ?? .data
            let image = NSWorkspace.shared.icon(for: type)
            byExtension[record.ext] = image
            return image
        }
    }
}
