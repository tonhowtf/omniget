import { describe, expect, it } from "vitest";
import {
  camelotCompatible,
  fmtTime,
  groupAlbums,
  lineAt,
  looksUpscaled,
  searchTracks,
  sortTracks,
  splitArtists,
} from "./library";
import type { Track } from "./types";

function track(p: Partial<Track>): Track {
  return {
    id: 1, path: "/m/a.mp3", folder: "/m", title: "", artist: "", album: "", album_artist: "", genre: "",
    composer: "", year: null, track_no: null, disc_no: null, duration_ms: 0, bitrate: null, sample_rate: null,
    bit_depth: null, codec: "MP3", cover: null, has_lyrics: false, rg_track_gain: null, rg_track_peak: null,
    rg_album_gain: null, rg_album_peak: null, loudness_lufs: null, bpm: null, musical_key: "", camelot: "",
    cutoff_hz: null, intro_s: null, outro_s: null, end_s: null, analyzed: false, play_count: 0, skip_count: 0,
    last_played: null, rating: 0, liked: false, added_at: 0, size: 0, source_url: "", ...p,
  };
}

describe("music library helpers", () => {
  it("groups albums by album artist and sorts tracks", () => {
    const tracks = [
      track({ id: 1, album: "A", artist: "X", track_no: 2, title: "b" }),
      track({ id: 2, album: "A", artist: "X", track_no: 1, title: "a", cover: "c1" }),
      track({ id: 3, album: "B", artist: "Y" }),
      track({ id: 4, album: "", artist: "Z" }),
    ];
    const albums = groupAlbums(tracks);
    expect(albums).toHaveLength(2);
    const a = albums.find((x) => x.title === "A")!;
    expect(a.tracks.map((t) => t.id)).toEqual([2, 1]);
    expect(a.cover).toBe("c1");
  });

  it("marks many-artist albums as compilations", () => {
    const tracks = ["p", "q", "r", "s"].map((artist, i) => track({ id: i, album: "Hits", artist }));
    expect(groupAlbums(tracks)[0].artist).toBe("Various Artists");
  });

  it("splits featured artists but keeps AC/DC", () => {
    expect(splitArtists("A feat. B")).toEqual(["A", "B"]);
    expect(splitArtists("AC/DC")).toEqual(["AC/DC"]);
    expect(splitArtists("")).toEqual(["Unknown Artist"]);
  });

  it("searches accent-insensitively across fields", () => {
    const t = [track({ id: 1, title: "Canção", artist: "Zé" }), track({ id: 2, title: "Other" })];
    expect(searchTracks(t, "cancao ze").map((x) => x.id)).toEqual([1]);
    expect(searchTracks(t, "").length).toBe(2);
  });

  it("sorts numerically aware", () => {
    const t = [track({ id: 1, title: "Track 10" }), track({ id: 2, title: "Track 2" })];
    expect(sortTracks(t, "title").map((x) => x.id)).toEqual([2, 1]);
    expect(sortTracks(t, "title", true).map((x) => x.id)).toEqual([1, 2]);
  });

  it("formats time", () => {
    expect(fmtTime(61_000)).toBe("1:01");
    expect(fmtTime(3_661_000)).toBe("1:01:01");
  });

  it("finds the current lyric line", () => {
    const lines = [{ time_ms: 1000, text: "a" }, { time_ms: 5000, text: "b" }];
    expect(lineAt(lines, 0)).toBe(-1);
    expect(lineAt(lines, 1000)).toBe(0);
    expect(lineAt(lines, 7000)).toBe(1);
  });

  it("flags lossless files with a lossy cutoff", () => {
    expect(looksUpscaled({ codec: "FLAC", cutoff_hz: 16000, sample_rate: 44100 })).toBe(true);
    expect(looksUpscaled({ codec: "FLAC", cutoff_hz: 21800, sample_rate: 44100 })).toBe(false);
    expect(looksUpscaled({ codec: "MP3", cutoff_hz: 16000, sample_rate: 44100 })).toBe(false);
  });

  it("knows harmonic neighbours", () => {
    expect(camelotCompatible("8A", "9A")).toBe(true);
    expect(camelotCompatible("12B", "1B")).toBe(true);
    expect(camelotCompatible("8A", "8B")).toBe(true);
    expect(camelotCompatible("8A", "3B")).toBe(false);
  });
});
