import Foundation
import CryptoKit

/// 파일 내용 지문. 같은 지문 = 같은 바이트열 = 같은 파일.
public enum ContentHasher {
    /// 전체 내용 SHA-256 (hex).
    public static func sha256(of url: URL, bufferSize: Int = 1 << 20) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while true {
            let chunk = try autoreleasepool { () -> Data? in
                let data = try handle.read(upToCount: bufferSize)
                return (data?.isEmpty ?? true) ? nil : data
            }
            guard let chunk else { break }
            hasher.update(data: chunk)
        }
        return hex(hasher.finalize())
    }

    /// 크기 + 앞 64KB + 뒤 64KB로 만든 빠른 지문. 중복 후보를 걸러내는 1차 필터로만 쓴다.
    /// 같은 quickHash라도 전체 내용이 다를 수 있으므로 최종 판정은 반드시 `sha256`으로.
    public static func quickHash(of url: URL, size: Int64, sample: Int = 64 * 1024) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        var sizeLE = size.littleEndian
        hasher.update(data: Data(bytes: &sizeLE, count: MemoryLayout<Int64>.size))
        if let head = try handle.read(upToCount: sample) { hasher.update(data: head) }
        if size > Int64(sample * 2) {
            try handle.seek(toOffset: UInt64(size - Int64(sample)))
            if let tail = try handle.read(upToCount: sample) { hasher.update(data: tail) }
        }
        return hex(hasher.finalize())
    }

    private static func hex(_ digest: SHA256Digest) -> String {
        digest.map { String(format: "%02x", $0) }.joined()
    }
}
