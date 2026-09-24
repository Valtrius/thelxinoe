import { describe, expect, it } from 'vitest';
import { downloadProgress, downloadState, type DownloadRow } from './downloads';
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
