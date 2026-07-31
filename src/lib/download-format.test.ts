import { describe, expect, it } from 'vitest';
import { formatBytes, formatEta, formatSpeed } from './download-format';

describe('download-format utilities', () => {
  describe('formatBytes', () => {
    it('formats bytes under 1KB correctly', () => {
      expect(formatBytes(0)).toBe('0 B');
      expect(formatBytes(512)).toBe('512 B');
    });

    it('formats kilobytes correctly', () => {
      expect(formatBytes(1024)).toBe('1.0 KB');
      expect(formatBytes(1536)).toBe('1.5 KB');
    });

    it('formats megabytes correctly', () => {
      expect(formatBytes(1024 * 1024)).toBe('1.0 MB');
      expect(formatBytes(234 * 1024 * 1024)).toBe('234.0 MB');
      expect(formatBytes(512 * 1024 * 1024)).toBe('512.0 MB');
    });

    it('formats gigabytes correctly', () => {
      expect(formatBytes(1024 * 1024 * 1024)).toBe('1.00 GB');
      expect(formatBytes(2.5 * 1024 * 1024 * 1024)).toBe('2.50 GB');
    });
  });

  describe('formatSpeed', () => {
    it('formats 0 or negative speed as 0 KB/s', () => {
      expect(formatSpeed(0)).toBe('0 KB/s');
      expect(formatSpeed(-100)).toBe('0 KB/s');
    });

    it('formats KB/s speed', () => {
      expect(formatSpeed(512 * 1024)).toBe('512 KB/s');
    });

    it('formats MB/s speed', () => {
      expect(formatSpeed(2.5 * 1024 * 1024)).toBe('2.5 MB/s');
    });
  });

  describe('formatEta', () => {
    it('returns empty string for non-positive or non-finite values', () => {
      expect(formatEta(null)).toBe('');
      expect(formatEta(undefined)).toBe('');
      expect(formatEta(0)).toBe('');
      expect(formatEta(-10)).toBe('');
      expect(formatEta(NaN)).toBe('');
      expect(formatEta(Infinity)).toBe('');
    });

    it('formats seconds (< 60s)', () => {
      expect(formatEta(45)).toBe('45s');
    });

    it('formats minutes and seconds (< 1h)', () => {
      expect(formatEta(200)).toBe('3m 20s');
      expect(formatEta(180)).toBe('3m');
    });

    it('formats hours and minutes (>= 1h)', () => {
      expect(formatEta(3600)).toBe('1h');
      expect(formatEta(3660)).toBe('1h 1m');
    });
  });
});
