export type SupportDownload = {
  id: number;
  title: string;
  status: string;
  remaining_mb: number | null;
  size_mb: number | null;
  downloaded_mb?: number | null;
  history_time?: number | null;
};

export type DownloadRow = SupportDownload & { history: boolean };

export function downloadRows(
  queue: SupportDownload[],
  history: SupportDownload[],
): DownloadRow[] {
  const activeIds = new Set(queue.map((item) => item.id));
  return [
    ...queue.map((item) => ({ ...item, history: false })),
    ...history
      .filter((item) => !activeIds.has(item.id))
      .sort((a, b) => (b.history_time ?? 0) - (a.history_time ?? 0))
      .map((item) => ({ ...item, history: true })),
  ];
}

export function downloadState(row: DownloadRow, globallyPaused = false) {
  const status = row.status || 'UNKNOWN';
  const detail = status
    .toLowerCase()
    .replaceAll('_', ' ')
    .replaceAll('/', ' · ');
  if (status.startsWith('FAILURE/'))
    return { icon: 'error', label: `Failed: ${detail}`, tone: 'bad' };
  if (status.startsWith('WARNING/'))
    return { icon: 'warning', label: detail, tone: 'warn' };
  if (status.startsWith('DELETED/'))
    return { icon: 'removed', label: detail, tone: 'muted' };
  if (row.history && status.startsWith('SUCCESS/'))
    return { icon: 'complete', label: `Completed: ${detail}`, tone: 'ok' };
  if (
    status === 'PAUSED' ||
    (!row.history &&
      globallyPaused &&
      ['DOWNLOADING', 'QUEUED'].includes(status))
  )
    return {
      icon: 'pause',
      label: globallyPaused ? `Downloads paused · ${detail}` : 'Paused',
      tone: 'warn',
    };
  if (status === 'QUEUED')
    return { icon: 'queue', label: 'Queued', tone: 'muted' };
  if (['DOWNLOADING', 'FETCHING'].includes(status))
    return { icon: 'download', label: detail, tone: 'accent' };
  if (
    [
      'PP_QUEUED',
      'LOADING_PARS',
      'VERIFYING_SOURCES',
      'REPAIRING',
      'VERIFYING_REPAIRED',
      'RENAMING',
      'UNPACKING',
      'MOVING',
      'POST_UNPACK_RENAMING',
      'POST_DOWNLOAD_RENAMING',
      'EXECUTING_SCRIPT',
      'PP_FINISHED',
    ].includes(status)
  )
    return {
      icon: 'processing',
      label: `Processing: ${detail}`,
      tone: 'accent',
    };
  return { icon: 'unknown', label: detail, tone: 'muted' };
}

const size = (value: number | null | undefined) =>
  typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? value
    : null;

export function downloadProgress(row: DownloadRow) {
  const total = size(row.size_mb);
  const success = row.history && row.status.startsWith('SUCCESS/');
  const reported = size(row.downloaded_mb);
  const remaining = success ? 0 : size(row.remaining_mb);
  const downloaded =
    reported ??
    (total !== null && remaining !== null
      ? Math.max(0, total - remaining)
      : null);
  const fraction =
    total !== null && total > 0
      ? success
        ? 1
        : remaining !== null
          ? (total - remaining) / total
          : downloaded !== null
            ? downloaded / total
            : null
      : null;
  return {
    downloaded,
    remaining,
    percent:
      fraction === null
        ? null
        : Math.round(Math.max(0, Math.min(1, fraction)) * 100),
  };
}

export function formatDownloadSize(mib: number | null) {
  if (mib === null) return '—';
  if (mib < 1024) return `${Math.round(mib)} MiB`;
  if (mib < 1024 ** 2) return `${Number((mib / 1024).toFixed(2))} GiB`;
  return `${Number((mib / 1024 ** 2).toFixed(2))} TiB`;
}
