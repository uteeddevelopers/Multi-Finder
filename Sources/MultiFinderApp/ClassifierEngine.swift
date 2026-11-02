import Foundation
import MultiFinderCore

#if canImport(FoundationModels)
import FoundationModels
#endif

/// 온디바이스 모델(Apple Intelligence)로 파일을 사용자의 묶음 중 하나에 넣는다.
/// 파일 내용은 기기 밖으로 나가지 않는다. 모델이 없거나 꺼져 있으면 `availability`가 이유를 알려준다.
enum ClassifierEngine {
    enum Availability: Equatable {
        case available
        case unavailable(String)
    }

    struct Result {
        let category: String
        let confidence: Double
        let summary: String
    }

    static var availability: Availability {
        #if canImport(FoundationModels)
        if #available(macOS 26, *) {
            switch SystemLanguageModel.default.availability {
            case .available: return .available
            case .unavailable(let reason):
                switch reason {
                case .deviceNotEligible: return .unavailable(String(localized: "이 Mac은 Apple Intelligence를 지원하지 않습니다"))
                case .appleIntelligenceNotEnabled: return .unavailable(String(localized: "시스템 설정 › Apple Intelligence 및 Siri에서 Apple Intelligence를 켜 주세요"))
                case .modelNotReady: return .unavailable(String(localized: "모델을 내려받는 중입니다. 잠시 뒤 다시 시도하세요"))
                @unknown default: return .unavailable(String(localized: "온디바이스 모델을 지금 쓸 수 없습니다"))
                }
            }
        }
        return .unavailable(String(localized: "macOS 26 이상이 필요합니다"))
        #else
        return .unavailable(String(localized: "이 빌드에는 Foundation Models가 없습니다"))
        #endif
    }

    #if canImport(FoundationModels)
    @available(macOS 26, *)
    @Generable(description: "The result of assigning one file to one bucket")
    struct Verdict {
        @Guide(description: "Exactly one bucket name, copied verbatim from the given list. Never invent a name.")
        var category: String
        @Guide(description: "Confidence from 0.0 to 1.0")
        var confidence: Double
        @Guide(description: "One short line (under 40 characters) saying what this file is, in the language the instructions ask for")
        var summary: String
    }
    #endif

    /// 앱이 지금 보여 주는 언어 (ko·en·ja·zh-Hans) — 요약을 이 언어로 쓰게 한다.
    static var appLanguage: String { Bundle.main.preferredLocalizations.first ?? "en" }

    /// 한 파일을 분류한다. 모델이 목록 밖 이름을 내면 가장 비슷한 묶음으로, 없으면 '기타'로.
    static func classify(
        record: FileRecord,
        text: String?,
        categories: [Bucket]
    ) async throws -> Result {
        #if canImport(FoundationModels)
        guard #available(macOS 26, *) else { throw ClassifierError.unavailable(String(localized: "macOS 26 이상이 필요합니다")) }
        let names = categories.map(\.name)
        let fallback = categories.fallbackName
        let prompt = Prompt.make(record: record, text: text, categories: categories, fallback: fallback, language: appLanguage)
        let session = LanguageModelSession(instructions: prompt.instructions)
        let response = try await session.respond(to: prompt.request, generating: Verdict.self)
        let v = response.content
        let category = names.contains(v.category) ? v.category : closest(v.category, in: names) ?? fallback
        return Result(category: category, confidence: min(max(v.confidence, 0), 1), summary: String(v.summary.prefix(60)))
        #else
        throw ClassifierError.unavailable(String(localized: "이 빌드에는 Foundation Models가 없습니다"))
        #endif
    }

    /// 모델에게 주는 지시와 질문. 한국어 화면은 지금까지 써 온 한국어 지시를 그대로, 그 밖의 언어는 영어 지시 + 요약 언어 지정.
    struct Prompt {
        let instructions: String
        let request: String

        static func make(record: FileRecord, text: String?, categories: [Bucket], fallback: String, language: String) -> Prompt {
            let list = categories.map { "- \($0.name): \($0.hint)" }.joined(separator: "\n")
            let folder = record.parent.replacingOccurrences(of: NSHomeDirectory(), with: "~")
            let excerpt = text.flatMap { $0.isEmpty ? nil : $0 }
            if language.hasPrefix("ko") {
                let instructions = """
                당신은 개인용 Mac의 파일 정리 도우미다. 파일 하나의 정보(이름, 들어 있는 폴더, 내용 일부)를 보고
                아래 묶음 중 정확히 하나를 고른다. 반드시 목록에 있는 이름을 그대로 쓴다. 애매하면 '\(fallback)'.
                파일명과 폴더 이름이 내용보다 더 분명한 단서일 때가 많다. 요약은 한국어 한 줄로 쓴다.

                묶음 목록:
                \(list)
                """
                var request = "파일명: \(record.name)\n폴더: \(folder)\n형식: \(record.ext.isEmpty ? "없음" : record.ext)\n"
                request += excerpt.map { "내용 일부:\n\($0)\n" } ?? "내용: (읽을 수 없는 형식 — 이름과 폴더로 판단)\n"
                return Prompt(instructions: instructions, request: request)
            }
            let languageName = Locale(identifier: "en").localizedString(forIdentifier: language) ?? "English"
            let instructions = """
            You help organize files on a personal Mac. Given one file's name, the folder it is in and part of its content,
            choose exactly one bucket from the list below. Copy the bucket name exactly as written. If unsure, choose '\(fallback)'.
            The file name and folder name are often clearer clues than the content. Write the summary as one line in \(languageName).

            Buckets:
            \(list)
            """
            var request = "File name: \(record.name)\nFolder: \(folder)\nType: \(record.ext.isEmpty ? "none" : record.ext)\n"
            request += excerpt.map { "Content excerpt:\n\($0)\n" } ?? "Content: (unreadable format — judge by the name and folder)\n"
            return Prompt(instructions: instructions, request: request)
        }
    }

    /// 목록 밖 이름이 나왔을 때 — 포함 관계로만 가볍게 맞춘다 (추측은 하지 않음).
    private static func closest(_ name: String, in names: [String]) -> String? {
        let n = name.lowercased()
        return names.first { $0.lowercased().contains(n) || n.contains($0.lowercased()) }
    }

    enum ClassifierError: LocalizedError {
        case unavailable(String)
        var errorDescription: String? {
            switch self { case .unavailable(let m): return m }
        }
    }
}
