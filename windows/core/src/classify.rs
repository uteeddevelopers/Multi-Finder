//! Categories: user buckets, stored results, the classify job and the model prompt.
//! Port of `Classification.swift`, `CategoriesModel.swift` and `ClassifierEngine.swift`.
//!
//! Files stay where they are; results are "virtual buckets". The model never sees more than
//! the file name, its folder and ~1500 characters of content, and runs locally.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;

use crate::index::IndexStore;
use crate::paths::{self, PathStyle};
use crate::progress::Cancel;
use crate::record::{FileKind, FileRecord};
use crate::text;
use crate::types::{
    Bucket, Classified, ClassifiedList, ClassifyFinishedEvent, ClassifyMode, ClassifyRequest,
    ProgressEvent,
};

/// Fallback bucket names in every UI language (buckets keep the language they were made in).
pub const FALLBACK_NAMES: &[&str] = &["기타", "Other", "その他", "其他"];
/// Installer bucket names in every UI language.
pub const INSTALLER_NAMES: &[&str] = &[
    "설치·임시",
    "Installers & Temporary",
    "インストーラ・一時ファイル",
    "安装包与临时文件",
];
/// Files put into the installer bucket without asking the model.
pub const INSTALLER_EXTENSIONS: &[&str] = &[
    "dmg",
    "pkg",
    "iso",
    "xip",
    "mpkg",
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
];

/// Characters of content given to the model.
pub const EXCERPT_LIMIT: usize = 1500;
/// Longest summary kept (the grammar allows the same).
pub const SUMMARY_MAX: usize = 60;

/// UI language from an OS locale ("ko-KR" → "ko"); anything unsupported → "en".
pub fn ui_language(locale: &str) -> &'static str {
    let l = locale.to_lowercase().replace('_', "-");
    if l.starts_with("ko") {
        "ko"
    } else if l.starts_with("ja") {
        "ja"
    } else if l.starts_with("zh") {
        let traditional =
            l.contains("hant") || l.ends_with("-tw") || l.ends_with("-hk") || l.ends_with("-mo");
        if traditional {
            "en"
        } else {
            "zh-Hans"
        }
    } else {
        "en"
    }
}

fn pick<'a>(lang: &str, ko: &'a str, en: &'a str, ja: &'a str, zh: &'a str) -> &'a str {
    match lang {
        "ko" => ko,
        "ja" => ja,
        "zh-Hans" => zh,
        _ => en,
    }
}

/// The fallback bucket in the UI language.
pub fn fallback_bucket(lang: &str) -> Bucket {
    Bucket {
        id: "other".into(),
        name: pick(lang, "기타", "Other", "その他", "其他").into(),
        hint: pick(
            lang,
            "위 어디에도 분명히 속하지 않음",
            "Doesn’t clearly belong anywhere above",
            "上のどれにもはっきり当てはまらない",
            "不明确属于以上任何一类",
        )
        .into(),
        is_fallback: true,
    }
}

/// First-run buckets in the UI language (strings from `Localizable.xcstrings`; the installer
/// hint names exe·msi instead of dmg·pkg on Windows).
pub fn default_buckets(lang: &str) -> Vec<Bucket> {
    let b = |id: &str, name: &str, hint: &str| Bucket {
        id: id.into(),
        name: name.into(),
        hint: hint.into(),
        is_fallback: false,
    };
    vec![
        b(
            "work",
            pick(lang, "업무", "Work", "仕事", "工作"),
            pick(
                lang,
                "회사·업무 문서 — 보고서, 기획서, 계약서, 회의 자료, 거래처와 주고받은 파일",
                "Job and business documents — reports, proposals, contracts, meeting materials, files exchanged with clients",
                "仕事・業務の書類 — 報告書、企画書、契約書、会議資料、取引先とやり取りしたファイル",
                "公司与业务文件——报告、方案、合同、会议资料、与客户往来的文件",
            ),
        ),
        b(
            "school",
            pick(lang, "학교·공부", "School & Study", "学校・勉強", "学校与学习"),
            pick(
                lang,
                "수업 자료, 강의 노트, 과제·레포트, 시험 자료, 학사 서류, 공부용 교재",
                "Class materials, lecture notes, assignments and papers, exam materials, school paperwork, study books",
                "授業資料、講義ノート、課題・レポート、試験資料、学務書類、勉強用の教材",
                "课程资料、课堂笔记、作业与论文、考试资料、学籍文件、学习用书",
            ),
        ),
        b(
            "personal",
            pick(lang, "개인", "Personal", "個人", "个人"),
            pick(
                lang,
                "개인 생활 — 영수증, 계약서, 여행, 취미, 건강·금융 등 사적인 문서",
                "Personal life — receipts, contracts, travel, hobbies, health and finance documents",
                "私生活 — 領収書、契約書、旅行、趣味、健康・お金など個人的な書類",
                "个人生活——收据、合同、旅行、爱好、健康与财务等私人文件",
            ),
        ),
        b(
            "dev",
            pick(lang, "개발", "Development", "開発", "开发"),
            pick(
                lang,
                "소스 코드, 프로젝트 압축본, 설정 파일, 개발 문서, 기술 자료",
                "Source code, project archives, config files, developer docs, technical references",
                "ソースコード、プロジェクトの圧縮ファイル、設定ファイル、開発ドキュメント、技術資料",
                "源代码、项目压缩包、配置文件、开发文档、技术资料",
            ),
        ),
        b(
            "media",
            pick(lang, "디자인·미디어", "Design & Media", "デザイン・メディア", "设计与媒体"),
            pick(
                lang,
                "로고, 사진, 이미지, 영상, 음악, 폰트, 디자인 시안",
                "Logos, photos, images, videos, music, fonts, design drafts",
                "ロゴ、写真、画像、動画、音楽、フォント、デザイン案",
                "标志、照片、图片、视频、音乐、字体、设计稿",
            ),
        ),
        b(
            "installers",
            pick(lang, INSTALLER_NAMES[0], INSTALLER_NAMES[1], INSTALLER_NAMES[2], INSTALLER_NAMES[3]),
            pick(
                lang,
                "exe·msi 설치 파일, 압축 해제 전 zip, 임시 캡처, 다시 받을 수 있는 것",
                "exe/msi installers, zips not yet extracted, temporary screenshots, things you can download again",
                "exe・msi のインストーラ、未展開の zip、一時的なスクリーンショット、再ダウンロードできるもの",
                "exe/msi 安装包、尚未解压的 zip、临时截图、可以重新下载的文件",
            ),
        ),
        fallback_bucket(lang),
    ]
}

