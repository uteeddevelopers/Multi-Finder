import Foundation
import MultiFinderCore

/// 파일 형식 분류. 내용 기반 분류(단계 3)와 별개로, 확장자만 보는 '보기 필터'.
enum FileCategory: String, CaseIterable, Identifiable {
    case all, document, image, video, audio, archive, installer, code, other
    var id: String { rawValue }

    var label: String {
        switch self {
        case .all: return String(localized: "모든 형식")
        case .document: return String(localized: "문서")
        case .image: return String(localized: "이미지")
        case .video: return String(localized: "영상")
        case .audio: return String(localized: "음악")
        case .archive: return String(localized: "압축")
        case .installer: return String(localized: "설치 파일")
        case .code: return String(localized: "코드")
        case .other: return String(localized: "기타")
        }
    }

    static let extensions: [FileCategory: Set<String>] = [
        .document: ["pdf", "doc", "docx", "ppt", "pptx", "key", "xls", "xlsx", "numbers", "pages", "hwp", "hwpx", "txt", "md", "rtf", "csv", "epub"],
        .image: ["jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tiff", "tif", "bmp", "svg", "psd", "ai", "raw", "dng"],
        .video: ["mp4", "mov", "mkv", "avi", "m4v", "webm", "wmv"],
        .audio: ["mp3", "m4a", "wav", "aac", "flac", "aiff", "ogg"],
        .archive: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz"],
        .installer: ["dmg", "pkg", "iso", "mpkg", "xip"],
        .code: ["swift", "js", "ts", "tsx", "jsx", "py", "java", "kt", "c", "cpp", "h", "m", "go", "rs", "rb", "php", "html", "css", "scss", "json", "yaml", "yml", "toml", "sh", "sql"],
    ]

    static func of(_ record: FileRecord) -> FileCategory {
        guard record.kind == .file else { return .other }
        for (cat, exts) in extensions where exts.contains(record.ext) { return cat }
        return .other
    }
}

enum SizeFloor: Int, CaseIterable, Identifiable {
    case any = 0, mb1 = 1, mb10 = 10, mb100 = 100, gb1 = 1024
    var id: Int { rawValue }
    var label: String {
        switch self {
        case .any: return String(localized: "모든 크기")
        case .mb1: return String(localized: "1 MB 이상")
        case .mb10: return String(localized: "10 MB 이상")
        case .mb100: return String(localized: "100 MB 이상")
        case .gb1: return String(localized: "1 GB 이상")
        }
    }
    var bytes: Int64 { Int64(rawValue) * 1024 * 1024 }
}

enum RecentRange: Int, CaseIterable, Identifiable {
    case any = 0, today = 1, week = 7, month = 30, year = 365
    var id: Int { rawValue }
    var label: String {
        switch self {
        case .any: return String(localized: "모든 기간")
        case .today: return String(localized: "오늘")
        case .week: return String(localized: "최근 7일")
        case .month: return String(localized: "최근 30일")
        case .year: return String(localized: "최근 1년")
        }
    }
}

struct FileFilter: Equatable {
    var text = ""
    var category: FileCategory = .all
    var sizeFloor: SizeFloor = .any
    var recent: RecentRange = .any

    var isActive: Bool { !text.isEmpty || category != .all || sizeFloor != .any || recent != .any }

    /// 폴더는 탐색을 위해 항상 남기고(검색어가 있을 때만 이름으로 거름), 파일만 거른다.
    func matches(_ r: FileRecord) -> Bool {
        if !text.isEmpty && !r.name.localizedCaseInsensitiveContains(text) { return false }
        if r.kind == .directory { return true }
        if category != .all && FileCategory.of(r) != category { return false }
        if r.size < sizeFloor.bytes { return false }
        if recent != .any {
            let cutoff = Calendar.current.date(byAdding: .day, value: -recent.rawValue, to: Calendar.current.startOfDay(for: Date()))!
            if r.modified < cutoff { return false }
        }
        return true
    }
}
