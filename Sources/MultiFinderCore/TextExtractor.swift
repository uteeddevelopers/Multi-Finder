import Foundation
import PDFKit
import Vision
import AppKit

/// 파일에서 분류에 쓸 짧은 글을 뽑는다. 온디바이스 모델은 한 번에 읽을 수 있는 양이 적으므로
/// 앞부분 위주로 `limit`자만 돌려준다. 못 읽는 형식은 nil (파일명·위치만으로 분류).
public enum TextExtractor {
    public static let textExtensions: Set<String> = ["txt", "md", "markdown", "csv", "tsv", "json", "yaml", "yml", "log", "rtf", "html", "htm", "xml", "tex"]
    public static let codeExtensions: Set<String> = ["swift", "js", "ts", "tsx", "jsx", "py", "java", "kt", "c", "cpp", "h", "m", "go", "rs", "rb", "php", "css", "scss", "sh", "sql"]
    public static let officeZipExtensions: Set<String> = ["docx", "pptx", "xlsx", "hwpx", "key", "pages", "numbers"]
    public static let imageExtensions: Set<String> = ["png", "jpg", "jpeg", "heic", "heif", "tiff", "tif", "webp", "gif", "bmp"]

    /// 이 형식은 내용을 읽어 분류할 수 있는가
    public static func canExtract(ext: String) -> Bool {
        ext == "pdf" || textExtensions.contains(ext) || codeExtensions.contains(ext)
            || officeZipExtensions.contains(ext) || imageExtensions.contains(ext) || ext == "doc"
    }

    public static func extract(from url: URL, ext: String, limit: Int = 1500) -> String? {
        let raw: String?
        switch ext {
        case "pdf": raw = pdfText(url, pages: 3)
        case "doc", "rtf":
            raw = (try? NSAttributedString(url: url, options: [:], documentAttributes: nil))?.string
        case _ where textExtensions.contains(ext) || codeExtensions.contains(ext):
            raw = headText(url)
        case _ where officeZipExtensions.contains(ext): raw = officeText(url, ext: ext)
        case _ where imageExtensions.contains(ext): raw = ocrText(url)
        default: raw = nil
        }
        guard var text = raw?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else { return nil }
        text = text.replacingOccurrences(of: "[ \\t]+", with: " ", options: .regularExpression)
            .replacingOccurrences(of: "\\n{3,}", with: "\n\n", options: .regularExpression)
        if text.count > limit { text = String(text.prefix(limit)) + "…" }
        return text
    }

    // MARK: - 형식별

    /// 글 파일의 앞부분만 읽는다 — 수 GB짜리 로그·CSV를 통째로 메모리에 올리지 않도록.
    static func headText(_ url: URL, bytes: Int = 64 * 1024) -> String? {
        guard let h = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? h.close() }
        guard let data = try? h.read(upToCount: bytes), !data.isEmpty else { return nil }
        if data.starts(with: [0xFF, 0xFE]) || data.starts(with: [0xFE, 0xFF]) {
            return String(data: data, encoding: .utf16)
        }
        // 잘린 끝의 깨진 UTF-8은 대체 문자로 — 나머지는 그대로 읽힌다
        return String(decoding: data, as: UTF8.self)
    }

    static func pdfText(_ url: URL, pages: Int) -> String? {
        guard let doc = PDFDocument(url: url) else { return nil }
        var out = ""
        for i in 0..<min(pages, doc.pageCount) {
            if let s = doc.page(at: i)?.string { out += s + "\n" }
            if out.count > 4000 { break }
        }
        // 글자가 없는 PDF(스캔본)는 첫 페이지를 그림으로 OCR
        if out.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, let page = doc.page(at: 0) {
            let image = page.thumbnail(of: CGSize(width: 1200, height: 1600), for: .mediaBox)
            if let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) { return ocrText(cgImage: cg) }
        }
        return out
    }

    /// docx/pptx/xlsx 등은 zip 안의 XML. 시스템 unzip으로 본문 XML만 꺼내 태그를 벗긴다.
    static func officeText(_ url: URL, ext: String) -> String? {
        let members: [String]
        switch ext {
        case "docx": members = ["word/document.xml"]
        case "pptx": members = ["ppt/slides/slide1.xml", "ppt/slides/slide2.xml", "ppt/slides/slide3.xml"]
        case "xlsx": members = ["xl/sharedStrings.xml"]
        case "hwpx": members = ["Contents/section0.xml"]
        case "key", "pages", "numbers": members = ["Index/Document.iwa"]   // 바이너리라 거의 못 읽음 → 메타데이터로 대체
        default: return nil
        }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        p.arguments = ["-p", url.path] + members
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = FileHandle.nullDevice
        do { try p.run() } catch { return nil }
        let data = pipe.fileHandleForReading.readData(ofLength: 400_000)
        // 필요한 만큼 읽었으면 끝낸다 — 남은 출력이 파이프를 채우면 unzip이 멈춰 영원히 기다리게 된다
        if p.isRunning { p.terminate() }
        p.waitUntilExit()
        guard !data.isEmpty else { return nil }
        let xml = String(decoding: data, as: UTF8.self)   // 중간에 잘렸어도 앞부분은 읽힌다
        // 태그 경계에 공백을 넣어 단어가 붙지 않게
        let stripped = xml.replacingOccurrences(of: "</[^>]+>", with: " ", options: .regularExpression)
            .replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
            .replacingOccurrences(of: "&amp;", with: "&").replacingOccurrences(of: "&lt;", with: "<")
            .replacingOccurrences(of: "&gt;", with: ">").replacingOccurrences(of: "&quot;", with: "\"")
        return stripped
    }

    static func ocrText(_ url: URL) -> String? {
        guard let image = NSImage(contentsOf: url),
              let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return nil }
        return ocrText(cgImage: cg)
    }

    static func ocrText(cgImage: CGImage) -> String? {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.recognitionLanguages = ["ko-KR", "en-US"]
        request.usesLanguageCorrection = true
        let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
        guard (try? handler.perform([request])) != nil else { return nil }
        let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }
}
