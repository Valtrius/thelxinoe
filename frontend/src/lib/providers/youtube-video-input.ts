const youtubeVideoIdPattern = /^[A-Za-z0-9_-]{11}$/;
const youtubeHosts = new Set([
  'youtube.com',
  'www.youtube.com',
  'm.youtube.com',
  'music.youtube.com',
]);
const youtubeShortHosts = new Set(['youtu.be', 'www.youtu.be']);
const schemelessYoutubeUrl =
  /^(?:(?:www|m|music)\.)?youtube\.com(?:\/|$)|^(?:www\.)?youtu\.be(?:\/|$)/i;

export function youtubeVideoIdFromInput(raw: string): string | null {
  const value = raw.trim();
  if (youtubeVideoIdPattern.test(value)) return value;

  const candidate = schemelessYoutubeUrl.test(value)
    ? `https://${value}`
    : value;
  let parsed: URL;
  try {
    parsed = new URL(candidate);
  } catch {
    return null;
  }

  if (
    !['http:', 'https:'].includes(parsed.protocol) ||
    parsed.username !== '' ||
    parsed.password !== ''
  ) {
    return null;
  }

  const hostname = parsed.hostname.toLowerCase();
  const segments = parsed.pathname.split('/').filter(Boolean);
  let videoId: string | null = null;

  if (youtubeShortHosts.has(hostname) && segments.length === 1) {
    videoId = segments[0];
  } else if (youtubeHosts.has(hostname)) {
    if (segments.length === 1 && segments[0] === 'watch') {
      videoId = parsed.searchParams.get('v');
    } else if (
      segments.length === 2 &&
      ['shorts', 'live'].includes(segments[0])
    ) {
      videoId = segments[1];
    }
  }

  return videoId && youtubeVideoIdPattern.test(videoId) ? videoId : null;
}