fn installer_summary(lang: &str) -> &'static str {
    pick(
        lang,
        "설치 파일 — 설치했다면 지워도 됨",
        "Installer — safe to delete once installed",
        "インストーラ — インストール済みなら削除して可",
        "安装包 — 安装后可删除",
    )
}

fn failure_summary(lang: &str, reason: &str) -> String {
    let r: String = reason.chars().take(80).collect();
    match lang {
        "ko" => format!("분류 실패: {r}"),
        "ja" => format!("分類に失敗: {r}"),
        "zh-Hans" => format!("分类失败：{r}"),
        _ => format!("Couldn’t categorize: {r}"),
    }
}

/// Extensions the Categories screen looks at.
pub fn classifiable_extensions() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    v.extend_from_slice(text::TEXT_EXTENSIONS);
    v.extend_from_slice(text::CODE_EXTENSIONS);
    v.extend_from_slice(text::OFFICE_ZIP_EXTENSIONS);
    v.extend_from_slice(text::IMAGE_EXTENSIONS);
    v.extend_from_slice(INSTALLER_EXTENSIONS);
    v.extend_from_slice(&[
        "pdf", "rtf", "doc", "xls", "ppt", "hwp", "key", "pages", "numbers", "zip", "7z", "rar",
        "mp4", "mov", "mkv", "avi", "mp3", "m4a", "wav", "flac", "ai", "psd", "fig", "sketch",
        "svg", "ttf", "otf",
    ]);
    v.sort_unstable();
    v.dedup();
    v
}

// ───────────── buckets ─────────────

fn is_fallback_name(name: &str) -> bool {
    FALLBACK_NAMES.contains(&paths::nfc(name.trim()).as_str())
}

