import { describe, expect, it } from 'vitest';
import {
  downloadProgress,
  downloadRows,
  downloadState,
  formatDownloadSize,
  type DownloadRow,
} from './downloads';
import { serviceUiUrl } from './presentation';

const row: DownloadRow = {
  id: 1,
  title: 'Example',
  status: 'DOWNLOADING',
  size_mb: 8192,
  remaining_mb: 2048,
  history: false,
};

describe('NZBGet presentation', () => {
  it('keeps active rows first and uses newest history without duplicating queue entries', () => {
    const history = [
      { ...row, id: 2, history_time: 10 },
      { ...row, id: 3, history_time: 20 },
      { ...row, history_time: 30 },
    ];
    expect(downloadRows([row], history).map((item) => item.id)).toEqual([
      1, 3, 2,
    ]);
    expect(history.map((item) => item.id)).toEqual([2, 3, 1]);
  });
  it('shows measured transfer progress and preserves post-processing status', () => {
    expect(downloadProgress(row)).toEqual({
      downloaded: 6144,
      remaining: 2048,
      percent: 75,
    });
    expect(downloadState({ ...row, status: 'UNPACKING' })).toMatchObject({
      icon: 'processing',
      label: 'Processing: unpacking',
    });
    expect(downloadState(row, true)).toMatchObject({ icon: 'pause' });
    expect(formatDownloadSize(6144)).toBe('6 GiB');
  });
  it('does not turn failed or incomplete history into successful downloads', () => {
    const failed = {
      ...row,
      history: true,
      status: 'FAILURE/UNPACK',
      remaining_mb: null,
      downloaded_mb: 8192,
    };
    expect(downloadState(failed)).toMatchObject({ icon: 'error', tone: 'bad' });
    expect(downloadProgress(failed)).toEqual({
      downloaded: 8192,
      remaining: null,
      percent: 100,
    });
    expect(
      downloadProgress({
        ...failed,
        status: 'SUCCESS/HIDDEN',
        size_mb: null,
        downloaded_mb: null,
      }),
    ).toEqual({ downloaded: null, remaining: 0, percent: null });
    expect(
      downloadProgress({ ...row, size_mb: 0, remaining_mb: 0 }).percent,
    ).toBeNull();
    expect(downloadProgress({ ...row, remaining_mb: NaN }).percent).toBeNull();
  });
});

describe('service UI links', () => {
  const ports = [
    { Type: 'tcp', PrivatePort: 7878, PublicPort: 17878, IP: '127.0.0.1' },
  ];
  it('uses an explicit external address or a reachable published port', () => {
    expect(
      serviceUiUrl(
        'https://media.example/radarr',
        ports,
        7878,
        'https://server.example',
      ),
    ).toBe('https://media.example/radarr');
    expect(serviceUiUrl(undefined, ports, 7878, 'http://localhost:18488')).toBe(
      'http://localhost:17878/',
    );
    expect(
      serviceUiUrl(
        undefined,
        [{ ...ports[0], IP: '0.0.0.0' }],
        7878,
        'https://server.example',
      ),
    ).toBe('http://server.example:17878/');
  });
  it('does not invent a link for remote loopback or expose credentials', () => {
    expect(serviceUiUrl(undefined, ports, 7878, 'https://server.example')).toBe(
      '',
    );
    expect(
      serviceUiUrl(
        'https://user:secret@server.example',
        ports,
        7878,
        'http://localhost',
      ),
    ).toBe('');
    expect(
      serviceUiUrl('javascript:alert(1)', ports, 7878, 'http://localhost'),
    ).toBe('');
    expect(serviceUiUrl(undefined, [], 7878, 'https://server.example')).toBe(
      '',
    );
  });
});
