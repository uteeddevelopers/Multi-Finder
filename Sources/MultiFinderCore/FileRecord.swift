import Foundation

/// 인덱스에 들어가는 항목의 종류.
public enum FileKind: Int, Sendable, Codable {
    case file = 0
    case directory = 1
    case symlink = 2
    /// .app 같은 번들. 하나의 항목으로만 기록하고 내부는 스캔하지 않음.
    case package = 3
}

/// 디스크 위 파일 하나에 대한 인덱스 레코드.
/// 모든 뷰(멀티 폴더 / 중복 / 분류)는 이 레코드 목록을 다르게 보여주는 것뿐이다.
public struct FileRecord: Sendable, Hashable, Identifiable {
    public var id: String { path }

    public var path: String
    public var name: String
    public var parent: String
    /// 소문자 확장자. 없으면 빈 문자열.
    public var ext: String
    public var size: Int64
    public var modified: Date
    public var inode: UInt64
    public var kind: FileKind
    /// iCloud '최적화 저장' 등으로 내용이 로컬에 없는 파일. 지문을 뜨면 다운로드가 시작되므로 건드리지 않는다.
    public var isDataless: Bool
    /// SHA-256 (hex). 중복 후보가 되어 계산한 뒤에만 채워진다.
    public var hash: String?

    public init(
        path: String, name: String, parent: String, ext: String,
        size: Int64, modified: Date, inode: UInt64, kind: FileKind,
        isDataless: Bool, hash: String? = nil
    ) {
        self.path = path
        self.name = name
        self.parent = parent
        self.ext = ext
        self.size = size
        self.modified = modified
        self.inode = inode
        self.kind = kind
        self.isDataless = isDataless
        self.hash = hash
    }

    public var url: URL { URL(fileURLWithPath: path) }
}
