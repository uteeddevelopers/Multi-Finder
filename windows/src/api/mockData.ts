// Fake disk for the browser mock. Every name here is invented for the demo.
import type { Kind } from "./types";

export const HOME = "C:\\Users\\Alex";
export const DOWNLOADS = `${HOME}\\Downloads`;
export const DESKTOP = `${HOME}\\Desktop`;
export const PICTURES = `${HOME}\\Pictures`;
export const CAMERA = "E:\\Camera Import";
export const SORTED = `${DESKTOP}\\Sorted`;
export const PHOTOS_LEFT = `${DESKTOP}\\Photos 2025`;
export const PHOTOS_RIGHT = `${DESKTOP}\\Photos backup`;

export interface MockNode {
  path: string;
  name: string;
  parent: string;
  kind: Kind;
  size: number;
  modified: number;
  cloud: boolean;
  hash: string;
  text?: string;
}

const DAY = 86_400_000;
const KB = 1000;
const MB = 1000 * 1000;
const GB = 1000 * MB;

/** Local time `daysAgo` days before today at hh:mm. */
function ago(daysAgo: number, hh = 10, mm = 0): number {
  const d = new Date();
  d.setHours(hh, mm, 0, 0);
  return d.getTime() - daysAgo * DAY;
}
const on = (y: number, m: number, d: number, hh = 10, mm = 0) => new Date(y, m - 1, d, hh, mm).getTime();

type Extra = { hash?: string; cloud?: boolean; text?: string };
type Spec = [rel: string, size: number, modified: number, extra?: Extra];

const TEXT = {
  readme: "Thanks for downloading!\r\n\r\n1. Run the installer.\r\n2. Sign in with your account.\r\n3. Pick the folders to sync.\r\n\r\nQuestions? See the help page in the app.",
  todo: "This week\n- [x] Send the Q3 numbers\n- [ ] Book the venue for the offsite\n- [ ] Back up Photos 2025 to the external drive\n- [ ] Clean up Downloads\n",
  meeting: "# Weekly sync\n\n**Attendees:** Dana, Min, Alex\n\n## Notes\n- Launch moved to the second week of December\n- Need final copy for the landing page\n- Budget review on Friday\n\n## Action items\n- Alex: draft the agenda\n- Min: update the timeline\n",
  csv: "id,region,units,revenue\n1001,North,320,48200\n1002,South,214,30150\n1003,East,401,61730\n1004,West,188,27420\n",
  week2: "# Week 2 — Data structures\n\n- Arrays vs. linked lists\n- Big-O of common operations\n- Hash maps: buckets, collisions, load factor\n\nReading: chapter 3 (pp. 41–67)\n",
  pkg: '{\n  "name": "weather-app",\n  "version": "0.3.1",\n  "private": true,\n  "scripts": { "dev": "vite", "build": "vite build" }\n}\n',
  appts: "import { fetchForecast } from \"./api\";\n\nexport async function main(city: string) {\n  const days = await fetchForecast(city);\n  for (const d of days) console.log(d.date, d.high, d.low);\n}\n",
  apits: "export async function fetchForecast(city: string) {\n  const res = await fetch(`/api/forecast?city=${encodeURIComponent(city)}`);\n  return res.json();\n}\n",
  wreadme: "# weather-app\n\nSmall demo that shows a 7-day forecast.\n\n```\nnpm install\nnpm run dev\n```\n",
};