fn load_buckets(store: &IndexStore) -> Result<Vec<Bucket>> {
    let c = store.conn();
    let mut st = c.prepare("SELECT id, name, hint, is_fallback FROM buckets ORDER BY position")?;
    let rows = st.query_map([], |r| {
        Ok(Bucket {
            id: r.get(0)?,
            name: r.get(1)?,
            hint: r.get(2)?,
            is_fallback: r.get::<_, i64>(3)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn write_buckets(store: &IndexStore, list: &[Bucket]) -> Result<()> {
    let mut c = store.conn();
    let tx = c.transaction()?;
    tx.execute("DELETE FROM buckets", [])?;
    for (i, b) in list.iter().enumerate() {
        tx.execute(
            "INSERT INTO buckets(id, name, hint, is_fallback, position) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![b.id, b.name, b.hint, b.is_fallback as i64, i as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// `get_buckets`: the saved buckets; on first use the defaults in the UI language are created
/// (existing buckets are never overwritten).
pub fn get_buckets(store: &IndexStore, lang: &str) -> Result<Vec<Bucket>> {
    let list = load_buckets(store)?;
    if !list.is_empty() {
        return Ok(list);
    }
    let defaults = default_buckets(lang);
    write_buckets(store, &defaults)?;
    Ok(defaults)
}

/// Clean a bucket list from the UI: names trimmed (control characters dropped), empty and
/// duplicate names removed, ids made unique, exactly one fallback bucket, placed last.
pub fn clean_buckets(input: &[Bucket], lang: &str) -> Vec<Bucket> {
    let mut out: Vec<Bucket> = Vec::new();
    let mut fallback: Option<Bucket> = None;
    let mut n = 0;
    for b in input {
        let name: String = b
            .name
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        if name.is_empty() || out.iter().chain(fallback.iter()).any(|o| o.name == name) {
            continue;
        }
        let hint: String = b
            .hint
            .chars()
            .filter(|c| *c != '\r')
            .collect::<String>()
            .trim()
            .to_string();
        let mut id = b.id.trim().to_string();
        if id.is_empty() || out.iter().chain(fallback.iter()).any(|o| o.id == id) {
            n += 1;
            id = format!("b{}-{n}", paths::now_ms());
        }
        let wants_fallback = b.is_fallback || is_fallback_name(&name);
        let bucket = Bucket {
            id,
            name,
            hint,
            is_fallback: false,
        };
        if wants_fallback && fallback.is_none() {
            fallback = Some(Bucket {
                is_fallback: true,
                ..bucket
            });
        } else {
            out.push(bucket);
        }
    }
    let fb = fallback.unwrap_or_else(|| {
        let mut f = fallback_bucket(lang);
        if out.iter().any(|o| o.id == f.id) {
            f.id = format!("other-{}", paths::now_ms());
        }
        f
    });
    out.push(fb);
    out
}

/// `save_buckets`: store the cleaned list. Renamed buckets keep their files; files of removed
/// buckets become unclassified again (manual choices move to the fallback bucket).
pub fn save_buckets(store: &IndexStore, input: &[Bucket], lang: &str) -> Result<Vec<Bucket>> {
    let new = clean_buckets(input, lang);
    let old = load_buckets(store)?;
    let fallback_name = new.last().map(|b| b.name.clone()).unwrap_or_default();
    {
        let mut c = store.conn();
        let tx = c.transaction()?;
        // two steps so swapped names do not collide
        let mut renamed = Vec::new();
        for o in &old {
            match new.iter().find(|b| b.id == o.id) {
                Some(b) if b.name != o.name => {
                    let tmp = format!("\x01{}", o.id);
                    tx.execute(
                        "UPDATE classes SET bucket = ?1 WHERE bucket = ?2",
                        params![tmp, o.name],
                    )?;
                    renamed.push((tmp, b.name.clone()));
                }
                Some(_) => {}
                None => {
                    tx.execute(
                        "DELETE FROM classes WHERE bucket = ?1 AND manual = 0",
                        [&o.name],
                    )?;
                    tx.execute(
                        "UPDATE classes SET bucket = ?1 WHERE bucket = ?2 AND manual = 1",
                        params![fallback_name, o.name],
                    )?;
                }
            }
        }
        for (tmp, name) in renamed {
            tx.execute(
                "UPDATE classes SET bucket = ?1 WHERE bucket = ?2",
                params![name, tmp],
            )?;
        }
        tx.commit()?;
    }
    write_buckets(store, &new)?;
    Ok(new)
}

// ───────────── results ─────────────

/// SQL scope clause on alias `f` (files) with its parameters.
fn scope_clause(store: &IndexStore, scope: &str, recursive: bool) -> (String, Vec<String>) {
    if recursive {
        let (lo, hi) = store.style().descendant_range(scope);
        ("f.path >= ? AND f.path < ?".into(), vec![lo, hi])
    } else {
        ("f.parent = ?".into(), vec![scope.to_string()])
    }
}

fn ext_list_sql() -> String {
    classifiable_extensions()
        .iter()
        .map(|e| format!("'{e}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// `list_classified`: results in scope whose file is unchanged since, plus how many
/// classifiable files are still pending.
pub fn list_classified(
    store: &IndexStore,
    scope: &str,
    include_subfolders: bool,
) -> Result<ClassifiedList> {
    let (clause, args) = scope_clause(store, scope, include_subfolders);
    let c = store.conn();
    let mut st = c.prepare(&format!(
        "SELECT c.path, c.bucket, c.confidence, c.summary, c.manual
         FROM classes c JOIN files f ON f.path = c.path
         WHERE {clause} AND f.mtime = c.mtime AND f.size = c.size
         ORDER BY c.bucket, c.path"
    ))?;
    let rows = st.query_map(rusqlite::params_from_iter(args.iter()), |r| {
        Ok(Classified {
            path: r.get(0)?,
            bucket: r.get(1)?,
            confidence: r.get(2)?,
            summary: r.get(3)?,
            manual: r.get::<_, i64>(4)? != 0,
        })
    })?;
    let results: Vec<Classified> = rows.collect::<rusqlite::Result<_>>()?;
    let pending: i64 = c.query_row(
        &format!(
            "SELECT COUNT(*) FROM files f
             LEFT JOIN classes c ON c.path = f.path AND c.mtime = f.mtime AND c.size = f.size
             WHERE f.kind = 0 AND f.cloud = 0 AND {clause} AND c.path IS NULL AND f.ext IN ({})",
            ext_list_sql()
        ),
        rusqlite::params_from_iter(args.iter()),
        |r| r.get(0),
    )?;
    Ok(ClassifiedList {
        results,
        pending: pending.max(0) as u64,
    })
}

/// Files in scope not classified yet (or changed since): shallow first, newest first.
pub fn unclassified_files(
    store: &IndexStore,
    scope: &str,
    include_subfolders: bool,
) -> Result<Vec<FileRecord>> {
    let (clause, args) = scope_clause(store, scope, include_subfolders);
    let c = store.conn();
    let mut st = c.prepare(&format!(
        "SELECT f.path, f.name, f.parent, f.ext, f.size, f.mtime, f.inode, f.kind, f.cloud, f.hash
         FROM files f
         LEFT JOIN classes c ON c.path = f.path AND c.mtime = f.mtime AND c.size = f.size
         WHERE f.kind = 0 AND f.cloud = 0 AND {clause} AND c.path IS NULL AND f.ext IN ({})",
        ext_list_sql()
    ))?;
    let rows = st.query_map(rusqlite::params_from_iter(args.iter()), |r| {
        Ok(FileRecord {
            path: r.get(0)?,
            name: r.get(1)?,
            parent: r.get(2)?,
            ext: r.get(3)?,
            size: r.get::<_, i64>(4)?.max(0) as u64,
            modified: r.get(5)?,
            inode: r.get::<_, i64>(6)? as u64,
            kind: FileKind::from_i64(r.get(7)?),
            cloud: r.get::<_, i64>(8)? != 0,
            hash: r.get(9)?,
        })
    })?;
    let mut v: Vec<FileRecord> = rows.collect::<rusqlite::Result<_>>()?;
    let style = store.style();
    v.sort_by(|a, b| {
        style
            .depth(&a.path)
            .cmp(&style.depth(&b.path))
            .then_with(|| b.modified.cmp(&a.modified))
    });
    Ok(v)
}

/// Store one result (replaces any previous one for the path).
#[allow(clippy::too_many_arguments)]
fn save_result(
    store: &IndexStore,
    rec: &FileRecord,
    bucket: &str,
    confidence: f64,
    summary: &str,
    manual: bool,
) -> Result<()> {
    store.conn().execute(
        "INSERT INTO classes(path, bucket, confidence, summary, mtime, size, manual) VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(path) DO UPDATE SET bucket=excluded.bucket, confidence=excluded.confidence,
             summary=excluded.summary, mtime=excluded.mtime, size=excluded.size, manual=excluded.manual",
        params![rec.path, bucket, confidence, summary, rec.modified, rec.size as i64, manual as i64],
    )?;
    Ok(())
}

/// `set_bucket`: the user moved files to a bucket by hand (manual — kept by re-classify).
/// `bucket` may be a bucket name or id.
pub fn set_bucket(store: &IndexStore, paths_in: &[String], bucket: &str) -> Result<()> {
    let buckets = load_buckets(store)?;
    let name = buckets
        .iter()
        .find(|b| b.id == bucket)
        .or_else(|| buckets.iter().find(|b| b.name == bucket))
        .map(|b| b.name.clone())
        .unwrap_or_else(|| bucket.to_string());
    for p in paths_in {
        let existing: Option<String> = {
            let c = store.conn();
            c.query_row("SELECT summary FROM classes WHERE path = ?1", [p], |r| {
                r.get(0)
            })
            .optional()?
        };
        let Some(rec) = store.record(p)? else {
            continue;
        };
        if rec.kind != FileKind::File {
            continue;
        }
        save_result(
            store,
            &rec,
            &name,
            1.0,
            existing.as_deref().unwrap_or(""),
            true,
        )?;
    }
    Ok(())
}

/// Forget non-manual (or all) results in scope.
pub fn clear_classifications(
    store: &IndexStore,
    scope: &str,
    include_subfolders: bool,
    keep_manual: bool,
) -> Result<usize> {
    let manual = if keep_manual { " AND manual = 0" } else { "" };
    let c = store.conn();
    let n = if include_subfolders {
        let (lo, hi) = store.style().descendant_range(scope);
        c.execute(
            &format!("DELETE FROM classes WHERE path >= ?1 AND path < ?2{manual}"),
            params![lo, hi],
        )?
    } else {
        c.execute(
            &format!("DELETE FROM classes WHERE path IN (SELECT path FROM files WHERE parent = ?1){manual}"),
            [scope],
        )?
    };
    Ok(n)
}

// ───────────── prompt, grammar, verdict ─────────────

/// System + user message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

impl Prompt {
    /// Qwen3 chat format with thinking disabled (empty think block).
    pub fn render_qwen(&self) -> String {
        format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n",
            self.system, self.user
        )
    }
}

fn language_name(lang: &str) -> &'static str {
    match lang {
        "ja" => "Japanese",
        "zh-Hans" => "Simplified Chinese",
        "ko" => "Korean",
        _ => "English",
    }
}

/// Port of `ClassifierEngine.Prompt.make`: Korean instructions for a Korean UI, English
/// otherwise with the summary language named.
pub fn make_prompt(
    name: &str,
    folder: &str,
    ext: &str,
    excerpt: Option<&str>,
    buckets: &[Bucket],
    fallback: &str,
    lang: &str,
) -> Prompt {
    let list = buckets
        .iter()
        .map(|b| format!("- {}: {}", b.name, b.hint))
        .collect::<Vec<_>>()
        .join("\n");
    let excerpt = excerpt.filter(|e| !e.trim().is_empty());
    if lang == "ko" {
        let system = format!(
            "당신은 개인용 PC의 파일 정리 도우미다. 파일 하나의 정보(이름, 들어 있는 폴더, 내용 일부)를 보고\n\
             아래 묶음 중 정확히 하나를 고른다. 반드시 목록에 있는 이름을 그대로 쓴다. 애매하면 '{fallback}'.\n\
             파일명과 폴더 이름이 내용보다 더 분명한 단서일 때가 많다. 요약은 한국어 한 줄로 쓴다.\n\
             답은 JSON 하나로만 쓴다: {{\"category\": 묶음 이름, \"confidence\": 0에서 1 사이 숫자, \"summary\": 40자 이내 한 줄 요약}}\n\
             \n\
             묶음 목록:\n{list}"
        );
        let mut user = format!(
            "파일명: {name}\n폴더: {folder}\n형식: {}\n",
            if ext.is_empty() { "없음" } else { ext }
        );
        match excerpt {
            Some(e) => user.push_str(&format!("내용 일부:\n{e}\n")),
            None => user.push_str("내용: (읽을 수 없는 형식 — 이름과 폴더로 판단)\n"),
        }
        return Prompt { system, user };
    }
    let language = language_name(lang);
    let system = format!(
        "You help organize files on a personal PC. Given one file's name, the folder it is in and part of its content,\n\
         choose exactly one bucket from the list below. Copy the bucket name exactly as written. If unsure, choose '{fallback}'.\n\
         The file name and folder name are often clearer clues than the content. Write the summary as one line in {language}.\n\
         Answer with one JSON object only: {{\"category\": bucket name, \"confidence\": number from 0 to 1, \"summary\": one line under 40 characters}}\n\
         \n\
         Buckets:\n{list}"
    );
    let mut user = format!(
        "File name: {name}\nFolder: {folder}\nType: {}\n",
        if ext.is_empty() { "none" } else { ext }
    );
    match excerpt {
        Some(e) => user.push_str(&format!("Content excerpt:\n{e}\n")),
        None => user.push_str("Content: (unreadable format — judge by the name and folder)\n"),
    }
    Prompt { system, user }
}

/// A GBNF string literal producing `s` exactly.
fn gbnf_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// GBNF grammar for `{"category": <one of names>, "confidence": <0..1>, "summary": "<≤ 60 chars>"}`.
pub fn grammar(names: &[String]) -> String {
    let alts: Vec<String> = names
        .iter()
        .map(|n| gbnf_literal(&serde_json::to_string(n).unwrap_or_else(|_| "\"\"".into())))
        .collect();
    let category = if alts.is_empty() {
        gbnf_literal("\"\"")
    } else {
        alts.join(" | ")
    };
    format!(
        "root ::= \"{{\" ws \"\\\"category\\\"\" ws \":\" ws category ws \",\" ws \"\\\"confidence\\\"\" ws \":\" ws confidence ws \",\" ws \"\\\"summary\\\"\" ws \":\" ws summary ws \"}}\"\n\
         category ::= {category}\n\
         confidence ::= \"0\" (\".\" [0-9] [0-9]?)? | \"1\" (\".\" \"0\" \"0\"?)?\n\
         summary ::= \"\\\"\" char{{0,{SUMMARY_MAX}}} \"\\\"\"\n\
         char ::= [^\"\\\\\\x7F\\x00-\\x1F] | \"\\\\\" [\"\\\\/bfnrt]\n\
         ws ::= \" \"?\n"
    )
}

/// What the model answered (after mapping to a real bucket).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Verdict {
    pub category: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub summary: String,
}

/// Parse the model's JSON (tolerates text around the object).
pub fn parse_verdict(raw: &str) -> Result<Verdict> {
    let start = raw
        .find('{')
        .ok_or_else(|| anyhow::anyhow!("no JSON in model output"))?;
    let end = raw
        .rfind('}')
        .ok_or_else(|| anyhow::anyhow!("no JSON in model output"))?;
    anyhow::ensure!(end > start, "no JSON in model output");
    Ok(serde_json::from_str(&raw[start..=end])?)
}

/// The bucket for a model answer: exact name, else a name containing it (or contained in
/// it), else the fallback. No guessing beyond that.
pub fn resolve_category(raw: &str, names: &[String], fallback: &str) -> String {
    let r = paths::nfc(raw.trim());
    if let Some(n) = names.iter().find(|n| paths::nfc(n) == r) {
        return n.clone();
    }
    let rl = r.to_lowercase();
    if !rl.is_empty() {
        if let Some(n) = names.iter().find(|n| {
            let nl = paths::fold(n);
            nl.contains(&rl) || rl.contains(&nl)
        }) {
            return n.clone();
        }
    }
    fallback.to_string()
}

/// Everything an engine needs for one file.
#[derive(Clone, Debug)]
pub struct ClassifyInput {
    pub name: String,
    /// folder shown to the model (home replaced by ~)
    pub folder: String,
    pub ext: String,
    /// content excerpt (≤ 1500 chars) or None
    pub text: Option<String>,
    pub buckets: Vec<Bucket>,
    pub fallback: String,
    /// "ko" | "en" | "ja" | "zh-Hans"
    pub lang: String,
}

impl ClassifyInput {
    /// Prompt with the excerpt cut to `excerpt_chars` (engines shrink it to fit their context).
    pub fn prompt(&self, excerpt_chars: usize) -> Prompt {
        let excerpt: Option<String> = self
            .text
            .as_ref()
            .map(|t| t.chars().take(excerpt_chars).collect());
        make_prompt(
            &self.name,
            &self.folder,
            &self.ext,
            excerpt.as_deref(),
            &self.buckets,
            &self.fallback,
            &self.lang,
        )
    }

    pub fn bucket_names(&self) -> Vec<String> {
        self.buckets.iter().map(|b| b.name.clone()).collect()
    }

    pub fn grammar(&self) -> String {
        grammar(&self.bucket_names())
    }
}

/// A classifier. The real one is `llm::LlamaEngine` (feature `llm`); tests use [`FakeEngine`].
pub trait Engine {
    fn classify(&mut self, input: &ClassifyInput) -> anyhow::Result<Verdict>;
}

/// Deterministic engine for tests and UI work: the first keyword found in the file name or
/// text picks the bucket, otherwise the fallback.
#[derive(Clone, Debug, Default)]
pub struct FakeEngine {
    /// (keyword, bucket name)
    pub rules: Vec<(String, String)>,
    /// file names that make the engine fail
    pub fail_on: Vec<String>,
    pub calls: usize,
    /// the last prompt seen (for assertions)
    pub last_prompt: Option<Prompt>,
}

impl Engine for FakeEngine {
    fn classify(&mut self, input: &ClassifyInput) -> anyhow::Result<Verdict> {
        self.calls += 1;
        self.last_prompt = Some(input.prompt(EXCERPT_LIMIT));
        if self.fail_on.contains(&input.name) {
            anyhow::bail!("fake failure");
        }
        let hay =
            format!("{} {}", input.name, input.text.clone().unwrap_or_default()).to_lowercase();
        for (k, b) in &self.rules {
            if hay.contains(&k.to_lowercase()) {
                return Ok(Verdict {
                    category: b.clone(),
                    confidence: 0.9,
                    summary: format!("{k} file"),
                });
            }
        }
        Ok(Verdict {
            category: input.fallback.clone(),
            confidence: 0.3,
            summary: String::new(),
        })
    }
}

// ───────────── the job ─────────────

/// `classify` command body. Loads the engine lazily (only when a file needs the model, once
/// per run), classifies pending files in scope, stores results. `mode: all` first forgets
/// non-manual results in scope. Progress: phase "classify", done/total, current = file name.
pub fn run_classify(
    store: &IndexStore,
    req: &ClassifyRequest,
    lang: &str,
    engine_factory: &mut dyn FnMut() -> Result<Box<dyn Engine>>,
    progress: &dyn Fn(&ProgressEvent),
    cancel: &Cancel,
) -> Result<ClassifyFinishedEvent> {
    let style: PathStyle = store.style();
    let scope = style.trim(&req.scope);
    if req.mode == ClassifyMode::All {
        clear_classifications(store, &scope, req.include_subfolders, true)?;
    }
    let buckets = get_buckets(store, lang)?;
    let names: Vec<String> = buckets.iter().map(|b| b.name.clone()).collect();
    let fallback = buckets
        .iter()
        .find(|b| b.is_fallback)
        .map(|b| b.name.clone())
        .unwrap_or_else(|| fallback_bucket(lang).name);
    let installers = buckets
        .iter()
        .find(|b| b.id == "installers" || INSTALLER_NAMES.contains(&b.name.as_str()))
        .map(|b| b.name.clone());
    let pending = unclassified_files(store, &scope, req.include_subfolders)?;
    let total = pending.len() as u64;
    let home = paths::home_string();
    let mut engine: Option<Box<dyn Engine>> = None;
    let mut done = ClassifyFinishedEvent::default();
    let mut cache: HashMap<String, String> = HashMap::new(); // folder display cache

    for (i, rec) in pending.iter().enumerate() {
        if cancel.is_cancelled() {
            done.cancelled = true;
            break;
        }
        progress(&ProgressEvent::new(
            "classify",
            i as u64,
            total,
            Some(rec.name.clone()),
        ));
        // obvious without the model: installers
        if let Some(inst) = &installers {
            if INSTALLER_EXTENSIONS.contains(&rec.ext.as_str()) {
                save_result(store, rec, inst, 0.95, installer_summary(lang), false)?;
                done.classified += 1;
                continue;
            }
        }
        if engine.is_none() {
            engine = Some(engine_factory()?);
        }
        let folder = cache
            .entry(rec.parent.clone())
            .or_insert_with(|| paths::display_with_home(&rec.parent, &home, style))
            .clone();
        let excerpt = if !rec.cloud && text::can_extract(&rec.ext) {
            text::extract(Path::new(&rec.path), &rec.ext, EXCERPT_LIMIT)
        } else {
            None
        };
        let input = ClassifyInput {
            name: rec.name.clone(),
            folder,
            ext: rec.ext.clone(),
            text: excerpt,
            buckets: buckets.clone(),
            fallback: fallback.clone(),
            lang: lang.to_string(),
        };
        let eng = engine.as_mut().expect("engine loaded");
        match eng.classify(&input) {
            Ok(v) => {
                let category = resolve_category(&v.category, &names, &fallback);
                let summary: String = v.summary.trim().chars().take(SUMMARY_MAX).collect();
                save_result(
                    store,
                    rec,
                    &category,
                    v.confidence.clamp(0.0, 1.0),
                    &summary,
                    false,
                )?;
                done.classified += 1;
            }
            Err(e) if crate::progress::is_cancelled(&e) => {
                done.cancelled = true;
                break;
            }
            Err(e) => {
                // one failure does not stop the run; recorded under the fallback so it is not retried every time
                save_result(
                    store,
                    rec,
                    &fallback,
                    0.0,
                    &failure_summary(lang, &e.to_string()),
                    false,
                )?;
                done.failed += 1;
            }
        }
    }
    progress(&ProgressEvent::new(
        "classify",
        done.classified + done.failed,
        total,
        None,
    ));
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::no_progress;
    use crate::rules::ScanRules;
    use crate::scanner::Scanner;
    use crate::types::Bucket;

    #[test]
    fn languages_and_defaults() {
        assert_eq!(ui_language("ko-KR"), "ko");
        assert_eq!(ui_language("ja_JP"), "ja");
        assert_eq!(ui_language("zh-CN"), "zh-Hans");
        assert_eq!(ui_language("zh-Hans-SG"), "zh-Hans");
        assert_eq!(ui_language("zh-TW"), "en");
        assert_eq!(ui_language("de-DE"), "en");
        for lang in ["ko", "en", "ja", "zh-Hans"] {
            let d = default_buckets(lang);
            assert_eq!(d.len(), 7);
            assert!(d.last().unwrap().is_fallback);
            assert!(FALLBACK_NAMES.contains(&d.last().unwrap().name.as_str()));
            assert!(INSTALLER_NAMES.contains(&d[5].name.as_str()));
            assert!(d[5].hint.contains("exe"));
            assert_eq!(d.iter().filter(|b| b.is_fallback).count(), 1);
        }
        assert_eq!(default_buckets("ko")[0].name, "업무");
        assert_eq!(default_buckets("en")[1].name, "School & Study");
    }

    #[test]
    fn fallback_bucket_recognized_in_any_language_and_forced_last() {
        for name in FALLBACK_NAMES {
            assert!(is_fallback_name(name));
        }
        assert!(!is_fallback_name("업무"));
        let input = vec![
            Bucket {
                id: "o".into(),
                name: "Other".into(),
                hint: "".into(),
                is_fallback: false,
            },
            Bucket {
                id: "w".into(),
                name: " Work ".into(),
                hint: "w".into(),
                is_fallback: false,
            },
            Bucket {
                id: "w".into(),
                name: "Work2".into(),
                hint: "".into(),
                is_fallback: false,
            },
            Bucket {
                id: "x".into(),
                name: "  ".into(),
                hint: "".into(),
                is_fallback: false,
            },
            Bucket {
                id: "y".into(),
                name: "Work".into(),
                hint: "".into(),
                is_fallback: false,
            },
        ];
        let out = clean_buckets(&input, "en");
        let names: Vec<_> = out.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["Work", "Work2", "Other"]);
        assert!(out[2].is_fallback);
        assert_ne!(out[0].id, out[1].id);
        let none = clean_buckets(
            &[Bucket {
                id: "".into(),
                name: "업무".into(),
                hint: "".into(),
                is_fallback: false,
            }],
            "ko",
        );
        assert_eq!(none.last().unwrap().name, "기타");
        assert!(!none[0].id.is_empty());
    }

    #[test]
    fn prompt_matches_the_macos_wording() {
        let buckets = default_buckets("ko");
        let p = make_prompt(
            "견적서.pdf",
            "~\\Downloads",
            "pdf",
            Some("금액 합계"),
            &buckets,
            "기타",
            "ko",
        );
        assert!(p
            .system
            .starts_with("당신은 개인용 PC의 파일 정리 도우미다."));
        assert!(p.system.contains("애매하면 '기타'."));
        assert!(p.system.contains("- 업무: 회사·업무 문서"));
        assert_eq!(
            p.user,
            "파일명: 견적서.pdf\n폴더: ~\\Downloads\n형식: pdf\n내용 일부:\n금액 합계\n"
        );
        let q = make_prompt("x", "~", "", None, &default_buckets("ja"), "その他", "ja");
        assert!(q
            .system
            .contains("Write the summary as one line in Japanese."));
        assert!(q
            .user
            .contains("Type: none\nContent: (unreadable format — judge by the name and folder)\n"));
        let r = q.render_qwen();
        assert!(r.starts_with("<|im_start|>system\nYou help organize files on a personal PC."));
        assert!(r.ends_with("<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"));
    }

    #[test]
    fn grammar_restricts_category_to_bucket_names() {
        let names = vec![
            "업무".to_string(),
            "Say \"hi\"".to_string(),
            "a\\b".to_string(),
        ];
        let g = grammar(&names);
        assert!(g.contains("category ::= \"\\\"업무\\\"\" | \"\\\"Say \\\\\\\"hi\\\\\\\"\\\"\" | \"\\\"a\\\\\\\\b\\\"\""), "{g}");
        assert!(g.contains("char{0,60}"));
        assert!(g.starts_with("root ::= \"{\" ws \"\\\"category\\\"\""));
    }

    #[test]
    fn verdict_parsing_and_closest_name() {
        let v =
            parse_verdict("{\"category\": \"업무\", \"confidence\": 0.8, \"summary\": \"견적서\"}")
                .unwrap();
        assert_eq!(v.category, "업무");
        assert!(parse_verdict("nothing").is_err());
        let names = vec![
            "업무".to_string(),
            "School & Study".to_string(),
            "기타".to_string(),
        ];
        assert_eq!(resolve_category("업무", &names, "기타"), "업무");
        assert_eq!(resolve_category("school", &names, "기타"), "School & Study");
        assert_eq!(resolve_category("Unknown", &names, "기타"), "기타");
        assert_eq!(resolve_category("", &names, "기타"), "기타");
    }

    fn setup() -> (tempfile::TempDir, String, IndexStore) {
        let base = tempfile::Builder::new().prefix("mf-cls").tempdir().unwrap();
        let root = base.path().join("data");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("invoice-2024.txt"), "Invoice total 120,000 KRW").unwrap();
        std::fs::write(root.join("lecture-notes.md"), "# Week 3 lecture").unwrap();
        std::fs::write(root.join("setup.exe"), "MZ").unwrap();
        std::fs::write(root.join("broken.txt"), "x").unwrap();
        std::fs::write(root.join("song.unknownext"), "x").unwrap();
        std::fs::write(root.join("sub/deep.txt"), "invoice").unwrap();
        let store = IndexStore::open(&base.path().join("db/index.sqlite")).unwrap();
        let r = root.to_str().unwrap().to_string();
        Scanner::new(ScanRules::open(store.style()))
            .scan(&store, &[r.clone()], &no_progress, &Cancel::new())
            .unwrap();
        (base, r, store)
    }

    fn fake() -> FakeEngine {
        FakeEngine {
            rules: vec![
                ("invoice".into(), "Work".into()),
                ("lecture".into(), "School & Study".into()),
            ],
            fail_on: vec!["broken.txt".into()],
            ..Default::default()
        }
    }

    #[test]
    fn job_classifies_keeps_manual_and_counts_pending() {
        let (_b, root, store) = setup();
        let req = ClassifyRequest {
            scope: root.clone(),
            include_subfolders: false,
            mode: ClassifyMode::New,
        };
        assert_eq!(list_classified(&store, &root, false).unwrap().pending, 4);
        assert_eq!(list_classified(&store, &root, true).unwrap().pending, 5);
        let mut created = 0;
        let mut factory = || -> Result<Box<dyn Engine>> {
            created += 1;
            Ok(Box::new(fake()))
        };
        let fin = run_classify(
            &store,
            &req,
            "en",
            &mut factory,
            &no_progress,
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(created, 1, "the model is loaded once per run");
        assert_eq!(
            fin,
            ClassifyFinishedEvent {
                classified: 3,
                failed: 1,
                cancelled: false
            }
        );
        let list = list_classified(&store, &root, false).unwrap();
        assert_eq!(list.pending, 0);
        let by_name: HashMap<String, Classified> = list
            .results
            .into_iter()
            .map(|c| {
                (
                    Path::new(&c.path)
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                    c,
                )
            })
            .collect();
        assert_eq!(by_name["invoice-2024.txt"].bucket, "Work");
        assert_eq!(by_name["lecture-notes.md"].bucket, "School & Study");
        assert_eq!(by_name["setup.exe"].bucket, "Installers & Temporary");
        assert_eq!(
            by_name["setup.exe"].summary,
            "Installer — safe to delete once installed"
        );
        assert_eq!(by_name["broken.txt"].bucket, "Other");
        assert!(by_name["broken.txt"]
            .summary
            .starts_with("Couldn’t categorize"));

        // manual choice survives "classify all"
        let lecture = by_name["lecture-notes.md"].path.clone();
        set_bucket(&store, &[lecture.clone()], "personal").unwrap();
        let mut factory2 = || -> Result<Box<dyn Engine>> { Ok(Box::new(fake())) };
        let all = ClassifyRequest {
            scope: root.clone(),
            include_subfolders: true,
            mode: ClassifyMode::All,
        };
        let fin = run_classify(
            &store,
            &all,
            "en",
            &mut factory2,
            &no_progress,
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(
            fin.classified + fin.failed,
            4,
            "lecture is manual and skipped; deep.txt is new"
        );
        let list = list_classified(&store, &root, true).unwrap();
        let lecture_row = list.results.iter().find(|c| c.path == lecture).unwrap();
        assert_eq!(lecture_row.bucket, "Personal");
        assert!(lecture_row.manual);
        assert_eq!(list.results.len(), 5);

        // a changed file is pending again
        std::fs::write(&lecture, "# changed and longer").unwrap();
        crate::watcher::apply_changes(&store, &ScanRules::open(store.style()), &[lecture.clone()]);
        assert_eq!(list_classified(&store, &root, false).unwrap().pending, 1);
    }

    #[test]
    fn renaming_and_removing_buckets_updates_results() {
        let (_b, root, store) = setup();
        let mut factory = || -> Result<Box<dyn Engine>> { Ok(Box::new(fake())) };
        let req = ClassifyRequest {
            scope: root.clone(),
            include_subfolders: false,
            mode: ClassifyMode::New,
        };
        run_classify(
            &store,
            &req,
            "en",
            &mut factory,
            &no_progress,
            &Cancel::new(),
        )
        .unwrap();
        let mut buckets = get_buckets(&store, "en").unwrap();
        buckets[0].name = "Job".into(); // Work → Job
        buckets.retain(|b| b.id != "school"); // remove School & Study
        let saved = save_buckets(&store, &buckets, "en").unwrap();
        assert_eq!(saved.last().unwrap().name, "Other");
        let list = list_classified(&store, &root, false).unwrap();
        assert!(list.results.iter().any(|c| c.bucket == "Job"));
        assert!(!list
            .results
            .iter()
            .any(|c| c.bucket == "Work" || c.bucket == "School & Study"));
        assert_eq!(list.pending, 1, "the lecture file is unclassified again");
    }

    #[test]
    fn cancel_stops_the_job() {
        let (_b, root, store) = setup();
        let c = Cancel::new();
        c.cancel();
        let mut factory = || -> Result<Box<dyn Engine>> { Ok(Box::new(fake())) };
        let req = ClassifyRequest {
            scope: root,
            include_subfolders: false,
            mode: ClassifyMode::New,
        };
        let fin = run_classify(&store, &req, "ko", &mut factory, &no_progress, &c).unwrap();
        assert!(fin.cancelled);
        assert_eq!(fin.classified, 0);
    }
}
