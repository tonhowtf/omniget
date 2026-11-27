type BookFormat =
  | "epub" | "kepub" | "mobi" | "azw3" | "fb2" | "fbz" | "pdf"
  | "cbz" | "cbr" | "cb7" | "cbt" | "txt" | "md" | "html" | "djvu" | "folder";

type ReadStatus = "unread" | "want" | "reading" | "finished" | "abandoned";

export type Book = {
  id: number;
  path: string;
  folder: string;
  format: BookFormat;
  size: number;
  hash: string;
  title: string;
  sort_title: string;
  authors: string[];
  series: string;
  series_index: number | null;
  language: string;
  publisher: string;
  description: string;
  subjects: string[];
  isbn: string;
  identifiers: [string, string][];
  published: string;
  cover: string | null;
  pages: number | null;
  rtl: boolean;
  fixed_layout: boolean;
  source: string;
  source_url: string;
  series_id: number | null;
  chapter_id: number | null;
  rating: number;
  favorite: boolean;
  status: ReadStatus;
  progress: number;
  location: string;
  last_read_at: number | null;
  started_at: number | null;
  finished_at: number | null;
  read_ms: number;
  tags: string[];
  prefs: Record<string, unknown>;
  added_at: number;
  mtime_ms: number;
};

export type Series = {
  id: number;
  source: string;
  source_key: string;
  title: string;
  alt_titles: string[];
  authors: string[];
  description: string;
  genres: string[];
  status: string;
  lang: string;
  url: string;
  cover_url: string;
  cover: string | null;
  in_library: boolean;
  chapter_langs: string[];
  reader: Record<string, unknown>;
  notes: string;
  tracking: Record<string, unknown>;
  auto_download: boolean;
  added_at: number;
  checked_at: number | null;
  new_at: number | null;
  categories: number[];
  unread: number;
  total: number;
  downloaded: number;
  last_read_at: number | null;
};

export type Chapter = {
  id: number;
  series_id: number;
  key: string;
  title: string;
  number: number | null;
  volume: number | null;
  lang: string;
  scanlator: string;
  published: number | null;
  pages: number | null;
  external_url: string | null;
  position: number;
  read: boolean;
  bookmarked: boolean;
  last_page: number;
  read_at: number | null;
  fetched_at: number;
  book_id: number | null;
};

export type SeriesFull = { series: Series; chapters: Chapter[] };

export type Annotation = {
  id: number;
  book_id: number;
  kind: "highlight" | "note" | "bookmark";
  locator: string;
  text: string;
  note: string;
  color: string;
  style: string;
  chapter: string;
  progress: number | null;
  created_at: number;
  updated_at: number;
};

export type SmartRule = { field: string; op: string; value: string | number | boolean };
export type SmartRules = { matching: "all" | "any"; rules: SmartRule[]; sort?: string; limit?: number | null };

export type Shelf = {
  id: number;
  name: string;
  icon: string;
  smart: SmartRules | null;
  position: number;
  book_ids: number[];
};

export type Download = { url: string; format: string; label: string };

export type Hit = {
  source: string;
  key: string;
  title: string;
  authors: string[];
  cover: string | null;
  summary: string;
  language: string;
  year: number | null;
  subjects: string[];
  kind: "book" | "series" | "folder";
  downloads: Download[];
  url: string;
  extra: Record<string, any>;
};

export type Page = { hits: Hit[]; total: number | null; next: string | null };

export type SourceInfo = {
  id: string;
  name: string;
  kind: "book" | "manga";
  lang: string;
  icon: string;
  builtin: boolean;
  search: boolean;
  latest: boolean;
  nsfw: boolean;
  url: string;
  filters: unknown;
};

export type Catalog = {
  id: number;
  kind: "opds" | "directory" | "suwayomi" | "declarative";
  title: string;
  url: string;
  username: string;
  password: string;
  config: Record<string, any>;
  enabled: boolean;
};

type ShareConfig = { enabled: boolean; port: number; username: string; password: string };

export type ReadStatusInfo = {
  port: number;
  token: string;
  roots: string[];
  book_count: number;
  series_count: number;
  scanning: boolean;
  download_dir: string;
  share: ShareConfig;
  share_running: boolean;
  share_port: number | null;
  lan_ip: string | null;
};

export type Stats = {
  books: number;
  comics: number;
  finished: number;
  reading: number;
  finished_this_year: number;
  total_ms: number;
  today_ms: number;
  week_ms: number;
  month_ms: number;
  year_ms: number;
  streak_days: number;
  longest_streak: number;
  days: Record<string, number>;
  hours: number[];
  top_authors: [string, number][];
  top_books: [number, string, number][];
  formats: [string, number][];
  finished_by_month: Record<string, number>;
  chapters_read: number;
  annotations: number;
  goal: { year: number; books: number; minutes_per_day: number } | null;
  avg_session_ms: number;
};

export type UpdateItem = {
  series_id: number;
  series_title: string;
  cover_url: string;
  cover: string | null;
  chapter: Chapter;
};

export type QueueItem = {
  chapter_id: number;
  series_title: string;
  chapter_title: string;
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  done: number;
  total: number;
  error: string | null;
};

export type ConvertJob = {
  id: number;
  label: string;
  state: "running" | "done" | "failed";
  progress: number;
  message: string;
  output: string | null;
  book_id: number | null;
};

export type Tool = { id: string; name: string; path: string | null; version: string | null; url: string };

export type Device = {
  kind: string;
  name: string;
  mount: string;
  books_dir: string;
  formats: string[];
  free_bytes: number | null;
};

export type DeviceProfile = { id: string; name: string; width: number; height: number; color: boolean };

export type MetaCandidate = {
  source: string;
  title: string;
  authors: string[];
  description: string;
  publisher: string;
  published: string;
  language: string;
  subjects: string[];
  isbn: string;
  series: string;
  pages: number | null;
  cover: string | null;
  url: string;
};

export type ChapterOpen = {
  book_id: number | null;
  pages: string[];
  external_url: string | null;
  start_page: number;
};

const COMIC_FORMATS: BookFormat[] = ["cbz", "cbr", "cb7", "cbt", "folder"];
export const isComic = (b: Pick<Book, "format">) => COMIC_FORMATS.includes(b.format);