const DOWNLOAD_FILES: Spec[] = [
  ["Invoices\\invoice-northwind-2026-09.pdf", 182 * KB, ago(40, 9, 12)],
  ["Invoices\\invoice-northwind-2026-10.pdf", 176 * KB, ago(9, 16, 40), { hash: "inv-oct" }],
  ["Invoices\\receipt-cloud-storage-2026-10.pdf", 64 * KB, ago(12, 8, 5)],
  ["Course materials\\week01-introduction.pdf", 2.4 * MB, ago(60, 13, 0)],
  ["Course materials\\week02-notes.md", 6 * KB, ago(53, 21, 10), { text: TEXT.week2 }],
  ["Course materials\\assignment-1-draft.docx", 88 * KB, ago(47, 23, 41)],
  ["PixelPaint-Setup-4.2.1.exe", 96.3 * MB, ago(21, 11, 2), { hash: "pixelpaint" }],
  ["OldEditor-Setup-2.0.exe", 54.1 * MB, ago(400, 15, 30)],
  ["CloudSync-Installer.msi", 38.7 * MB, ago(5, 10, 18)],
  ["Screenshot 2026-10-21 094512.png", 1.2 * MB, ago(13, 9, 45)],
  ["Screenshot 2026-10-29 173008.png", 846 * KB, ago(5, 17, 30)],
  ["mountain-lake.jpg", 3.4 * MB, ago(30, 12, 0), { hash: "lake" }],
  ["team-offsite-agenda.docx", 42 * KB, ago(2, 14, 20)],
  ["quarterly-sales-q3.xlsx", 128 * KB, ago(25, 18, 2)],
  ["sample-data.csv", 18 * KB, ago(70, 10, 10), { text: TEXT.csv }],
  ["fonts-collection.zip", 12.8 * MB, ago(90, 20, 0)],
  ["conference-keynote.mp4", 412 * MB, ago(33, 19, 0)],
  ["podcast-episode-42.mp3", 48.2 * MB, ago(15, 7, 30)],
  ["readme-first.txt", 3 * KB, ago(0, 8, 50), { text: TEXT.readme }],
  ["invoice-northwind-2026-10 (1).pdf", 176 * KB, ago(9, 16, 42), { hash: "inv-oct" }],
  ["project-plan.pptx", 2.1 * MB, ago(8, 11, 11), { cloud: true }],
];

const photoSize = (n: number) => Math.round((2.3 + ((n * 37) % 60) / 100) * MB);
const photoDate = (n: number) => on(2025, 7, 12 + Math.floor((n - 2040) / 3), 9 + (n % 8), (n * 7) % 60);

function photos(): Spec[] {
  const out: Spec[] = [];
  for (let n = 2041; n <= 2052; n++) out.push([`Photos 2025\\DSC_${n}.jpg`, photoSize(n), photoDate(n), { hash: `photo-${n}` }]);
  out.push(["Photos backup\\DSC_2040.jpg", photoSize(2040), photoDate(2040), { hash: "photo-2040" }]);
  for (let n = 2041; n <= 2048; n++) out.push([`Photos backup\\DSC_${n}.jpg`, photoSize(n), photoDate(n), { hash: `photo-${n}` }]);
  out.push(["Photos backup\\DSC_2049.jpg", photoSize(2049) - 182_000, photoDate(2049) - 2 * DAY, { hash: "photo-2049-old" }]);
  out.push(["Photos backup\\edited\\DSC_2052.jpg", photoSize(2052), photoDate(2052), { hash: "photo-2052" }]);
  return out;
}

const DESKTOP_FILES: Spec[] = [
  ...photos(),
  ["Setup files\\PixelPaint-Setup-4.2.1.exe", 96.3 * MB, ago(21, 11, 2), { hash: "pixelpaint" }],
  ["Tax 2025\\tax-summary-2025.pdf", 512 * KB, ago(160, 10, 0)],
  ["Tax 2025\\donation-receipts.pdf", 233 * KB, ago(158, 10, 30)],
  ["weather-app\\package.json", 1 * KB, ago(18, 22, 4), { text: TEXT.pkg }],
  ["weather-app\\README.md", 2 * KB, ago(18, 22, 4), { text: TEXT.wreadme }],
  ["weather-app\\src\\app.ts", 4 * KB, ago(17, 23, 15), { text: TEXT.appts }],
  ["weather-app\\src\\api.ts", 2 * KB, ago(17, 23, 1), { text: TEXT.apits }],
  ["weather-app\\assets\\logo.png", 1.4 * MB, ago(19, 20, 0), { hash: "wlogo" }],
  ["weather-app\\assets\\logo-copy.png", 1.4 * MB, ago(19, 20, 1), { hash: "wlogo" }],
  ["todo.txt", 1 * KB, ago(1, 9, 3), { text: TEXT.todo }],
  ["meeting-notes.md", 4 * KB, ago(3, 15, 0), { text: TEXT.meeting }],
  ["mountain-lake (1).jpg", 3.4 * MB, ago(30, 12, 0), { hash: "lake" }],
  ["family-trip.mov", 1.3 * GB, ago(120, 18, 0)],
];

