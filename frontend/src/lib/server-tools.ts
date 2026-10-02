export type ToolVersion = {
  id: string;
  candidate_id?: string;
  version: string;
  channel?: string;
};
export type ServerTool = {
  id: string;
  policy: string;
  effective_policy: string;
  channel: string;
  pinned: boolean;
  revision: number;
  installed: ToolVersion | null;
  previous: ToolVersion | null;
  candidate: { id: string; version: string; notes_url: string | null } | null;
  checked_at: number | null;
  check_error: string | null;
  integrity_error: string | null;
  held?: string | null;
  job: {
    id: string;
    action: string;
    manual: boolean;
    stage: string;
    received: number;
    total: number;
    reason: string | null;
    error: string | null;
  } | null;
};
export type ServerToolStatus = { items: ServerTool[]; supported: boolean };
export function toolBusy(tool: ServerTool) {
  return (
    !!tool.job && !['complete', 'failed', 'canceled'].includes(tool.job.stage)
  );
}
export function toolName(id: string) {
  return (
    {
      'yt-dlp': 'yt-dlp',
      deno: 'Deno',
      streamlink: 'Streamlink',
      ffmpeg: 'FFmpeg + FFprobe',
    }[id] ?? id
  );
}