const PICTURE_FILES: Spec[] = [
  ["Camera Roll\\WIN_20251012_14_22_31_Pro.jpg", 1.9 * MB, on(2025, 10, 12, 14, 22)],
  ["Camera Roll\\WIN_20251012_14_23_05_Pro.jpg", 2.0 * MB, on(2025, 10, 12, 14, 23)],
  ["Saved Pictures\\sunset-pier.jpg", 2.7 * MB, ago(200, 19, 0)],
  ["Saved Pictures\\city-lights.png", 4.1 * MB, ago(180, 21, 0)],
];

const CAMERA_FILES: Spec[] = [
  ...Array.from({ length: 8 }, (_, i): Spec => [`DSCF${3101 + i}.JPG`, Math.round((5.1 + i * 0.13) * MB), on(2026, 10, 18, 10 + i, 3 * i)]),
  ["DSCF3109.MOV", 640 * MB, on(2026, 10, 18, 17, 40)],
];

function extOf(name: string) {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

/** `?big=1`: 30,000 extra files in Desktop\Scans, to exercise the virtualized lists. */
function bigFolder(): Spec[] {
  const out: Spec[] = [];
  for (let i = 1; i <= 30_000; i++) out.push([`Scans\\scan_${String(i).padStart(5, "0")}.pdf`, 40_000 + ((i * 7919) % 900_000), ago(i % 900, 8 + (i % 10), i % 60)]);
  return out;
}

export function buildDisk(big = false): Map<string, MockNode> {
  const disk = new Map<string, MockNode>();
  const addDir = (path: string, modified: number) => {
    const existing = disk.get(path);
    if (existing) {
      existing.modified = Math.max(existing.modified, modified);
      return;
    }
    const i = path.lastIndexOf("\\");
    disk.set(path, { path, name: path.slice(i + 1), parent: path.slice(0, i), kind: "dir", size: 0, modified, cloud: false, hash: "" });
  };
  const addTree = (root: string, specs: Spec[]) => {
    addDir(root, 0);
    for (const [rel, size, modified, extra] of specs) {
      const parts = rel.split("\\");
      let dir = root;
      for (const part of parts.slice(0, -1)) {
        dir = `${dir}\\${part}`;
        addDir(dir, modified);
      }
      const name = parts[parts.length - 1];
      const path = `${root}\\${rel}`;
      disk.set(path, {
        path,
        name,
        parent: dir,
        kind: "file",
        size: Math.round(size),
        modified,
        cloud: extra?.cloud ?? false,
        hash: extra?.hash ?? `h:${path.toLowerCase()}`,
        text: extra?.text,
      });
      // folders get the newest modification time of what was put in them
      let p = dir;
      while (p.length >= root.length) {
        addDir(p, modified);
        p = p.slice(0, p.lastIndexOf("\\"));
      }
    }
  };
  addTree(DOWNLOADS, DOWNLOAD_FILES);
  addTree(DESKTOP, big ? [...DESKTOP_FILES, ...bigFolder()] : DESKTOP_FILES);
  addDir(SORTED, ago(6, 12, 0));
  addTree(PICTURES, PICTURE_FILES);
  addTree(CAMERA, CAMERA_FILES);
  for (const n of disk.values()) if (n.kind === "file" && !n.text && ["txt", "md", "csv", "json", "ts"].includes(extOf(n.name))) n.text = "";
  return disk;
}

// ───────────────────────────── categories ─────────────────────────────

/** Bucket slots in the default order: 0 work, 1 school, 2 personal, 3 dev, 4 media, 5 installers, -1 fallback. */
type Rule = [slot: number, confidence: number, ko: string, en: string];

const BY_NAME: Record<string, Rule> = {
  "PixelPaint-Setup-4.2.1.exe": [5, 0.95, "", ""],
  "OldEditor-Setup-2.0.exe": [5, 0.95, "", ""],
  "CloudSync-Installer.msi": [5, 0.95, "", ""],
  "Screenshot 2026-10-21 094512.png": [5, 0.78, "설정 화면을 잠깐 찍어 둔 캡처", "Quick capture of a settings screen"],
  "Screenshot 2026-10-29 173008.png": [5, 0.74, "오류 메시지 화면 캡처", "Capture of an error message"],
  "mountain-lake.jpg": [4, 0.91, "산과 호수 풍경 사진", "Landscape photo of a mountain lake"],
  "team-offsite-agenda.docx": [0, 0.93, "팀 워크숍 일정과 안건", "Agenda for the team offsite"],
  "quarterly-sales-q3.xlsx": [0, 0.89, "3분기 지역별 매출 집계", "Q3 sales by region"],
  "sample-data.csv": [3, 0.62, "테스트용 지역별 판매 샘플 데이터", "Sample sales data for testing"],
  "fonts-collection.zip": [4, 0.57, "글꼴 파일 모음 압축본", "Archive of font files"],
  "conference-keynote.mp4": [4, 0.71, "콘퍼런스 기조연설 녹화 영상", "Recording of a conference keynote"],
  "podcast-episode-42.mp3": [4, 0.66, "팟캐스트 42화 음성 파일", "Podcast episode 42"],
  "readme-first.txt": [-1, 0.34, "프로그램 설치 안내 메모", "Notes on installing an app"],
  "invoice-northwind-2026-10 (1).pdf": [0, 0.82, "Northwind 10월 청구서 (사본)", "Northwind invoice for October (copy)"],
  "invoice-northwind-2026-09.pdf": [0, 0.86, "Northwind 9월 청구서", "Northwind invoice for September"],
  "invoice-northwind-2026-10.pdf": [0, 0.87, "Northwind 10월 청구서", "Northwind invoice for October"],
  "receipt-cloud-storage-2026-10.pdf": [2, 0.81, "클라우드 저장공간 10월 결제 영수증", "Cloud storage receipt for October"],
  "week01-introduction.pdf": [1, 0.94, "1주차 강의 자료 — 과목 소개", "Week 1 slides — course introduction"],
  "week02-notes.md": [1, 0.9, "2주차 수업 필기 — 자료 구조", "Week 2 class notes on data structures"],
  "assignment-1-draft.docx": [1, 0.88, "과제 1 초안", "Draft of assignment 1"],
  "todo.txt": [2, 0.6, "이번 주 할 일 목록", "This week's to-do list"],
  "meeting-notes.md": [0, 0.85, "주간 회의 메모와 할 일", "Weekly meeting notes and action items"],
  "family-trip.mov": [4, 0.8, "가족 여행 영상", "Family trip video"],
  "mountain-lake (1).jpg": [4, 0.9, "산과 호수 풍경 사진", "Landscape photo of a mountain lake"],
};

const BY_EXT: Record<string, Rule> = {
  jpg: [4, 0.7, "사진", "Photo"],
  png: [4, 0.65, "이미지", "Image"],
  pdf: [-1, 0.4, "PDF 문서", "PDF document"],
  exe: [5, 0.95, "", ""],
  msi: [5, 0.95, "", ""],
  ts: [3, 0.85, "소스 코드", "Source code"],
  json: [3, 0.8, "설정 파일", "Configuration file"],
  md: [3, 0.6, "메모", "Notes"],
};

export function classifyRule(name: string): Rule {
  return BY_NAME[name] ?? BY_EXT[extOf(name)] ?? [-1, 0.3, "내용으로 분명히 판단하기 어려움", "Hard to tell from the contents"];
}
